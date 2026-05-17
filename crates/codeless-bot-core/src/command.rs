//! Parser for Surface 1 of the Slack control plane. Translates a raw
//! Slack message body (with an optional notification-thread context)
//! into a typed [`Command`] enum the dispatcher in stage 4 can hand
//! straight to `RpcServer`.
//!
//! The grammar is intentionally narrow — five verbs, one positional
//! keyword, one optional quoted comment — so the parser fits in a
//! single hand-rolled state machine without pulling in `nom` or
//! similar. Anti-patterns called out in
//! `.codeless/jobs/slack-integration/SCOPE.md` (Block Kit modals,
//! per-channel "last job id" memory, reactions as decisions) stay
//! out of scope by construction: the [`Command`] enum has no variant
//! that could carry them.
//!
//! # Thread vs. cold context
//!
//! When the bot posts an outbound failure notification it remembers
//! the notification's job id keyed off the message's Slack thread.
//! Replies inside that thread arrive here with a populated
//! [`ThreadContext`]: the parser then accepts the short forms
//! (`stop`, `resume`, `resume bypass`, `resume "<comment>"`) and
//! substitutes the thread's job id. Outside a notification thread
//! every action verb requires an explicit job id.
//!
//! The single exception is `status`: with no thread the bare verb
//! lists every job; inside a thread it returns one-job detail for
//! the thread's job. That mirrors the operator's likely intent in
//! each context — outside, "what is going on across the fleet";
//! inside, "what is the state of *this* failure right now".
//!
//! # Stripping the Slack bot mention
//!
//! Slack rewrites `@codeless` as `<@U12345>` (or `<@U12345|alias>`)
//! at the head of the message. The parser strips one leading mention
//! token automatically so the dispatcher can pass the raw text in;
//! it does not check that the mention is actually *the bot*, since
//! Slack already routes the message to us by mention or thread
//! subscription. A spurious mention to a different user that
//! happens to arrive here just gets stripped and the rest parses
//! as normal.

use std::str::FromStr;

use codeless_rpc::methods::ChatMode;
use codeless_types::JobId;
use thiserror::Error;

/// Context carried alongside an inbound Slack message. The dispatcher
/// fills `job_id` when the message is a reply in a thread previously
/// posted as an outbound failure notification; otherwise the field is
/// `None` and the parser refuses any verb that would need a job id
/// it cannot supply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThreadContext {
    /// Job id bound to the notification thread this message replied
    /// to. `None` means the message is "cold" — a DM, a top-level
    /// channel message, or a reply in a thread the bot does not own.
    pub job_id: Option<JobId>,
}

impl ThreadContext {
    /// Convenience for the cold case used pervasively in tests and
    /// in the dispatcher's "no thread mapping found" branch.
    pub const COLD: Self = Self { job_id: None };

    /// Convenience for thread replies bound to a specific job id.
    pub fn for_job(job_id: JobId) -> Self {
        Self {
            job_id: Some(job_id),
        }
    }
}

/// One parsed Slack command. Each variant maps to one `RpcServer`
/// method; the dispatcher in stage 4 is a straight match-and-call,
/// so keep the variants aligned with the method names in
/// `codeless-rpc` rather than the Slack verb spellings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Bare `status` outside a thread — list every job.
    ListJobs,
    /// `status <id>` or bare `status` inside a thread — detail for
    /// one job. The dispatcher renders this as the same one-line
    /// status block the outbound notification uses.
    GetJob { job_id: JobId },
    /// `start <id>` (or bare `start` inside a thread) — promote a
    /// `Draft` job to `Queued`.
    StartJob { job_id: JobId },
    /// `stop` / `stop <id>` — transition `Running` / `Queued` →
    /// `Stopped`.
    StopJob { job_id: JobId },
    /// `resume` / `resume <id>` plus optional `bypass` keyword and
    /// optional trailing quoted comment. Maps directly onto
    /// `ResumeJobArgs.bypass` and `ResumeJobArgs.next_stage_comment`
    /// — the parser does not normalise an empty comment, since the
    /// runtime already collapses `Some("")` to `None` on entry.
    ResumeJob {
        job_id: JobId,
        bypass: bool,
        comment: Option<String>,
    },
    /// `chat <id> <message…>` (or bare `chat <message…>` inside a
    /// thread) — one-shot agent_chat turn against the job. `mode`
    /// distinguishes `Work` (verb `chat`) from `Spec` (verb `spec`).
    /// The message is the rest of the line verbatim — no quoting,
    /// since chat is the one verb whose payload is free-form prose.
    Chat {
        job_id: JobId,
        mode: ChatMode,
        message: String,
    },
    /// `help` (or any bare mention with no other tokens). The
    /// dispatcher renders the canned help block; emitting a variant
    /// rather than returning `None` keeps unknown input
    /// distinguishable from "the user asked for help".
    Help,
}

/// Reasons the parser refused a message. Each variant carries
/// enough text to render a helpful one-line reply pointing the
/// operator at the right grammar — Surface 1 explicitly avoids
/// multi-step interactive flows, so a single rejection message
/// has to be self-contained.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    /// The message is empty (only whitespace, or only the leading
    /// mention with nothing after). The dispatcher silently ignores
    /// this — Slack frequently delivers `<@U...>` with no payload
    /// when the user typed only the mention and hit send.
    #[error("empty command")]
    Empty,

    /// First token is not one of the known verbs. The dispatcher
    /// renders the help text in reply.
    #[error("unknown command `{verb}`; try `help`")]
    UnknownVerb { verb: String },

    /// A verb that needs a job id was issued cold (no thread
    /// context) without one. The message names the verb so the
    /// reply can be specific.
    #[error("`{verb}` needs a job id when used outside a notification thread")]
    MissingJobId { verb: &'static str },

    /// A token that should have been a ULID job id did not parse.
    /// Carries the offending text so the reply can echo it back —
    /// the most common cause is a copy/paste that picked up
    /// surrounding markdown.
    #[error("`{token}` is not a valid job id (expected a 26-character ULID)")]
    InvalidJobId { token: String },

    /// Trailing tokens after the grammar was satisfied. Surface 1
    /// rejects rather than silently dropping them so a typo
    /// (`resume <id> bypas`) is visible.
    #[error("unexpected trailing input: `{rest}`")]
    Trailing { rest: String },

    /// The optional quoted comment opened with `"` but never closed.
    /// Slack does not normalise smart quotes either way; the
    /// operator who pasted from a styled keyboard sees this error
    /// and knows to fix the quoting.
    #[error("comment is missing its closing `\"`")]
    UnclosedComment,

    /// `chat` / `spec` was issued without any message body.
    /// Surfacing this separately from `Empty` lets the reply point
    /// the operator at the right grammar (`chat <N> <message>`).
    #[error("`{verb}` needs a message body")]
    EmptyChatMessage { verb: &'static str },
}

/// Entry point. Consumes one Slack message body plus the
/// dispatcher-supplied thread context and returns either a typed
/// `Command` or a `ParseError` the dispatcher can render verbatim.
pub fn parse(input: &str, ctx: ThreadContext) -> Result<Command, ParseError> {
    let body = strip_leading_mention(input.trim());
    if body.is_empty() {
        return Err(ParseError::Empty);
    }

    let (verb_raw, rest) = split_first_token(body);
    let verb = verb_raw.to_ascii_lowercase();
    let rest = rest.trim_start();

    match verb.as_str() {
        "help" | "?" => {
            // `help` accepts no further tokens; flag a typo rather
            // than silently dropping arguments so `help me please`
            // becomes a clear rejection.
            require_no_trailing(rest)?;
            Ok(Command::Help)
        }
        "status" => parse_status(rest, ctx),
        "start" => parse_action_verb(rest, ctx, "start", false, |job_id, bypass, comment| {
            // `start` does not take `bypass` or comment; the parser
            // forbids them by passing `false` and asserting comment
            // is None below.
            debug_assert!(!bypass);
            debug_assert!(comment.is_none());
            Command::StartJob { job_id }
        }),
        "stop" => parse_action_verb(rest, ctx, "stop", false, |job_id, bypass, comment| {
            debug_assert!(!bypass);
            debug_assert!(comment.is_none());
            Command::StopJob { job_id }
        }),
        "resume" => parse_action_verb(rest, ctx, "resume", true, |job_id, bypass, comment| {
            Command::ResumeJob {
                job_id,
                bypass,
                comment,
            }
        }),
        "chat" => parse_chat(rest, ctx, "chat", ChatMode::Work),
        "spec" => parse_chat(rest, ctx, "spec", ChatMode::Spec),
        _ => Err(ParseError::UnknownVerb {
            verb: verb_raw.to_string(),
        }),
    }
}

/// `status` is the one verb whose meaning changes by context: cold
/// without an id lists every job, in-thread without an id returns
/// the thread's job, and an explicit id always overrides the
/// context. Carved out from the action-verb parser because that
/// shape (job-id-or-error) does not fit `status`'s "fall through to
/// list_jobs" branch.
fn parse_status(rest: &str, ctx: ThreadContext) -> Result<Command, ParseError> {
    if rest.is_empty() {
        return match ctx.job_id {
            Some(job_id) => Ok(Command::GetJob { job_id }),
            None => Ok(Command::ListJobs),
        };
    }
    let (id_tok, tail) = split_first_token(rest);
    require_no_trailing(tail.trim_start())?;
    let job_id = parse_job_id(id_tok)?;
    Ok(Command::GetJob { job_id })
}

/// Shared engine for `start` / `stop` / `resume`. `allow_resume_tail`
/// controls whether the optional `bypass` keyword and trailing quoted
/// comment are accepted; only `resume` sets it. `build` packages the
/// resolved arguments into the variant the verb wanted.
fn parse_action_verb(
    rest: &str,
    ctx: ThreadContext,
    verb: &'static str,
    allow_resume_tail: bool,
    build: impl FnOnce(JobId, bool, Option<String>) -> Command,
) -> Result<Command, ParseError> {
    let mut cursor = rest;
    let job_id = resolve_job_id(&mut cursor, ctx, verb)?;
    let cursor = cursor.trim_start();

    if !allow_resume_tail {
        require_no_trailing(cursor)?;
        return Ok(build(job_id, false, None));
    }

    let (bypass, after_bypass) = parse_optional_bypass(cursor);
    let comment = parse_optional_comment(after_bypass)?;
    Ok(build(job_id, bypass, comment))
}

/// `chat` / `spec` have a different shape from `start`/`stop`/
/// `resume`: the payload is free-form prose, not a quoted comment,
/// so the parser does not require quoting or reject trailing
/// tokens. The id, if present, is the first whitespace-delimited
/// token; everything after it (verbatim, with internal whitespace
/// preserved) is the message. If the first token does not parse
/// as a ULID the parser falls back to the thread context rather
/// than erroring — `chat what is up` in a thread treats `what is
/// up` as the entire message, which is the shape that makes the
/// in-thread shortcut feel natural.
fn parse_chat(
    rest: &str,
    ctx: ThreadContext,
    verb: &'static str,
    mode: ChatMode,
) -> Result<Command, ParseError> {
    if rest.is_empty() {
        return Err(ParseError::EmptyChatMessage { verb });
    }

    let (first, after) = split_first_token(rest);
    let (job_id, message) = match JobId::from_str(first) {
        Ok(id) => (id, after.trim_start()),
        Err(_) => {
            let id = ctx.job_id.ok_or(ParseError::MissingJobId { verb })?;
            (id, rest)
        }
    };

    let message = message.trim();
    if message.is_empty() {
        return Err(ParseError::EmptyChatMessage { verb });
    }

    Ok(Command::Chat {
        job_id,
        mode,
        message: message.to_string(),
    })
}

/// Consumes the first token off `cursor` when it parses as a ULID;
/// otherwise falls back to the thread context. The thread context
/// case advances `cursor` only when it would otherwise be empty, so
/// `resume bypass` *in thread* parses `bypass` as the bypass
/// keyword (not as a job id), while `resume 01ABCDEFGH...` *cold*
/// consumes the ULID.
///
/// Returns the resolved id or a precise error: a token that looks
/// like an id but does not parse trips `InvalidJobId`, while a
/// missing id with no thread context trips `MissingJobId`.
fn resolve_job_id(
    cursor: &mut &str,
    ctx: ThreadContext,
    verb: &'static str,
) -> Result<JobId, ParseError> {
    if cursor.is_empty() {
        return ctx.job_id.ok_or(ParseError::MissingJobId { verb });
    }

    let (first, after) = split_first_token(cursor);

    // Look-ahead: if the first token is the literal `bypass` keyword
    // *and* a thread context is available, treat the keyword as
    // belonging to the resume grammar rather than as a malformed job
    // id. Without this, an in-thread `resume bypass` would trip
    // `InvalidJobId { token: "bypass" }`, which is the wrong error
    // for the wrong reason.
    if first.eq_ignore_ascii_case("bypass") {
        if let Some(job_id) = ctx.job_id {
            // Leave `cursor` untouched so the optional-bypass parser
            // sees the keyword and consumes it.
            return Ok(job_id);
        }
        // Cold: `resume bypass` with no thread is just a missing id.
        // Surfacing it as MissingJobId rather than InvalidJobId
        // matches the operator's intent — they forgot the id, not
        // the keyword.
        return Err(ParseError::MissingJobId { verb });
    }

    // A leading quoted comment with no preceding id is the same
    // "forgot the id" case; otherwise the comment would be
    // misparsed as a malformed ULID. Only meaningful for `resume`
    // since `start`/`stop` reject the trailing comment further down.
    if first.starts_with('"') {
        if let Some(job_id) = ctx.job_id {
            return Ok(job_id);
        }
        return Err(ParseError::MissingJobId { verb });
    }

    let job_id = parse_job_id(first)?;
    *cursor = after;
    Ok(job_id)
}

/// Detects the `bypass` keyword in the slot immediately after the
/// job id and consumes it. Case-insensitive to match operator habit
/// on phones; any token other than `bypass` is left for the comment
/// parser (which will reject anything that is not a quoted string).
fn parse_optional_bypass(rest: &str) -> (bool, &str) {
    if rest.is_empty() {
        return (false, rest);
    }
    let (token, after) = split_first_token(rest);
    if token.eq_ignore_ascii_case("bypass") {
        (true, after.trim_start())
    } else {
        (false, rest)
    }
}

/// Consumes the optional trailing comment. The grammar guarantees
/// the comment is always the *last* token, always wrapped in `"…"`,
/// and uses backslash escapes (`\"`, `\\`) inside. Any tokens after
/// the closing quote trip `ParseError::Trailing` so a typo right of
/// the quote is visible.
fn parse_optional_comment(rest: &str) -> Result<Option<String>, ParseError> {
    if rest.is_empty() {
        return Ok(None);
    }
    if !rest.starts_with('"') {
        // Not a quoted comment, not whitespace: trailing garbage.
        return Err(ParseError::Trailing {
            rest: rest.to_string(),
        });
    }
    let (text, after_quote) = consume_quoted(rest)?;
    require_no_trailing(after_quote.trim_start())?;
    Ok(Some(text))
}

/// Reads a `"…"` token from `input` (whose first byte must be `"`)
/// returning the unescaped content and the slice after the closing
/// quote. Recognises `\"` and `\\` only — every other escape is
/// passed through verbatim so a Slack-flavoured `\n` keeps its
/// literal backslash, matching what the operator typed.
fn consume_quoted(input: &str) -> Result<(String, &str), ParseError> {
    debug_assert!(input.starts_with('"'));
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().saturating_sub(2));
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() && (bytes[i + 1] == b'"' || bytes[i + 1] == b'\\') => {
                out.push(bytes[i + 1] as char);
                i += 2;
            }
            b'"' => {
                // SAFETY (no unsafe): UTF-8 boundary, since we only
                // advanced over ASCII bytes (`\`, `"`) and otherwise
                // copied bytes one-for-one into `out` from a UTF-8
                // input by character index below. We re-slice the
                // original `input` from `i + 1`, which is right
                // after the ASCII closing quote — always a valid
                // boundary.
                return Ok((out, &input[i + 1..]));
            }
            _ => {
                // Copy the next *character* (not byte) so multibyte
                // sequences survive the parse. Find the char by
                // re-decoding from the current byte position.
                let ch = input[i..]
                    .chars()
                    .next()
                    .expect("non-empty slice yields a char");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    Err(ParseError::UnclosedComment)
}

fn parse_job_id(token: &str) -> Result<JobId, ParseError> {
    JobId::from_str(token).map_err(|_| ParseError::InvalidJobId {
        token: token.to_string(),
    })
}

/// Reject any non-empty tail. Used after every terminal arm so
/// `status x y` and `stop <id> garbage` produce a `Trailing` error
/// rather than a silent partial-success.
fn require_no_trailing(rest: &str) -> Result<(), ParseError> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(ParseError::Trailing {
            rest: rest.to_string(),
        })
    }
}

/// Strip one leading `<@USERID>` or `<@USERID|name>` mention plus
/// any whitespace after it. Slack wraps `@bot` mentions in that
/// envelope before delivering the message; the parser is mention-
/// agnostic because Slack already routes the message to us based on
/// the mention or the thread subscription. Returns the input
/// unchanged when no mention is present.
fn strip_leading_mention(input: &str) -> &str {
    if let Some(rest) = input.strip_prefix("<@") {
        if let Some(end) = rest.find('>') {
            // Everything between `<@` and `>` is the user id and
            // optional `|name` alias; we do not parse it.
            return rest[end + 1..].trim_start();
        }
    }
    input
}

/// Splits at the first whitespace run, returning `(first_token, rest)`.
/// The returned `rest` keeps any leading whitespace so callers can
/// decide whether to `trim_start` (most do).
fn split_first_token(input: &str) -> (&str, &str) {
    match input.find(char::is_whitespace) {
        Some(i) => (&input[..i], &input[i..]),
        None => (input, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_id() -> JobId {
        JobId::new()
    }

    fn must_parse(input: &str, ctx: ThreadContext) -> Command {
        parse(input, ctx).unwrap_or_else(|e| panic!("expected ok for {input:?}, got {e:?}"))
    }

    #[test]
    fn empty_input_is_an_error_dispatcher_can_swallow() {
        assert_eq!(
            parse("", ThreadContext::COLD).unwrap_err(),
            ParseError::Empty
        );
        assert_eq!(
            parse("   ", ThreadContext::COLD).unwrap_err(),
            ParseError::Empty
        );
        assert_eq!(
            parse("<@U12345>", ThreadContext::COLD).unwrap_err(),
            ParseError::Empty
        );
        assert_eq!(
            parse("<@U12345|alias>  ", ThreadContext::COLD).unwrap_err(),
            ParseError::Empty
        );
    }

    #[test]
    fn leading_mention_is_optional_and_invisible() {
        // With and without the leading mention the parser produces
        // the same command — the dispatcher hands us the raw text
        // either way.
        let id = fresh_id();
        let raw = format!("stop {id}");
        let with_mention = format!("<@UBOT123> {raw}");
        let with_alias = format!("<@UBOT123|codeless> {raw}");
        let direct = must_parse(&raw, ThreadContext::COLD);
        let via_mention = must_parse(&with_mention, ThreadContext::COLD);
        let via_alias = must_parse(&with_alias, ThreadContext::COLD);
        assert_eq!(direct, Command::StopJob { job_id: id });
        assert_eq!(via_mention, direct);
        assert_eq!(via_alias, direct);
    }

    #[test]
    fn verb_matching_is_case_insensitive_id_is_not() {
        let id = fresh_id();
        assert_eq!(
            must_parse(&format!("STOP {id}"), ThreadContext::COLD),
            Command::StopJob { job_id: id }
        );
        assert_eq!(
            must_parse(&format!("Resume {id}"), ThreadContext::COLD),
            Command::ResumeJob {
                job_id: id,
                bypass: false,
                comment: None,
            }
        );
    }

    #[test]
    fn help_aliases_and_rejects_trailing() {
        assert_eq!(must_parse("help", ThreadContext::COLD), Command::Help);
        assert_eq!(must_parse("?", ThreadContext::COLD), Command::Help);
        let err = parse("help me", ThreadContext::COLD).unwrap_err();
        assert!(matches!(err, ParseError::Trailing { .. }));
    }

    #[test]
    fn status_cold_lists_in_thread_returns_get_explicit_id_always_wins() {
        let thread_id = fresh_id();
        let explicit_id = fresh_id();
        // cold + bare → list
        assert_eq!(must_parse("status", ThreadContext::COLD), Command::ListJobs);
        // in thread + bare → get_job for the thread's id
        assert_eq!(
            must_parse("status", ThreadContext::for_job(thread_id)),
            Command::GetJob { job_id: thread_id }
        );
        // explicit id overrides the thread context — surface what
        // the operator actually typed.
        assert_eq!(
            must_parse(
                &format!("status {explicit_id}"),
                ThreadContext::for_job(thread_id)
            ),
            Command::GetJob {
                job_id: explicit_id
            }
        );
    }

    #[test]
    fn start_requires_explicit_id_cold_thread_supplies_it() {
        let id = fresh_id();
        assert_eq!(
            must_parse(&format!("start {id}"), ThreadContext::COLD),
            Command::StartJob { job_id: id }
        );
        assert_eq!(
            must_parse("start", ThreadContext::for_job(id)),
            Command::StartJob { job_id: id }
        );
        assert_eq!(
            parse("start", ThreadContext::COLD).unwrap_err(),
            ParseError::MissingJobId { verb: "start" }
        );
    }

    #[test]
    fn stop_short_form_works_in_thread_long_form_works_cold() {
        let id = fresh_id();
        assert_eq!(
            must_parse("stop", ThreadContext::for_job(id)),
            Command::StopJob { job_id: id }
        );
        assert_eq!(
            must_parse(&format!("stop {id}"), ThreadContext::COLD),
            Command::StopJob { job_id: id }
        );
        assert_eq!(
            parse("stop", ThreadContext::COLD).unwrap_err(),
            ParseError::MissingJobId { verb: "stop" }
        );
    }

    #[test]
    fn stop_rejects_bypass_and_comment() {
        let id = fresh_id();
        let err = parse(&format!("stop {id} bypass"), ThreadContext::COLD).unwrap_err();
        assert!(matches!(err, ParseError::Trailing { .. }));
        let err = parse(&format!("stop {id} \"why\""), ThreadContext::COLD).unwrap_err();
        assert!(matches!(err, ParseError::Trailing { .. }));
    }

    #[test]
    fn resume_basic_forms() {
        let id = fresh_id();
        assert_eq!(
            must_parse(&format!("resume {id}"), ThreadContext::COLD),
            Command::ResumeJob {
                job_id: id,
                bypass: false,
                comment: None,
            }
        );
        assert_eq!(
            must_parse("resume", ThreadContext::for_job(id)),
            Command::ResumeJob {
                job_id: id,
                bypass: false,
                comment: None,
            }
        );
        assert_eq!(
            parse("resume", ThreadContext::COLD).unwrap_err(),
            ParseError::MissingJobId { verb: "resume" }
        );
    }

    #[test]
    fn resume_bypass_keyword_in_thread_and_cold() {
        let id = fresh_id();
        assert_eq!(
            must_parse("resume bypass", ThreadContext::for_job(id)),
            Command::ResumeJob {
                job_id: id,
                bypass: true,
                comment: None,
            }
        );
        assert_eq!(
            must_parse(&format!("resume {id} bypass"), ThreadContext::COLD),
            Command::ResumeJob {
                job_id: id,
                bypass: true,
                comment: None,
            }
        );
        // Cold `resume bypass` with no id surfaces as a missing-id
        // error rather than `InvalidJobId { token: "bypass" }`,
        // since the operator's mistake is forgetting the id.
        assert_eq!(
            parse("resume bypass", ThreadContext::COLD).unwrap_err(),
            ParseError::MissingJobId { verb: "resume" }
        );
    }

    #[test]
    fn resume_comment_keyword_slot_is_load_bearing() {
        // Per SCOPE.md: `bypass` is the literal keyword in the
        // keyword slot. A comment whose text contains the word
        // "bypass" must NOT be interpreted as bypass=true.
        let id = fresh_id();
        let with_keyword = must_parse(
            &format!("resume {id} bypass \"this also bypasses linting\""),
            ThreadContext::COLD,
        );
        assert_eq!(
            with_keyword,
            Command::ResumeJob {
                job_id: id,
                bypass: true,
                comment: Some("this also bypasses linting".into()),
            }
        );

        let comment_only = must_parse(
            &format!("resume {id} \"please bypass the linter manually\""),
            ThreadContext::COLD,
        );
        assert_eq!(
            comment_only,
            Command::ResumeJob {
                job_id: id,
                bypass: false,
                comment: Some("please bypass the linter manually".into()),
            }
        );
    }

    #[test]
    fn resume_comment_in_thread_no_id_needed() {
        let id = fresh_id();
        let cmd = must_parse(
            "resume \"redo this stage; do not list the design doc\"",
            ThreadContext::for_job(id),
        );
        assert_eq!(
            cmd,
            Command::ResumeJob {
                job_id: id,
                bypass: false,
                comment: Some("redo this stage; do not list the design doc".into()),
            }
        );
    }

    #[test]
    fn resume_comment_escape_handling() {
        let id = fresh_id();
        let cmd = must_parse(
            // Embedded \" becomes a literal " in the comment; the
            // surrounding backslash itself is consumed.
            &format!(r#"resume {id} "he said \"go\" then \\stop\\""#),
            ThreadContext::COLD,
        );
        assert_eq!(
            cmd,
            Command::ResumeJob {
                job_id: id,
                bypass: false,
                comment: Some(r#"he said "go" then \stop\"#.into()),
            }
        );
    }

    #[test]
    fn resume_unclosed_comment_is_an_explicit_error() {
        let id = fresh_id();
        let err = parse(&format!(r#"resume {id} "no closing"#), ThreadContext::COLD).unwrap_err();
        assert_eq!(err, ParseError::UnclosedComment);
    }

    #[test]
    fn resume_trailing_after_comment_is_rejected() {
        let id = fresh_id();
        let err = parse(
            &format!(r#"resume {id} bypass "comment" extra-junk"#),
            ThreadContext::COLD,
        )
        .unwrap_err();
        assert!(matches!(err, ParseError::Trailing { .. }));
    }

    #[test]
    fn unknown_verb_carries_the_offending_token() {
        let err = parse("dance now", ThreadContext::COLD).unwrap_err();
        match err {
            ParseError::UnknownVerb { verb } => assert_eq!(verb, "dance"),
            other => panic!("expected UnknownVerb, got {other:?}"),
        }
    }

    #[test]
    fn malformed_job_id_carries_the_offending_token() {
        let err = parse("stop not-a-ulid", ThreadContext::COLD).unwrap_err();
        match err {
            ParseError::InvalidJobId { token } => assert_eq!(token, "not-a-ulid"),
            other => panic!("expected InvalidJobId, got {other:?}"),
        }
    }

    #[test]
    fn unicode_in_comment_survives_round_trip() {
        let id = fresh_id();
        let cmd = must_parse(
            &format!("resume {id} \"café — ☕ keep going\""),
            ThreadContext::COLD,
        );
        assert_eq!(
            cmd,
            Command::ResumeJob {
                job_id: id,
                bypass: false,
                comment: Some("café — ☕ keep going".into()),
            }
        );
    }

    #[test]
    fn resume_cold_bare_quoted_comment_is_missing_id_not_invalid_id() {
        // A user who forgot the id and started typing the comment
        // straight away. The clearer error is "missing id".
        let err = parse("resume \"please be careful\"", ThreadContext::COLD).unwrap_err();
        assert_eq!(err, ParseError::MissingJobId { verb: "resume" });
    }

    #[test]
    fn extra_whitespace_between_tokens_is_tolerated() {
        let id = fresh_id();
        let cmd = must_parse(
            &format!("  resume    {id}    bypass   \"comment\"  "),
            ThreadContext::COLD,
        );
        assert_eq!(
            cmd,
            Command::ResumeJob {
                job_id: id,
                bypass: true,
                comment: Some("comment".into()),
            }
        );
    }

    #[test]
    fn chat_with_explicit_id_takes_message_tail() {
        let id = fresh_id();
        let cmd = must_parse(&format!("chat {id} how many rows"), ThreadContext::COLD);
        assert_eq!(
            cmd,
            Command::Chat {
                job_id: id,
                mode: ChatMode::Work,
                message: "how many rows".into(),
            }
        );
    }

    #[test]
    fn spec_picks_up_spec_mode() {
        let id = fresh_id();
        let cmd = must_parse(&format!("spec {id} tighten the SCOPE"), ThreadContext::COLD);
        assert_eq!(
            cmd,
            Command::Chat {
                job_id: id,
                mode: ChatMode::Spec,
                message: "tighten the SCOPE".into(),
            }
        );
    }

    #[test]
    fn chat_in_thread_treats_whole_input_as_message() {
        let id = fresh_id();
        let cmd = must_parse(
            "chat what does the diff look like",
            ThreadContext::for_job(id),
        );
        assert_eq!(
            cmd,
            Command::Chat {
                job_id: id,
                mode: ChatMode::Work,
                message: "what does the diff look like".into(),
            }
        );
    }

    #[test]
    fn chat_without_message_errors() {
        let id = fresh_id();
        assert_eq!(
            parse(&format!("chat {id}"), ThreadContext::COLD).unwrap_err(),
            ParseError::EmptyChatMessage { verb: "chat" }
        );
        assert_eq!(
            parse("chat", ThreadContext::for_job(fresh_id())).unwrap_err(),
            ParseError::EmptyChatMessage { verb: "chat" }
        );
    }

    #[test]
    fn chat_cold_with_no_id_errors() {
        assert_eq!(
            parse("chat hello", ThreadContext::COLD).unwrap_err(),
            ParseError::MissingJobId { verb: "chat" }
        );
    }
}

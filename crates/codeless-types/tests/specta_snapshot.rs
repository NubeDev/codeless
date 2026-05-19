//! Specta TypeScript codegen snapshot. The generated `.ts` is the wire
//! contract every UI client compiles against (SCOPE.md "Rule 1 — One
//! transport interface, many implementations"). A diff against the
//! checked-in `wire.ts.snap` is a wire-format change; if intentional,
//! re-run with `SPECTA_UPDATE=1 cargo test -p codeless-types
//! --test specta_snapshot` to regenerate the snapshot, otherwise the
//! test fails so the agent making the change has to look at it.

use std::path::PathBuf;

use codeless_types::{
    AdapterError, AssistantAction, AssistantActionCard, AssistantActionStatus, AssistantAttachment,
    AssistantAttachmentCard, AssistantAttachmentCardItem, AssistantAttachmentId, AssistantMessage,
    AssistantMessageId, AssistantMessageRole, AssistantThread, AssistantThreadId,
    AttachWorkspaceArgs, AttachWorkspaceResult, AttachedWorkspace, AttachmentRef, ChatAdapterKind,
    ChatAdapterRow, ChatAdapterSecretProblem, ChatBinding, ChatMessage, ChatRole, ChatTransport,
    CostCents, DetachPolicy, DetachWorkspaceArgs, Event, EventCursor, EventEnvelope, FsEntry,
    FsEntryKind, GitAuth, Job, JobId, JobStatus, ListChatAdaptersResult, ListRunnersResult,
    ListWorkspacesResult, MessageId, PausePoint, PausePointId, PausePointPosition,
    PausePointTarget, PreCheckOutcome, ProposedScopePatch, Repo, RepoId, RestartServerArgs,
    RestartServerResult, Review, ReviewId, ReviewStatus, ReviewVerdict, RunnerRow, ScopePatch,
    ScopePatchId, ScopePatchKind, ScopePatchTarget, SetChatAdapterEnabledArgs,
    SetRunnerEnabledArgs, Stage, StageId, StageStatus, StopReason, Task, TaskId, TaskStatus,
    TodoSelector, UnixMillis, ValidateChatAdapterSecretsArgs, ValidateChatAdapterSecretsResult,
    ValidateWorkspacePathArgs, ValidateWorkspacePathResult, WorkspaceError, WorkspaceProblem,
};
use specta::TypeCollection;
use specta_typescript::{BigIntExportBehavior, Typescript};

fn collect() -> TypeCollection {
    let mut types = TypeCollection::default();
    types
        .register_mut::<RepoId>()
        .register_mut::<JobId>()
        .register_mut::<StageId>()
        .register_mut::<TaskId>()
        .register_mut::<ReviewId>()
        .register_mut::<UnixMillis>()
        .register_mut::<CostCents>()
        .register_mut::<EventCursor>()
        .register_mut::<GitAuth>()
        .register_mut::<Repo>()
        .register_mut::<Job>()
        .register_mut::<JobStatus>()
        .register_mut::<StopReason>()
        .register_mut::<Stage>()
        .register_mut::<StageStatus>()
        .register_mut::<Task>()
        .register_mut::<TaskStatus>()
        .register_mut::<Review>()
        .register_mut::<ReviewStatus>()
        .register_mut::<Event>()
        .register_mut::<EventEnvelope>()
        .register_mut::<FsEntry>()
        .register_mut::<FsEntryKind>()
        .register_mut::<AttachedWorkspace>()
        .register_mut::<AttachWorkspaceArgs>()
        .register_mut::<AttachWorkspaceResult>()
        .register_mut::<ListWorkspacesResult>()
        .register_mut::<DetachWorkspaceArgs>()
        .register_mut::<DetachPolicy>()
        .register_mut::<ValidateWorkspacePathArgs>()
        .register_mut::<ValidateWorkspacePathResult>()
        .register_mut::<WorkspaceProblem>()
        .register_mut::<WorkspaceError>()
        .register_mut::<AssistantThreadId>()
        .register_mut::<AssistantMessageId>()
        .register_mut::<AssistantAttachmentId>()
        .register_mut::<AssistantThread>()
        .register_mut::<AssistantMessage>()
        .register_mut::<AssistantMessageRole>()
        .register_mut::<AssistantAttachment>()
        .register_mut::<AssistantAction>()
        .register_mut::<AssistantActionStatus>()
        .register_mut::<AssistantActionCard>()
        .register_mut::<AttachmentRef>()
        .register_mut::<AssistantAttachmentCardItem>()
        .register_mut::<AssistantAttachmentCard>()
        .register_mut::<ScopePatchId>()
        .register_mut::<ScopePatchKind>()
        .register_mut::<ScopePatchTarget>()
        .register_mut::<ScopePatch>()
        .register_mut::<ProposedScopePatch>()
        .register_mut::<PreCheckOutcome>()
        .register_mut::<ReviewVerdict>()
        .register_mut::<PausePointId>()
        .register_mut::<PausePointPosition>()
        .register_mut::<TodoSelector>()
        .register_mut::<PausePointTarget>()
        .register_mut::<PausePoint>()
        .register_mut::<MessageId>()
        .register_mut::<ChatTransport>()
        .register_mut::<ChatRole>()
        .register_mut::<ChatMessage>()
        .register_mut::<ChatBinding>()
        .register_mut::<ChatAdapterKind>()
        .register_mut::<ChatAdapterRow>()
        .register_mut::<RunnerRow>()
        .register_mut::<ListChatAdaptersResult>()
        .register_mut::<ListRunnersResult>()
        .register_mut::<SetChatAdapterEnabledArgs>()
        .register_mut::<SetRunnerEnabledArgs>()
        .register_mut::<ValidateChatAdapterSecretsArgs>()
        .register_mut::<ValidateChatAdapterSecretsResult>()
        .register_mut::<ChatAdapterSecretProblem>()
        .register_mut::<RestartServerArgs>()
        .register_mut::<RestartServerResult>()
        .register_mut::<AdapterError>();
    types
}

fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wire.ts.snap")
}

#[test]
fn wire_types_match_snapshot() {
    let types = collect();
    // `Number` (rather than `BigInt`/`String`) because serde_json
    // round-trips i64 as a JSON number and every i64 we emit (cents,
    // unix-ms, lease expirations, cost caps) fits inside JS's
    // `Number.MAX_SAFE_INTEGER` window for the foreseeable scale —
    // SCOPE.md "Cost" caps in dollars-of-cents, and Unix-ms is good
    // until year 287396. Revisit if a column ever needs > 2^53-1.
    let ts = Typescript::default().bigint(BigIntExportBehavior::Number);
    let rendered = ts.export(&types).expect("export typescript");

    let path = snapshot_path();
    if std::env::var("SPECTA_UPDATE").is_ok() {
        std::fs::write(&path, &rendered).expect("write snapshot");
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "snapshot {} missing ({e}). Run with SPECTA_UPDATE=1 to create it.",
            path.display()
        )
    });

    if expected != rendered {
        let diff_path = path.with_extension("snap.actual");
        std::fs::write(&diff_path, &rendered).expect("write actual");
        panic!(
            "specta TS snapshot drift. Compare {} vs {}; rerun with \
             SPECTA_UPDATE=1 if the change is intended.",
            path.display(),
            diff_path.display(),
        );
    }
}

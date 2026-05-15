//! CLAUDE.md R2 — no emojis in source files. Anywhere. Ever.
//!
//! The rule is a hygiene rail more than a stylistic preference: emojis
//! in code, doc comments, or commit-tracked prose erode the signal that
//! a comment is load-bearing intent. A probe is the only way to keep
//! the rail from drifting one PR at a time.
//!
//! Detection uses Unicode code-point ranges rather than a library so
//! the crate stays dependency-free for Step 3. The ranges below cover
//! the practical emoji surface (emoticons, miscellaneous symbols and
//! pictographs, transport and map, supplemental pictographs, regional
//! indicators, dingbats), plus the variation selector U+FE0F and the
//! zero-width joiner U+200D that drive emoji presentation. A character
//! in one of these ranges in a tracked source file is a violation.
//!
//! Self-skip: the probe's own sources spell out the code-point ranges
//! and ship test fixtures with emoji literals; scanning the predicate
//! crate would always flag the probe. The exclusion is keyed on the
//! crate's source prefix, identical to the other probes.

use crate::{norm_path, ChangedFile, Violation};

const PROBE: &str = "no-emojis-in-source";
const SELF_PREFIX: &str = "crates/codeless-predicates/";

/// Tracked text extensions. The rule is "anywhere"; this list is the
/// pragmatic set of files a stage's diff actually carries. Binary
/// blobs (PNG, ICO) are handled at the file-read layer — they fail
/// UTF-8 decoding and never reach a probe.
const TEXT_EXTS: &[&str] = &[
    ".rs", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".py", ".go", ".md", ".toml", ".yaml",
    ".yml", ".json", ".html", ".css", ".scss", ".sh", ".sql", ".txt",
];

pub fn run(files: &[ChangedFile]) -> Vec<Violation> {
    let mut out = Vec::new();
    for file in files {
        let path = norm_path(&file.path);
        if path.starts_with(SELF_PREFIX) {
            continue;
        }
        if !TEXT_EXTS.iter().any(|ext| path.ends_with(ext)) {
            continue;
        }
        for (idx, line) in file.content.lines().enumerate() {
            if let Some(c) = line.chars().find(|c| is_emoji_codepoint(*c)) {
                out.push(Violation {
                    probe: PROBE,
                    path: file.path.clone(),
                    line: Some(idx + 1),
                    message: format!("emoji U+{:04X} in source per CLAUDE.md R2", c as u32),
                });
            }
        }
    }
    out
}

/// True for code points that present as emoji in tracked text. Tight
/// enough to avoid flagging mathematical symbols and Latin extended
/// punctuation; broad enough to catch the common pictographic ranges.
fn is_emoji_codepoint(c: char) -> bool {
    let cp = c as u32;
    matches!(cp,
        0x1F300..=0x1F5FF // misc symbols + pictographs
        | 0x1F600..=0x1F64F // emoticons
        | 0x1F680..=0x1F6FF // transport + map
        | 0x1F700..=0x1F77F // alchemical symbols
        | 0x1F780..=0x1F7FF // geometric shapes extended
        | 0x1F800..=0x1F8FF // supplemental arrows-C
        | 0x1F900..=0x1F9FF // supplemental symbols + pictographs
        | 0x1FA00..=0x1FAFF // chess + symbols + pictographs extended-A
        | 0x1F1E6..=0x1F1FF // regional indicators
        | 0x2600..=0x26FF   // miscellaneous symbols
        | 0x2700..=0x27BF   // dingbats
        | 0xFE0F            // variation selector emoji presentation
        | 0x200D            // zero-width joiner
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(path: &str, content: &str) -> ChangedFile {
        ChangedFile {
            path: PathBuf::from(path),
            content: content.to_string(),
        }
    }

    #[test]
    fn flags_pictograph_in_rust_source() {
        // U+1F680 ROCKET.
        let rocket = char::from_u32(0x1F680).unwrap();
        let content = format!("// kickoff {rocket} test\n");
        let files = vec![file("crates/codeless-runtime/src/foo.rs", &content)];
        let v = run(&files);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].line, Some(1));
    }

    #[test]
    fn flags_dingbat() {
        // U+2705 WHITE HEAVY CHECK MARK.
        let check = char::from_u32(0x2705).unwrap();
        let content = format!("PASS {check}\n");
        let files = vec![file("README.md", &content)];
        assert_eq!(run(&files).len(), 1);
    }

    #[test]
    fn allows_plain_ascii() {
        let files = vec![file(
            "crates/codeless-runtime/src/foo.rs",
            "fn main() { println!(\"hello\"); }\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn allows_latin_extended_diacritics() {
        // U+00E9 LATIN SMALL LETTER E WITH ACUTE is text, not emoji.
        let files = vec![file("DOCS/notes.md", "café résumé\n")];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn skips_non_text_extensions() {
        let rocket = char::from_u32(0x1F680).unwrap();
        let content = format!("binary-ish {rocket}\n");
        let files = vec![file("assets/icon.png.txt.bin", &content)];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn skips_self_crate() {
        let rocket = char::from_u32(0x1F680).unwrap();
        let content = format!("// test fixture {rocket}\n");
        let files = vec![file(
            "crates/codeless-predicates/src/probes/no_emojis.rs",
            &content,
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn flags_each_offending_line() {
        let rocket = char::from_u32(0x1F680).unwrap();
        let check = char::from_u32(0x2705).unwrap();
        let content = format!("line one {rocket}\nline two {check}\n");
        let files = vec![file("DOCS/notes.md", &content)];
        let v = run(&files);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].line, Some(1));
        assert_eq!(v[1].line, Some(2));
    }
}

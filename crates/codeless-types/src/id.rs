use serde::{Deserialize, Serialize};
use ulid::Ulid;

macro_rules! ulid_newtype {
    ($name:ident, $desc:literal) => {
        #[doc = $desc]
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
            specta::Type,
        )]
        #[serde(transparent)]
        #[specta(transparent)]
        pub struct $name(#[specta(type = String)] pub Ulid);

        impl $name {
            pub fn new() -> Self {
                Self(Ulid::new())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<Ulid> for $name {
            fn from(u: Ulid) -> Self {
                Self(u)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ulid::from_str(s).map(Self)
            }
        }
    };
}

ulid_newtype!(RepoId, "Identity of a managed git repository row.");
ulid_newtype!(JobId, "Identity of one unit of work scoped to one repo.");
ulid_newtype!(StageId, "Identity of a verify-gated chunk within a job.");
ulid_newtype!(
    TaskId,
    "Identity of one atomic runner invocation within a stage."
);
ulid_newtype!(ReviewId, "Identity of a review gate attached to a stage.");
ulid_newtype!(
    TodoId,
    "Identity of one user-visible sub-step within a task (`todos` row)."
);
ulid_newtype!(
    AssistantThreadId,
    "Identity of one conversational thread on the /assistant surface."
);
ulid_newtype!(
    AssistantMessageId,
    "Identity of one persisted turn (user, assistant, system, or tool) in an assistant thread."
);
ulid_newtype!(
    AssistantAttachmentId,
    "Identity of one file uploaded into an assistant thread (`<codeless-data>/threads/<thread_id>/attachments/`)."
);
ulid_newtype!(
    MessageId,
    "Identity of one row in `chat_messages` — the per-Job chat substrate from `DOCS/JOB-CHAT.md`. Shared by every transport (web, CLI, Telegram, Slack, supervisor); the ULID is minted by the runtime on `post_job_message`, never by the transport adapter."
);

//! Specta TypeScript codegen snapshot for the RPC surface. This snapshot
//! covers method argument and result types plus the subscribe filter,
//! sitting alongside the core-domain snapshot in
//! `codeless-types::tests::specta_snapshot`. Two snapshots, one per
//! crate, avoids a `codeless-types -> codeless-rpc` dev-dependency cycle
//! while still keeping the wire contract enumerable.
//!
//! Regenerate with
//! `SPECTA_UPDATE=1 cargo test -p codeless-rpc --test specta_snapshot`.

use std::path::PathBuf;

use codeless_rpc::methods::{
    AddRepoArgs, AgentChatArgs, AgentChatResult, AppendAssistantMessageArgs,
    AppendAssistantMessageResult, ApproveReviewArgs, ApproveScopePatchArgs, BindChatThreadArgs,
    CancelAssistantActionArgs, CancelAssistantActionResult, CancelChatTaskArgs, ClaudeStatus,
    CommentReviewArgs, ConfirmAssistantActionArgs, ConfirmAssistantActionResult,
    CreateAssistantThreadArgs, DeleteAssistantThreadArgs, DeleteJobFileArgs,
    DraftJobFromConversationArgs, EditScopePatchArgs, FsCwdResult, FsReadDirArgs, FsReadDirResult,
    FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs, GetJobArgs,
    JobDiffArgs, JobDiffFile, JobDiffResult, JobFileEntry, ListAssistantMessagesArgs,
    ListAssistantMessagesResult, ListAssistantThreadsArgs, ListAssistantThreadsResult,
    ListJobFilesArgs, ListJobFilesResult, ListJobMessagesArgs, ListJobMessagesResult, ListJobsArgs,
    ListJobsResult, ListReposResult, ListReviewsArgs, ListReviewsResult, ListStagesArgs,
    ListStagesResult, PostJobMessageArgs, ReadJobFileArgs, ReadJobFileResult, RejectScopePatchArgs,
    RemoveRepoArgs, RevertScopePatchArgs, RevertScopePatchResult, RunnerInfo,
    ScopePatchActionResult, ScopePatchResolution, ServerFeatureFlags, ServerInfo, SetJobPolicyArgs,
    StageRollup, StopActiveArgs, StopActiveResult, StopJobArgs, StopReviewArgs, SubmitJobArgs,
    UpdateJobScopeArgs, UpdateJobScopeResult, UpdateJobTemplateArgs, UpdateJobTemplateResult,
    UploadAssistantAttachmentArgs, UploadAssistantAttachmentResult, WriteHandoverArgs,
    WriteHandoverResult, WriteJobFileArgs, WriteJobFileResult,
};
use codeless_rpc::subscribe::EventFilter;
use specta::TypeCollection;
use specta_typescript::{BigIntExportBehavior, Typescript};

fn collect() -> TypeCollection {
    let mut types = TypeCollection::default();
    types
        .register_mut::<AddRepoArgs>()
        .register_mut::<RemoveRepoArgs>()
        .register_mut::<ListReposResult>()
        .register_mut::<SubmitJobArgs>()
        .register_mut::<GetJobArgs>()
        .register_mut::<ListJobsArgs>()
        .register_mut::<ListJobsResult>()
        .register_mut::<StopJobArgs>()
        .register_mut::<ListReviewsArgs>()
        .register_mut::<ListReviewsResult>()
        .register_mut::<ApproveReviewArgs>()
        .register_mut::<CommentReviewArgs>()
        .register_mut::<StopReviewArgs>()
        .register_mut::<EventFilter>()
        .register_mut::<FsReadDirArgs>()
        .register_mut::<FsReadDirResult>()
        .register_mut::<FsReadFileArgs>()
        .register_mut::<FsReadFileResult>()
        .register_mut::<FsWriteFileArgs>()
        .register_mut::<FsStatArgs>()
        .register_mut::<FsStatResult>()
        .register_mut::<FsCwdResult>()
        .register_mut::<RunnerInfo>()
        .register_mut::<ClaudeStatus>()
        .register_mut::<ServerInfo>()
        .register_mut::<ServerFeatureFlags>()
        .register_mut::<JobDiffArgs>()
        .register_mut::<JobDiffFile>()
        .register_mut::<JobDiffResult>()
        .register_mut::<ListJobFilesArgs>()
        .register_mut::<JobFileEntry>()
        .register_mut::<ListJobFilesResult>()
        .register_mut::<ReadJobFileArgs>()
        .register_mut::<ReadJobFileResult>()
        .register_mut::<WriteJobFileArgs>()
        .register_mut::<WriteJobFileResult>()
        .register_mut::<DeleteJobFileArgs>()
        .register_mut::<UpdateJobTemplateArgs>()
        .register_mut::<UpdateJobTemplateResult>()
        .register_mut::<WriteHandoverArgs>()
        .register_mut::<WriteHandoverResult>()
        .register_mut::<StageRollup>()
        .register_mut::<ListStagesArgs>()
        .register_mut::<ListStagesResult>()
        .register_mut::<AgentChatArgs>()
        .register_mut::<AgentChatResult>()
        .register_mut::<CancelChatTaskArgs>()
        .register_mut::<StopActiveArgs>()
        .register_mut::<StopActiveResult>()
        .register_mut::<ListAssistantThreadsArgs>()
        .register_mut::<ListAssistantThreadsResult>()
        .register_mut::<CreateAssistantThreadArgs>()
        .register_mut::<DeleteAssistantThreadArgs>()
        .register_mut::<UploadAssistantAttachmentArgs>()
        .register_mut::<UploadAssistantAttachmentResult>()
        .register_mut::<ListAssistantMessagesArgs>()
        .register_mut::<ListAssistantMessagesResult>()
        .register_mut::<AppendAssistantMessageArgs>()
        .register_mut::<AppendAssistantMessageResult>()
        .register_mut::<ConfirmAssistantActionArgs>()
        .register_mut::<ConfirmAssistantActionResult>()
        .register_mut::<CancelAssistantActionArgs>()
        .register_mut::<CancelAssistantActionResult>()
        .register_mut::<UpdateJobScopeArgs>()
        .register_mut::<UpdateJobScopeResult>()
        .register_mut::<DraftJobFromConversationArgs>()
        .register_mut::<ApproveScopePatchArgs>()
        .register_mut::<RejectScopePatchArgs>()
        .register_mut::<EditScopePatchArgs>()
        .register_mut::<ScopePatchResolution>()
        .register_mut::<ScopePatchActionResult>()
        .register_mut::<RevertScopePatchArgs>()
        .register_mut::<RevertScopePatchResult>()
        .register_mut::<SetJobPolicyArgs>()
        .register_mut::<PostJobMessageArgs>()
        .register_mut::<ListJobMessagesArgs>()
        .register_mut::<ListJobMessagesResult>()
        .register_mut::<BindChatThreadArgs>();
    types
}

fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wire-rpc.ts.snap")
}

#[test]
fn rpc_wire_types_match_snapshot() {
    let types = collect();
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

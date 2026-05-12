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
    AddRepoArgs, ApproveReviewArgs, CommentReviewArgs, FsCwdResult, FsReadDirArgs, FsReadDirResult,
    FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs, GetJobArgs,
    ListJobsArgs, ListJobsResult, ListReposResult, ListReviewsArgs, ListReviewsResult,
    RemoveRepoArgs, StopJobArgs, StopReviewArgs, SubmitJobArgs,
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
        .register_mut::<FsCwdResult>();
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

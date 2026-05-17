use codeless_rpc::{
    DeleteJobArgs, DeleteJobFileArgs, DraftJobFromConversationArgs, GcWorktreesArgs,
    GcWorktreesResult, GetJobArgs, JobDiffArgs, JobDiffResult, JobReportArgs, JobReportResult,
    ListJobFilesArgs, ListJobFilesResult, ListJobsArgs, ListJobsResult, ListStagesArgs,
    ListStagesResult, PauseJobArgs, ReadJobFileArgs, ReadJobFileResult, RerunJobArgs, ResetJobArgs,
    ResumeJobArgs, StartJobArgs, StopJobArgs, SubmitJobArgs, UpdateJobArgs, UpdateJobScopeArgs,
    UpdateJobScopeResult, UpdateJobTemplateArgs, UpdateJobTemplateResult, WriteHandoverArgs,
    WriteHandoverResult, WriteJobFileArgs, WriteJobFileResult,
};
use codeless_types::Job;
use tauri::State;

use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub async fn rpc_submit_job(state: State<'_, AppState>, args: SubmitJobArgs) -> CommandResult<Job> {
    Ok(state.rpc.submit_job(args).await?)
}

#[tauri::command]
pub async fn rpc_get_job(state: State<'_, AppState>, args: GetJobArgs) -> CommandResult<Job> {
    Ok(state.rpc.get_job(args).await?)
}

#[tauri::command]
pub async fn rpc_list_jobs(
    state: State<'_, AppState>,
    args: ListJobsArgs,
) -> CommandResult<ListJobsResult> {
    Ok(state.rpc.list_jobs(args).await?)
}

#[tauri::command]
pub async fn rpc_stop_job(state: State<'_, AppState>, args: StopJobArgs) -> CommandResult<()> {
    Ok(state.rpc.stop_job(args).await?)
}

#[tauri::command]
pub async fn rpc_update_job(state: State<'_, AppState>, args: UpdateJobArgs) -> CommandResult<Job> {
    Ok(state.rpc.update_job(args).await?)
}

#[tauri::command]
pub async fn rpc_delete_job(state: State<'_, AppState>, args: DeleteJobArgs) -> CommandResult<()> {
    Ok(state.rpc.delete_job(args).await?)
}

#[tauri::command]
pub async fn rpc_pause_job(state: State<'_, AppState>, args: PauseJobArgs) -> CommandResult<()> {
    Ok(state.rpc.pause_job(args).await?)
}

#[tauri::command]
pub async fn rpc_start_job(state: State<'_, AppState>, args: StartJobArgs) -> CommandResult<Job> {
    Ok(state.rpc.start_job(args).await?)
}

#[tauri::command]
pub async fn rpc_resume_job(state: State<'_, AppState>, args: ResumeJobArgs) -> CommandResult<Job> {
    Ok(state.rpc.resume_job(args).await?)
}

#[tauri::command]
pub async fn rpc_reset_job(state: State<'_, AppState>, args: ResetJobArgs) -> CommandResult<Job> {
    Ok(state.rpc.reset_job(args).await?)
}

#[tauri::command]
pub async fn rpc_list_stages(
    state: State<'_, AppState>,
    args: ListStagesArgs,
) -> CommandResult<ListStagesResult> {
    Ok(state.rpc.list_stages(args).await?)
}

#[tauri::command]
pub async fn rpc_job_report(
    state: State<'_, AppState>,
    args: JobReportArgs,
) -> CommandResult<JobReportResult> {
    Ok(state.rpc.job_report(args).await?)
}

#[tauri::command]
pub async fn rpc_rerun_job(state: State<'_, AppState>, args: RerunJobArgs) -> CommandResult<Job> {
    Ok(state.rpc.rerun_job(args).await?)
}

#[tauri::command]
pub async fn rpc_gc_worktrees(
    state: State<'_, AppState>,
    args: GcWorktreesArgs,
) -> CommandResult<GcWorktreesResult> {
    Ok(state.rpc.gc_worktrees(args).await?)
}

#[tauri::command]
pub async fn rpc_job_diff(
    state: State<'_, AppState>,
    args: JobDiffArgs,
) -> CommandResult<JobDiffResult> {
    Ok(state.rpc.job_diff(args).await?)
}

#[tauri::command]
pub async fn rpc_list_job_files(
    state: State<'_, AppState>,
    args: ListJobFilesArgs,
) -> CommandResult<ListJobFilesResult> {
    Ok(state.rpc.list_job_files(args).await?)
}

#[tauri::command]
pub async fn rpc_read_job_file(
    state: State<'_, AppState>,
    args: ReadJobFileArgs,
) -> CommandResult<ReadJobFileResult> {
    Ok(state.rpc.read_job_file(args).await?)
}

#[tauri::command]
pub async fn rpc_write_job_file(
    state: State<'_, AppState>,
    args: WriteJobFileArgs,
) -> CommandResult<WriteJobFileResult> {
    Ok(state.rpc.write_job_file(args).await?)
}

#[tauri::command]
pub async fn rpc_delete_job_file(
    state: State<'_, AppState>,
    args: DeleteJobFileArgs,
) -> CommandResult<()> {
    Ok(state.rpc.delete_job_file(args).await?)
}

#[tauri::command]
pub async fn rpc_update_job_template(
    state: State<'_, AppState>,
    args: UpdateJobTemplateArgs,
) -> CommandResult<UpdateJobTemplateResult> {
    Ok(state.rpc.update_job_template(args).await?)
}

#[tauri::command]
pub async fn rpc_write_handover(
    state: State<'_, AppState>,
    args: WriteHandoverArgs,
) -> CommandResult<WriteHandoverResult> {
    Ok(state.rpc.write_handover(args).await?)
}

#[tauri::command]
pub async fn rpc_update_job_scope(
    state: State<'_, AppState>,
    args: UpdateJobScopeArgs,
) -> CommandResult<UpdateJobScopeResult> {
    Ok(state.rpc.update_job_scope(args).await?)
}

#[tauri::command]
pub async fn rpc_draft_job_from_conversation(
    state: State<'_, AppState>,
    args: DraftJobFromConversationArgs,
) -> CommandResult<Job> {
    Ok(state.rpc.draft_job_from_conversation(args).await?)
}

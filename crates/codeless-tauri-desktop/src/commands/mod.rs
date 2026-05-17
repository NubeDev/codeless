pub mod assistant;
pub mod chat;
pub mod fs;
pub mod jobs;
pub mod personas;
pub mod repos;
pub mod reviews;
pub mod scope_patches;
pub mod server_info;
pub mod subscribe;
pub mod workspaces;

pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        server_info::rpc_server_info,
        subscribe::rpc_subscribe,
        subscribe::rpc_unsubscribe,
        repos::rpc_add_repo,
        repos::rpc_remove_repo,
        repos::rpc_list_repos,
        jobs::rpc_submit_job,
        jobs::rpc_get_job,
        jobs::rpc_list_jobs,
        jobs::rpc_stop_job,
        jobs::rpc_update_job,
        jobs::rpc_delete_job,
        jobs::rpc_pause_job,
        jobs::rpc_start_job,
        jobs::rpc_resume_job,
        jobs::rpc_reset_job,
        jobs::rpc_list_stages,
        jobs::rpc_job_report,
        jobs::rpc_rerun_job,
        jobs::rpc_gc_worktrees,
        jobs::rpc_job_diff,
        jobs::rpc_list_job_files,
        jobs::rpc_read_job_file,
        jobs::rpc_write_job_file,
        jobs::rpc_delete_job_file,
        jobs::rpc_update_job_template,
        jobs::rpc_write_handover,
        jobs::rpc_update_job_scope,
        jobs::rpc_draft_job_from_conversation,
        fs::rpc_fs_read_dir,
        fs::rpc_fs_read_file,
        fs::rpc_fs_write_file,
        fs::rpc_fs_stat,
        fs::rpc_fs_cwd,
        fs::rpc_fs_create_file,
        fs::rpc_fs_create_dir,
        fs::rpc_fs_move,
        fs::rpc_fs_delete,
        reviews::rpc_list_reviews,
        reviews::rpc_approve_review,
        reviews::rpc_comment_review,
        reviews::rpc_stop_review,
        assistant::rpc_list_assistant_threads,
        assistant::rpc_create_assistant_thread,
        assistant::rpc_delete_assistant_thread,
        assistant::rpc_upload_assistant_attachment,
        assistant::rpc_list_assistant_messages,
        assistant::rpc_append_assistant_message,
        assistant::rpc_confirm_assistant_action,
        assistant::rpc_cancel_assistant_action,
        chat::rpc_agent_chat,
        chat::rpc_upload_chat_attachment,
        chat::rpc_cancel_chat_task,
        chat::rpc_stop_active,
        personas::rpc_list_personas,
        personas::rpc_get_persona,
        personas::rpc_upsert_persona,
        personas::rpc_delete_persona,
        scope_patches::rpc_approve_scope_patch,
        scope_patches::rpc_reject_scope_patch,
        scope_patches::rpc_edit_scope_patch,
        scope_patches::rpc_revert_scope_patch,
        scope_patches::rpc_list_proposed_patches,
        scope_patches::rpc_set_job_policy,
        workspaces::rpc_attach_workspace,
        workspaces::rpc_detach_workspace,
        workspaces::rpc_list_workspaces,
        workspaces::rpc_validate_workspace_path,
    ]
}

use codeless_rpc::{
    DeletePersonaArgs, GetPersonaArgs, ListPersonasArgs, ListPersonasResult, UpsertPersonaArgs,
};
use codeless_types::Persona;
use tauri::State;

use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub async fn rpc_list_personas(
    state: State<'_, AppState>,
    args: ListPersonasArgs,
) -> CommandResult<ListPersonasResult> {
    Ok(state.rpc.list_personas(args).await?)
}

#[tauri::command]
pub async fn rpc_get_persona(
    state: State<'_, AppState>,
    args: GetPersonaArgs,
) -> CommandResult<Persona> {
    Ok(state.rpc.get_persona(args).await?)
}

#[tauri::command]
pub async fn rpc_upsert_persona(
    state: State<'_, AppState>,
    args: UpsertPersonaArgs,
) -> CommandResult<Persona> {
    Ok(state.rpc.upsert_persona(args).await?)
}

#[tauri::command]
pub async fn rpc_delete_persona(
    state: State<'_, AppState>,
    args: DeletePersonaArgs,
) -> CommandResult<()> {
    Ok(state.rpc.delete_persona(args).await?)
}

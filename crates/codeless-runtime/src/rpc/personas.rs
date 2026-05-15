use codeless_rpc::{
    DeletePersonaArgs, GetPersonaArgs, ListPersonasArgs, ListPersonasResult, RpcError, RpcResult,
    UpsertPersonaArgs,
};
use codeless_types::Persona;

use super::InProcessRpc;
use crate::time::now_ms;

pub(super) async fn list_personas(
    rpc: &InProcessRpc,
    _args: ListPersonasArgs,
) -> RpcResult<ListPersonasResult> {
    let personas = rpc.store.list_personas().await.map_err(super::db_err)?;
    Ok(ListPersonasResult { personas })
}

pub(super) async fn get_persona(rpc: &InProcessRpc, args: GetPersonaArgs) -> RpcResult<Persona> {
    rpc.store
        .get_persona(&args.id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("persona {}", args.id)))
}

pub(super) async fn upsert_persona(
    rpc: &InProcessRpc,
    args: UpsertPersonaArgs,
) -> RpcResult<Persona> {
    if args.id.trim().is_empty() {
        return Err(RpcError::InvalidArgument("persona id is empty".into()));
    }
    if args.name.trim().is_empty() {
        return Err(RpcError::InvalidArgument("persona name is empty".into()));
    }
    if args.instructions.trim().is_empty() {
        return Err(RpcError::InvalidArgument(
            "persona instructions are empty".into(),
        ));
    }
    let now = now_ms();
    // built_in / created_at are placeholders — the store overwrites
    // built_in with the existing value (or 0 for new rows) and
    // preserves the prior created_at when the row already existed.
    let candidate = Persona {
        id: args.id,
        name: args.name,
        description: args.description,
        icon: args.icon,
        instructions: args.instructions,
        use_for_jobs: args.use_for_jobs,
        default_model: args.default_model,
        allowed_subagents: args.allowed_subagents,
        default_snippets: args.default_snippets,
        built_in: false,
        created_at: now,
        updated_at: now,
    };
    rpc.store
        .upsert_persona(&candidate)
        .await
        .map_err(super::db_err)
}

pub(super) async fn delete_persona(rpc: &InProcessRpc, args: DeletePersonaArgs) -> RpcResult<()> {
    let existing = rpc
        .store
        .get_persona(&args.id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("persona {}", args.id)))?;
    if existing.built_in {
        return Err(RpcError::Conflict(format!(
            "persona {} is built-in and cannot be deleted",
            args.id
        )));
    }
    let removed = rpc
        .store
        .delete_persona(&args.id)
        .await
        .map_err(super::db_err)?;
    if !removed {
        return Err(RpcError::NotFound(format!("persona {}", args.id)));
    }
    Ok(())
}

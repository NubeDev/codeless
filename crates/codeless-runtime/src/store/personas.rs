use codeless_types::Persona;

use super::codec::{persona_from_row, serde_err};
use super::SqliteStore;

impl SqliteStore {
    /// Snapshot every persona row. Built-ins (`built_in = 1`) come
    /// first, ordered by id for a stable rail; user rows follow in
    /// `created_at` order so a freshly minted row lands at the bottom.
    /// JSON columns (`allowed_subagents`, `default_snippets`) are
    /// decoded here so the caller does not have to know the column
    /// shape — the wire type is `Vec<String>` either way.
    pub async fn list_personas(&self) -> sqlx::Result<Vec<Persona>> {
        let rows = sqlx::query(
            "SELECT * FROM personas \
             ORDER BY built_in DESC, \
                      CASE WHEN built_in = 1 THEN id END ASC, \
                      created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(persona_from_row).collect()
    }

    pub async fn get_persona(&self, id: &str) -> sqlx::Result<Option<Persona>> {
        let row = sqlx::query("SELECT * FROM personas WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(persona_from_row).transpose()
    }

    /// Upsert into the personas table. Caller supplies `now` so the
    /// runtime can hold a single timestamp across the surrounding
    /// publish; built-in rows preserve their seeded `created_at` (the
    /// `INSERT OR REPLACE` is replaced with explicit insert/update so
    /// the historical timestamp is not clobbered).
    ///
    /// `built_in` is *not* a parameter — new rows always land with
    /// `built_in = 0`, and existing rows keep whatever value they had.
    /// The runtime enforces "user cannot mint a built-in" without the
    /// schema growing a CHECK constraint.
    pub async fn upsert_persona(&self, persona: &Persona) -> sqlx::Result<Persona> {
        let allowed = serde_json::to_string(&persona.allowed_subagents).map_err(serde_err)?;
        let snippets = serde_json::to_string(&persona.default_snippets).map_err(serde_err)?;
        let allowed_tools = serde_json::to_string(&persona.allowed_tools).map_err(serde_err)?;
        let existing = self.get_persona(&persona.id).await?;
        match existing {
            Some(prev) => {
                sqlx::query(
                    "UPDATE personas SET \
                        name=?, description=?, icon=?, instructions=?, \
                        use_for_jobs=?, default_model=?, allowed_subagents=?, \
                        default_snippets=?, allowed_tools=?, \
                        default_model_family=?, default_attachments_policy=?, \
                        updated_at=? \
                     WHERE id=?",
                )
                .bind(&persona.name)
                .bind(&persona.description)
                .bind(&persona.icon)
                .bind(&persona.instructions)
                .bind(persona.use_for_jobs as i64)
                .bind(&persona.default_model)
                .bind(&allowed)
                .bind(&snippets)
                .bind(&allowed_tools)
                .bind(&persona.default_model_family)
                .bind(&persona.default_attachments_policy)
                .bind(persona.updated_at.0)
                .bind(&persona.id)
                .execute(&self.pool)
                .await?;
                Ok(Persona {
                    built_in: prev.built_in,
                    created_at: prev.created_at,
                    ..persona.clone()
                })
            }
            None => {
                sqlx::query(
                    "INSERT INTO personas \
                        (id, name, description, icon, instructions, use_for_jobs, \
                         default_model, allowed_subagents, default_snippets, built_in, \
                         allowed_tools, default_model_family, \
                         default_attachments_policy, created_at, updated_at) \
                     VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                )
                .bind(&persona.id)
                .bind(&persona.name)
                .bind(&persona.description)
                .bind(&persona.icon)
                .bind(&persona.instructions)
                .bind(persona.use_for_jobs as i64)
                .bind(&persona.default_model)
                .bind(&allowed)
                .bind(&snippets)
                .bind(0_i64)
                .bind(&allowed_tools)
                .bind(&persona.default_model_family)
                .bind(&persona.default_attachments_policy)
                .bind(persona.created_at.0)
                .bind(persona.updated_at.0)
                .execute(&self.pool)
                .await?;
                Ok(Persona {
                    built_in: false,
                    ..persona.clone()
                })
            }
        }
    }

    /// Delete one persona row by id. Returns `true` when a row was
    /// removed. Refusing built-ins is the RPC layer's responsibility
    /// — the store happily removes whatever id it is given so tests
    /// and migrations can clean up freely.
    pub async fn delete_persona(&self, id: &str) -> sqlx::Result<bool> {
        let res = sqlx::query("DELETE FROM personas WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }
}

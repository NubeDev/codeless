//! Codeless ↔ ai-ui glue.
//!
//! Codeless validates ai-ui's BYO-AI design by implementing
//! [`ai_ui_core::Provider`] against the existing `ai-runner` stack —
//! the same runners that power codeless's job loop. No second model
//! client lives in this crate.
//!
//! The provider is mounted by `codeless-server` as the `Provider` half
//! of an [`ai_ui_core::AiUiState`]; the server owns the axum surface
//! (under `/api/ai-ui/*`) and never depends on `ai-ui-axum`. This
//! sidesteps the axum 0.7 / 0.8 version skew between the two repos
//! and proves `ai-ui-core` is usable without any HTTP framework.

pub mod provider;
pub mod sse;

pub use provider::CodelessProvider;

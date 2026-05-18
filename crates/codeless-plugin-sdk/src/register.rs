//! `register!` macro -- the per-flavour packaging seam.
//!
//! Today this is a stub: it type-checks the argument as a
//! [`crate::ToolBehavior`] impl and emits nothing else. The real
//! per-flavour expansion lands in later stages of the
//! plugin-substrate-runtimes job:
//!
//! - `feature = "builtin"` -> `inventory::submit!` entry the host
//!   collects at startup (PLUGIN-WASM.md "register! macro" §,
//!   builtin bullet). Replaces the hand-written `RegisterFn` the
//!   notes plugin uses through `codeless-tools::plugin` today.
//! - `feature = "wasm"`    -> `wit-bindgen` `export tool` glue --
//!   `describe()` returns the manifest list, `call()` matches on
//!   `tool-id` and dispatches into the right `<T as
//!   ToolBehavior>::call` (PLUGIN-WASM.md, wasm bullet).
//! - `feature = "process"` -> reserved; lands with
//!   PLUGIN-PROCESS.md. No-op for now.
//!
//! The stub is deliberately a stub rather than a `todo!()` panic so
//! a plugin author can compile their crate against this SDK today
//! and have it Just Work once the builtin expansion lands -- no
//! source change at the call site. That property is the reason the
//! macro exists at the SDK root in stage 2 rather than being added
//! later alongside the per-flavour glue.

/// Register a tool with the host. Wraps the per-flavour expansion
/// described above.
///
/// Usage:
///
/// ```ignore
/// use codeless_plugin_sdk::{register, Tool};
///
/// #[derive(Tool)]
/// #[tool(id = "notes.append", tier = "write")]
/// pub struct NotesAppend;
///
/// // ... impl ToolBehavior for NotesAppend { ... } ...
///
/// register!(NotesAppend);
/// ```
///
/// The macro is `macro_export`'d at the crate root. `register!` --
/// not `codeless_plugin_sdk::register!` -- is the documented call
/// shape, matching the rubix `extensions-sdk` precedent.
#[macro_export]
macro_rules! register {
    ($t:ty) => {
        // The const-eval block proves at compile time that `$t`
        // implements `ToolBehavior`. Once the per-flavour expansion
        // lands this assertion stays -- a `wit-bindgen` export that
        // dispatches into a type whose `ToolBehavior` impl was
        // accidentally removed would otherwise turn into a runtime
        // dispatch error.
        const _: fn() = || {
            fn __codeless_assert_tool_behavior<T: $crate::ToolBehavior>() {}
            __codeless_assert_tool_behavior::<$t>();
        };
    };
}

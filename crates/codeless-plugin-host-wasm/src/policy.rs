//! Per-call execution limits enforced at the WASM boundary.
//!
//! Resolved 2026-05-18 (plugin-substrate-runtimes stage 1) under
//! OQ-WASM-5 in `DOCS/plugins/PLUGIN-WASM.md`: this struct carries
//! the global defaults; the codeless `config.toml`
//! `[plugins.<id>]` block may *lower* a cap; the plugin manifest
//! itself cannot set fuel / memory / deadline at all (the
//! `codeless-server` manifest parser, stage 13, rejects the fields).
//!
//! Defaults are the table in `PLUGIN-WASM.md § Limits`. They are
//! held here -- not on the Wasmtime [`wasmtime::Config`] -- so a
//! per-plugin override is one struct copy, not a re-instantiated
//! engine.

use std::time::Duration;

/// Caps every WASM call runs under. One value per [`WasmPlugin`];
/// the per-server defaults come from [`HostPolicy::defaults`] and
/// the per-plugin overrides land in stage 13's manifest parser.
///
/// [`WasmPlugin`]: crate::WasmPlugin
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPolicy {
    /// Wasmtime fuel units the store starts each call with. Burned
    /// proportionally to executed instructions; refilled per call,
    /// not per instance. Crossing the cap surfaces as
    /// [`crate::HostError::LimitExceeded`] with reason `fuel`.
    pub fuel: u64,
    /// Linear-memory ceiling for the per-call store. Enforced
    /// through [`wasmtime::StoreLimits::memory_size`]; exceeding it
    /// aborts the call with [`crate::HostError::LimitExceeded`]
    /// reason `memory`. Stored as a `u64` so an operator override
    /// that bumps the cap past `usize::MAX` on a 32-bit host fails
    /// at config-parse rather than at the cast site.
    pub memory_max_bytes: u64,
    /// Wall-clock deadline for the entire call future. Backstops
    /// the fuel cap for "spent fuel but stuck in a host call"
    /// cases (`PLUGIN-WASM.md § Limits`). Implemented as a
    /// [`tokio::time::timeout`] around `call_call`.
    pub deadline: Duration,
}

impl HostPolicy {
    /// Defaults from `PLUGIN-WASM.md § Limits`: 100M fuel units,
    /// 64 MiB linear memory, 10 s deadline. These are the
    /// per-server starting point; the codeless config may lower
    /// any of them per plugin (`[plugins.<id>] fuel = ...`).
    pub const fn defaults() -> Self {
        Self {
            fuel: 100_000_000,
            memory_max_bytes: 64 * 1024 * 1024,
            deadline: Duration::from_secs(10),
        }
    }

    /// Apply an override from the codeless config. Per OQ-WASM-5
    /// the override **must only lower** the cap; this method
    /// returns `Err` if the override exceeds the global default so
    /// the boot-time config-parse can surface the violation as a
    /// structured error rather than silently raising the sandbox.
    pub fn with_override(self, override_: HostPolicyOverride) -> Result<Self, PolicyError> {
        let fuel = match override_.fuel {
            Some(v) if v > self.fuel => return Err(PolicyError::ExceedsCap("fuel")),
            Some(v) => v,
            None => self.fuel,
        };
        let memory_max_bytes = match override_.memory_max_bytes {
            Some(v) if v > self.memory_max_bytes => {
                return Err(PolicyError::ExceedsCap("memory_max_bytes"))
            }
            Some(v) => v,
            None => self.memory_max_bytes,
        };
        let deadline = match override_.deadline {
            Some(d) if d > self.deadline => return Err(PolicyError::ExceedsCap("deadline")),
            Some(d) => d,
            None => self.deadline,
        };
        Ok(Self {
            fuel,
            memory_max_bytes,
            deadline,
        })
    }
}

impl Default for HostPolicy {
    fn default() -> Self {
        Self::defaults()
    }
}

/// Per-plugin override carried in `config.toml`'s
/// `[plugins.<id>]` block. Each field is `Option`-typed so an
/// override that touches only `fuel` leaves the other two at the
/// global default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostPolicyOverride {
    pub fuel: Option<u64>,
    pub memory_max_bytes: Option<u64>,
    pub deadline: Option<Duration>,
}

/// Errors surfaced by [`HostPolicy::with_override`]. The carried
/// `&'static str` is the field name, ready for inclusion in the
/// operator-facing config-parse error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("override for `{0}` exceeds the global default; overrides may only lower the cap")]
    ExceedsCap(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_doc_table() {
        let p = HostPolicy::defaults();
        assert_eq!(p.fuel, 100_000_000);
        assert_eq!(p.memory_max_bytes, 64 * 1024 * 1024);
        assert_eq!(p.deadline, Duration::from_secs(10));
    }

    #[test]
    fn override_may_only_lower() {
        let base = HostPolicy::defaults();
        let lowered = base
            .with_override(HostPolicyOverride {
                fuel: Some(50_000_000),
                ..Default::default()
            })
            .expect("lowering fuel is allowed");
        assert_eq!(lowered.fuel, 50_000_000);

        let err = base
            .with_override(HostPolicyOverride {
                memory_max_bytes: Some(u64::MAX),
                ..Default::default()
            })
            .expect_err("raising memory must fail");
        assert_eq!(err, PolicyError::ExceedsCap("memory_max_bytes"));
    }
}

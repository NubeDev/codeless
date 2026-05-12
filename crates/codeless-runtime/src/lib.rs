// The job-runtime brain — state machine, scheduler, queue, event bus,
// SQLite via sqlx. See the `codeless-runtime` row of the crate table
// in DOCS/SCOPE.md. Host-only by design: mobile shells reach the
// runtime only over the network via `codeless-client`. Process spawn
// and PTY live in `codeless-adapters-host` precisely so this crate can
// stay focused on orchestration.

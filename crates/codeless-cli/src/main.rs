// The `codeless` CLI binary — see the `codeless-cli` row of the crate
// table in DOCS/SCOPE.md. Local-mode invocations call `codeless-runtime`
// in-process and skip auth (same trust boundary as the invoking user);
// hosted-mode invocations (`codeless --core https://… [--token …]`) use
// `codeless-client` and authenticate via the bearer token from
// `~/.config/codeless/auth.toml`.

fn main() {}

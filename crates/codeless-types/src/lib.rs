// The serde data types shared across the wire — see the `codeless-types`
// row of the crate table in DOCS/SCOPE.md. No I/O lives here; this crate
// must remain iOS- and Android-safe so the mobile shell can depend on
// it directly via `codeless-client`.

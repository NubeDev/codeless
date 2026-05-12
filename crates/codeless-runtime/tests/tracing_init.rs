//! `try_init_json` must succeed once per process and reject the
//! second call rather than panicking — the CLI/server callers treat
//! the first error as fatal but tests rely on the idempotency
//! contract so test binaries can race subscriber setup.

use codeless_runtime::try_init_json;

#[test]
fn try_init_json_is_idempotent() {
    let first = try_init_json();
    assert!(first.is_ok(), "first init failed: {first:?}");
    let second = try_init_json();
    assert!(second.is_err(), "second init must report already-set");
}

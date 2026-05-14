# Chat for this job

## user @ 2026-05-14T13:08:02Z

help me write a spec to make a simple rust cli
- stage 1 wirte the code
- stage 2 test it
- stage 3 add an extra festiore and retest

## assistant @ 2026-05-14T13:08:46Z

Spec drafted. Three stages wired up in `template.yaml`, each pointing at its own doc:

- `SCOPE.md` — single binary crate using `clap`, with one observable command; deps kept minimal; no `unwrap` outside tests.
- `WORKFLOW.md` — strict ordering, end-of-stage gates (`cargo build`/`test`/`clippy -D warnings`), no `--no-verify`.
- `stage-1-write.md` — scaffold the crate + one subcommand (suggested `greet <name>`).
- `stage-2-test.md` — extract core logic, add unit tests + an `assert_cmd` integration test.
- `stage-3-extend.md` — pick one small feature (`--shout`, `count`, or `--repeat`), add tests, keep old ones green.

A couple of things you may want to tweak before clicking run:
- The crate name (`asd-cli`) and exact command shape (`greet <name>`) are suggestions in SCOPE — pin them if you have a preference.
- Stage 3 lists three feature options; if you already know which one, narrow it to one to remove drift risk.

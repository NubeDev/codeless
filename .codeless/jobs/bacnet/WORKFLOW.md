# Workflow — bacnet power-meter example

How the agent drives the three stages. The stages are independent
add-only changes; nothing is rewritten between them, so a failure
in stage 2 still leaves stage 1's CSV on the branch.

## Stage 1 — expand the CSV

Generate `examples/power-meter.csv` directly, no script. Write the
file as a single deterministic block of rows:

- 5 meters: `PM-001` .. `PM-005`.
- 24 hours of timestamps starting at `2026-01-01T00:00:00Z`,
  one row every 5 minutes -> 288 rows per meter, 1,440 total.
- Two meters (`PM-001`, `PM-002`) follow a daily duty cycle:
  load ramps up morning, peaks afternoon, decays evening.
- The other three meters stay roughly flat with small noise.
- `energy_kwh` is monotonic per meter and consistent with the
  `power_w` integrated over time.

Commit as `bacnet stage 1: expand power-meter.csv to 5 meters x 24h`.
Verify by reading the first and last rows back; the file must have
1,441 lines total (header + 1,440 data rows).

## Stage 2 — analyser

Write `examples/analyse_power.py`. Stdlib only (`csv`,
`collections`, `datetime`, `argparse`). The script:

1. Accepts an optional `--csv PATH` argument; default
   `examples/power-meter.csv` resolved relative to the script.
2. Groups rows by `meter_id` and by UTC date.
3. For each `(meter, date)` prints one line:
   `PM-001 2026-01-01 kWh=12.34 peak_w=1850.0`.
4. Exits non-zero on any malformed row, with a message naming the
   offending line number.

Commit as `bacnet stage 2: add analyse_power.py`. Verify by running
`python3 examples/analyse_power.py` from the repo root; output must
include 5 lines (one per meter for `2026-01-01`) and exit 0.

## Stage 3 — docs

Write `examples/README.md`. Sections, in order:

- **What this is** — two sentences.
- **Schema** — copy the column table from SCOPE.md.
- **Sample queries** — 2-3 example queries phrased as
  pseudo-SQL or Python one-liners (e.g. "peak power per meter",
  "total kWh on 2026-01-01"). They do not need to be runnable;
  they document intent.
- **Running the analyser** — the one-line
  `python3 examples/analyse_power.py` invocation plus a sample of
  its first 2 output lines.

Commit as `bacnet stage 3: add examples README`. No runtime
verification beyond the file existing and being non-empty.

## What counts as done

All three stages committed cleanly on the job branch. No edits
outside `examples/`. The analyser runs to completion against the
generated CSV with exit code 0.

## What to avoid

- Do not introduce a build system, a `requirements.txt`, or a
  `pyproject.toml`. The example is a single script + a CSV + a
  README, by design.
- Do not generate the CSV from inside the Python script at runtime.
  Stage 1 produces the file; stage 2 only reads it.
- Do not push the branch. The user reviews the worktree before
  any push happens.

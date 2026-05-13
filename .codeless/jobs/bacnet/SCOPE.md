# Scope — bacnet power-meter example

A small, self-contained worked example of power-meter data in CSV
form, with a Python analyser and docs explaining the schema. Lives
entirely under `examples/` in the source repo. Intended as a
reference dataset other jobs and humans can use without needing a
real BACnet stack on the network.

## What success looks like

After all stages run cleanly, the repo contains:

- `examples/power-meter.csv` — 5 meters x 24 hours at 5-minute
  intervals = 1,440 data rows + 1 header row. Each row is one
  reading from one meter at one timestamp.
- `examples/analyse_power.py` — a single-file Python 3 script,
  stdlib only, that reads `power-meter.csv` and prints per-meter
  daily kWh totals and peak power. Runnable as
  `python3 examples/analyse_power.py`.
- `examples/README.md` — explains the schema, the units, a couple
  of sample SQL-style queries against the CSV, and how to run the
  analyser.

Each stage commits its own files to the job's branch with a
descriptive message. No file outside `examples/` changes.

## Schema (canonical for the CSV)

| column         | type       | unit     | notes                                  |
|----------------|------------|----------|----------------------------------------|
| `timestamp`    | string     | —        | ISO-8601 UTC, second precision (`Z`).  |
| `meter_id`     | string     | —        | `PM-001` .. `PM-005`. Stable per row.  |
| `voltage_v`    | number     | volts    | Line-to-neutral RMS.                   |
| `current_a`    | number     | amps     | RMS, per phase aggregated.             |
| `power_w`      | number     | watts    | Active power.                          |
| `energy_kwh`   | number     | kWh      | Monotonic cumulative since meter zero. |
| `power_factor` | number     | unitless | 0.0 .. 1.0, leading or lagging.        |

Values may include realistic small noise; they need not represent a
specific physical load. Two of the five meters should show a
visible daily duty cycle so the analyser's "peak power" output is
interesting.

## Out of scope

- Real BACnet protocol parsing — this job is about the *example
  data shape* and the analysis surface, not the wire protocol.
- A web UI, dashboard, or charting. Stdout is the only output of
  the analyser.
- Non-stdlib Python dependencies. No `pandas`, no `numpy`. The
  example must run on a clean Python 3.10+ install.
- Any change outside `examples/`. The repo's existing code is not
  touched by this job.

## Constraints

- File encoding: UTF-8.
- Line endings: LF.
- CSV: header row present, comma-separated, no quoting needed for
  the values above (numeric and short strings only).
- Python: 3.10+ syntax features OK, stdlib only.
- All work happens in the job's worktree branch; never on `master`.

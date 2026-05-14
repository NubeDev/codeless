# Scope — simple-python-file

## Goal

Create a minimal Python script and verify it executes without error.

## Deliverables

- `hello.py` — a simple Python script (e.g. prints "Hello, world!")
- A run result: either a success confirmation or a single failure report

## Constraints

- The script must be plain Python 3, no external dependencies.
- The run step attempts execution exactly once. If it fails, the agent reports the error and stops — no retry, no fix loop.

## Out of scope

- Installing packages or setting up a virtual environment.
- Multiple run attempts or automatic error correction.

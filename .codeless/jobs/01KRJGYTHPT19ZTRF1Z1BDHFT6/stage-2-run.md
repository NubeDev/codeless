# Stage 2 — Run the Python file

## Task

Execute `hello.py` with `python3` and report the outcome.

## Procedure

1. Run: `python3 hello.py`
2. If exit code is 0 — report success and mark stage done.
3. If exit code is non-zero — capture stdout and stderr, report the full error to the user, and halt. Do not edit the file or retry.

## Acceptance criteria

- Exactly one execution attempt is made.
- The outcome (success output or failure details) is reported to the user.
- On failure, no further action is taken.

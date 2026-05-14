# Workflow — simple-python-file

## Stage order

1. **stage-1** — Write `hello.py`
2. **stage-2** — Run `hello.py` (one attempt only)

## Success criteria

- Stage 1 is complete when `hello.py` exists and is valid Python 3 syntax.
- Stage 2 is complete when the script runs without error, **or** when one failed attempt has been reported to the user.

## Failure handling

If `python3 hello.py` exits non-zero in stage 2, the agent must:

1. Capture stdout and stderr.
2. Report the error to the user.
3. Mark the stage as failed and halt — do not retry or edit the file.

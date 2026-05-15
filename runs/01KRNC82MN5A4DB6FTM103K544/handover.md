## Done

- verified stage 3 (MVP slice 2 job-submit persona dropdown) is already implemented and committed as bd65341 on branch codeless/agent-personas
- confirmed SubmitJobDialog.tsx filters personas by `useForJobs`, seeds Model from `defaultModel`, and sends persona `instructions` as `system_prompt` in SubmitJobArgs
- confirmed migration 0009_job_system_prompt.sql, SubmitJobArgs/jobs row plumbing, and DefaultRunnerFactory prompt composition all landed in the same commit
- no new commit created — stage was already shipped before this session ran

## Next

- Stage 4 of 11 in the agent-personas job; a fresh session should pick it up
- job tracker may be ahead of branch by ≥2 stages (slices 1 and 2 both shipped before their nominal stages fired), so the loop driver should be reconciled before stage 4 fires to avoid another no-op

## What you need to know

- branch `codeless/agent-personas` already contains slice 1 (0f859fd) and slice 2 (bd65341); both stages 2 and 3 in the tracker map to slice 2's commit message
- working tree is clean apart from the untracked `runs/01KRNC82MN5A4DB6FTM103K544/` scaffold
- the slice keeps snake_case at the wire boundary (`system_prompt`, SubmitJobArgs) and camelCase in the UI (`useForJobs`, `defaultModel`); the persona record is still KV-store only (SQLite move comes later in the plan)

## Open questions

- should the loop driver be advanced past stages 2 and 3, or is each verify-pass intentional? Two consecutive sessions have now found the work already merged

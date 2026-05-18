# Plan engine P1 — Stage 1 survey findings

Stage-1 survey for the Plan engine (P1) per `DOCS/JOB-WORKFLOW.md`
"Job chaining" (lines 431–600). Subsequent stages should not re-derive
the facts here.

## P1 scope, verbatim from DOCS/JOB-WORKFLOW.md §"(P1)"

- `codeless-tools/src/plan/` mirrors `email/` and `schedule/`:
  pure data (`PlanSpec`, `PlanStep`, `Transition`) in one module;
  in-memory `PlanEngine` with an injected `JobSpawner` trait in
  another. No SQLite. Engine = event bus + spawn callback.
- Tools: `codeless.plan.create` / `.start` / `.list` / `.cancel`.
- Wire engine into `codeless-runtime` as an event-bus consumer.
  In-memory; restart loses in-flight PlanRuns (documented limit).
- Scheduled Plan = `Schedule` fires → `Action` calls
  `PlanEngine::start_run(plan_id)`. This composition is the boundary
  proof.

Linear chain transition vocabulary (no DAG, no `when:` predicates):

```yaml
steps:
  - id: <step-id>
    job_template: <template>
    on_success: <step-id> | stop | omitted (= stop)
    on_failure: <step-id> | stop | omitted (= stop)
```

Out of scope: persistence, RPC surface, DAG, UI (those are P2/P3),
cross-repo plans, conditional re-runs of the same step.

## Mirror layout — `codeless-tools/src/schedule/`

Crate `codeless-tools` (host-only per R1). The plan module should
mirror this exact shape:

```
schedule/
├── mod.rs        # 27 lines — module doc + re-exports
├── spec.rs       # pure data: Schedule enum, Weekday, TimeOfDay, ScheduleTz, next_fire_after
├── scheduler.rs  # Scheduler {entries, action}, Action trait, ScheduleId,
│                 # SchedulerError; in-memory HashMap<Id, Entry> + tokio task per entry
└── dispatch.rs   # PayloadDispatcher (kind-keyed Action router) + LogAction
```

`Action` trait shape (reuse pattern for `JobSpawner`):

```rust
#[async_trait]
pub trait Action: Send + Sync + 'static {
    async fn fire(&self, id: &ScheduleId, payload: &Value);
}
```

`ActionFn = Arc<dyn Fn(ScheduleId, Value) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static>`
also implements `Action` for ergonomic closure callers.

Email mirror (`codeless-tools/src/email/`): same idiom — `message.rs`
is pure data (`Message`, `Mailbox`), `mailer.rs` is the transport
trait (`Mailer`, `SendOutcome`), `gmail.rs` is one impl. Re-exports
flat from `mod.rs`.

## Tool registration

`crates/codeless-tools/src/tools/schedule_create.rs` is the template
for `codeless.plan.*` tools. Pattern:

- `pub struct FooTool { schema: Value, scheduler: Arc<Scheduler> }`
  holding an `Arc` of the injected engine.
- `impl Tool` with `name()` returning the dotted id
  (`"codeless.schedule.create"`), `schema()`, and `call(ctx, args)`.
- A single tool with an `action: "create"|"list"|"cancel"` enum
  parameter is the existing convention — favour that over four
  separate tool types unless the schemas diverge a lot.
- Registered via `pub use` in `crates/codeless-tools/src/tools/mod.rs`;
  host wiring (codeless-mcp / runtime) builds the `Arc<Scheduler>`
  and constructs the tool. The library does not own the registration
  decision.

## Runtime event bus — terminal job events

`crates/codeless-runtime/src/event_bus.rs`:

- `EventBus { pool: SqlitePool, sender: broadcast::Sender<EventEnvelope> }`.
- `EventBus::new(pool, capacity)` and `Arc<EventBus>` is the shared
  handle. RPC layer exposes it via `RpcServer::bus()`.
- `publish(...)` persists then broadcasts. Two-stage = catch-up safe.
- `subscribe_since(filter, since: Option<EventCursor>) -> EventStream`
  returns a boxed `Stream<Item = Result<EventEnvelope, RpcError>>`.
  Filter is `SubscribeFilter::All | Job(JobId)`.

**Terminal job event variants live in `codeless-types::Event`
(`crates/codeless-types/src/event.rs`):**

- `Event::JobCompleted { job_id }` — wire label `"job-completed"`.
  Doc text in JOB-WORKFLOW says "JobFinished", but the actual variant
  is `JobCompleted`. Use the real name.
- `Event::JobFailed { job_id }` — `"job-failed"`.
- `Event::JobStopped { job_id, reason: StopReason }` — `"job-stopped"`.
- `Event::JobPaused { job_id, reason }` — NOT terminal for our
  purposes; a paused job is expected to be resumed (`job-resumed`).
  Treat pause/resume as non-terminal; only the three above advance
  a PlanRun.

`EventEnvelope { cursor, job_id: Option<JobId>, stage_id, task_id,
created_at, event }` — `job_id` is the field the engine joins on.

## EventSource pattern — `codeless-bot-core::outbound`

`crates/codeless-bot-core/src/outbound.rs`. This is the reference
the spec names ("Plan engine is a second consumer, no new bus") and
it is the closest precedent for what we need.

Minimal upstream seam — copy this shape verbatim for the plan engine:

```rust
#[async_trait]
pub trait EventSource: Send + Sync + 'static {
    async fn subscribe_all(&self) -> RpcResult<EventStream>;
}

pub struct RpcServerEventSource { inner: Arc<dyn RpcServer> }
// impl EventSource by forwarding subscribe(EventFilter::All, None).
```

Loop skeleton: `OutboundPublisher::spawn(...) -> Self` returns a
struct holding `JoinHandle<()>` + a `oneshot::Sender<()>` shutdown
signal; `run_loop` selects between `shutdown` and `stream.next()`,
matches `event` against the variants it cares about, and dispatches.
A subscription open failure logs a `tracing::warn!` and returns
(the rest of the host keeps serving). Each envelope error is logged
and skipped; `None` from the stream is a clean exit.

Outbound also shows two patterns the plan engine should adopt:

- Replay policy: subscribe at `since: None` (live only). The Plan
  engine spec says in-memory + lossy on restart, so live-only is
  correct.
- Per-id state map (Outbound's `Debouncer<JobId, Instant>`): the
  plan engine needs an analogous `HashMap<JobId, PlanRunId>` so
  each terminal envelope can resolve back to the PlanRun it
  belongs to.

## How a fired Schedule becomes `PlanEngine::start_run`

`PayloadDispatcher` in `schedule/dispatch.rs` is the join point. The
host registers a plan-keyed `Action` whose payload carries
`{"kind": "start_plan", "plan_id": "..."}`. Implementation:

```rust
struct StartPlanAction { engine: Arc<PlanEngine> }

#[async_trait]
impl Action for StartPlanAction {
    async fn fire(&self, _id: &ScheduleId, payload: &Value) {
        let plan_id = payload["plan_id"].as_str()...;
        let _ = self.engine.start_run(plan_id).await;
    }
}
```

Wiring lives in the host (codeless-runtime or codeless-mcp), not in
the library, exactly as `LogAction` shows for the schedule crate.

## R1 dependency edges

`codeless-tools` is already host-only by virtue of its existing
schedule/email/browser deps. Adding `plan/` does not change the
matrix. But: the `JobSpawner` trait must NOT pull in
`codeless-runtime` types in `codeless-tools`. Use a minimal
`JobTemplate` / `SpawnedJobId` shape defined inside the plan module
(or re-use `codeless-types::JobId`, which is mobile-safe). The host
adapter implementing `JobSpawner` lives outside `codeless-tools`.

`codeless-tools` already depends on `async-trait`, `tokio`,
`serde_json`, `chrono`, `tokio-util`, `thiserror`, `tracing` —
everything the plan module needs.

## Open follow-ups for Stage 2+

- Stage 2 will define `PlanSpec`/`PlanStep`/`Transition` data in
  `plan/spec.rs`. Decide whether `Transition` is an enum
  (`Step(StepId) | Stop`) or `Option<StepId>` with `None` = stop —
  the spec wording "omitted = stop" leans toward `Option`.
- Stage 3+ will need a `JobSpawner` trait whose method takes the
  step's `job_template` string plus any per-step inputs and returns
  a `JobId` plus a future that resolves on terminal event. Two ways
  to wait: (a) `JobSpawner` returns just `JobId` and the engine
  itself owns the bus subscription that matches `job_id`; or (b)
  `JobSpawner` returns a oneshot. Outbound's pattern argues for (a):
  one subscription, dispatch by `job_id`.
- `EventSource::subscribe_all` already exists in `codeless-bot-core`
  but the plan engine will live in `codeless-tools`, which cannot
  depend on `codeless-bot-core`. Re-declare the same trait shape
  inside the plan module (it is three lines) rather than reaching
  across crates.

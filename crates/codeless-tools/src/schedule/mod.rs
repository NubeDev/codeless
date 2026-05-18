//! Reusable scheduler: one-shot timers and weekly recurring schedules.
//!
//! Two layers, split because they are useful independently:
//!
//!   - `spec` is pure data: a `Schedule` describes when something
//!     should fire (one-shot at a point in time, or a weekly grid of
//!     day-of-week × time-of-day). `next_fire_after` is the only
//!     thing it computes. No tokio, no I/O.
//!   - `scheduler` owns a tokio task per registered schedule that
//!     sleeps until the next fire instant, invokes an `Action`
//!     handler, and reschedules. Storage is in-memory; persistence
//!     is the host's job (e.g. mirror inserts into SQLite and
//!     re-hydrate on restart).
//!
//! The seven-day cron example "every Mon, Wed at 8am, 11am and 5pm"
//! is `Schedule::Weekly { days: [Mon, Wed], times: [08:00, 11:00,
//! 17:00], tz: Local }` — a structured form rather than a cron
//! string, because the LLM tool surface is easier to validate when
//! the schema enumerates the fields.

pub mod scheduler;
pub mod spec;

pub use scheduler::{Action, ActionFn, ScheduleId, Scheduler};
pub use spec::{Schedule, ScheduleTz, TimeOfDay, Weekday};

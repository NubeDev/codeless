// Schedule create/list/cancel tool. Wraps an injected
// `crate::schedule::Scheduler`. The host wires the scheduler with
// the action it wants fired (post to a thread, enqueue a job, etc.);
// this tool only mutates the schedule registry.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::DateTime;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::schedule::{Schedule, ScheduleId, ScheduleTz, Scheduler, TimeOfDay, Weekday};
use crate::tool::Tool;

pub struct ScheduleCreateTool {
    schema: Value,
    scheduler: Arc<Scheduler>,
}

impl ScheduleCreateTool {
    pub fn new(scheduler: Arc<Scheduler>) -> Self {
        Self {
            scheduler,
            schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["create", "list", "cancel"],
                        "description": "Operation to perform."
                    },
                    "id": {
                        "type": "string",
                        "description": "Schedule identifier. Required for create and cancel. Re-using an existing id on create replaces the previous schedule."
                    },
                    "payload": {
                        "description": "Arbitrary JSON value passed to the host-registered action when the schedule fires."
                    },
                    "schedule": {
                        "type": "object",
                        "description": "When to fire. Either a one-shot or a weekly grid.",
                        "properties": {
                            "kind": { "type": "string", "enum": ["one_shot", "weekly"] },
                            "at": {
                                "type": "string",
                                "description": "RFC 3339 instant. Required for kind=one_shot."
                            },
                            "days": {
                                "type": "array",
                                "items": {
                                    "type": "string",
                                    "enum": ["mon","tue","wed","thu","fri","sat","sun"]
                                },
                                "description": "Days of week the schedule fires on. Required for kind=weekly."
                            },
                            "times": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "hour":   { "type": "integer", "minimum": 0, "maximum": 23 },
                                        "minute": { "type": "integer", "minimum": 0, "maximum": 59 }
                                    },
                                    "required": ["hour", "minute"]
                                },
                                "description": "Times of day the schedule fires at. Required for kind=weekly."
                            },
                            "tz": {
                                "type": "string",
                                "enum": ["local", "utc"],
                                "description": "Timezone the weekly grid is interpreted in. Defaults to local."
                            }
                        },
                        "required": ["kind"]
                    }
                },
                "required": ["action"]
            }),
        }
    }
}

#[async_trait]
impl Tool for ScheduleCreateTool {
    fn name(&self) -> &str {
        "codeless.schedule.create"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, _ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'action'"))?;

        match action {
            "create" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolError::invalid_args("create requires 'id'"))?
                    .to_string();
                let schedule_v = args
                    .get("schedule")
                    .ok_or_else(|| ToolError::invalid_args("create requires 'schedule'"))?;
                let schedule = parse_schedule(schedule_v)?;
                let payload = args.get("payload").cloned().unwrap_or(Value::Null);

                self.scheduler
                    .create(ScheduleId::new(id.clone()), schedule.clone(), payload)
                    .await
                    .map_err(|e| ToolError::invalid_args(format!("scheduler: {e}")))?;

                let next = schedule.next_fire_after(chrono::Utc::now());
                Ok(json!({
                    "created": true,
                    "id": id,
                    "next_fire": next.map(|d| d.to_rfc3339()),
                }))
            }
            "cancel" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolError::invalid_args("cancel requires 'id'"))?;
                let removed = self.scheduler.cancel(&ScheduleId::new(id)).await;
                Ok(json!({ "cancelled": removed, "id": id }))
            }
            "list" => {
                let now = chrono::Utc::now();
                let items: Vec<Value> = self
                    .scheduler
                    .list()
                    .await
                    .into_iter()
                    .map(|(id, sched)| {
                        json!({
                            "id": id.0,
                            "schedule": sched,
                            "next_fire": sched.next_fire_after(now).map(|d| d.to_rfc3339()),
                        })
                    })
                    .collect();
                let count = items.len();
                Ok(json!({ "schedules": items, "count": count }))
            }
            other => Err(ToolError::invalid_args(format!("unknown action '{other}'"))),
        }
    }
}

fn parse_schedule(value: &Value) -> Result<Schedule, ToolError> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::invalid_args("schedule missing 'kind'"))?;
    match kind {
        "one_shot" => {
            let at_str = value
                .get("at")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::invalid_args("one_shot schedule requires 'at'"))?;
            let at = DateTime::parse_from_rfc3339(at_str)
                .map_err(|e| ToolError::invalid_args(format!("bad 'at' timestamp: {e}")))?
                .with_timezone(&chrono::Utc);
            Ok(Schedule::OneShot { at })
        }
        "weekly" => {
            let days = parse_days(value.get("days"))?;
            let times = parse_times(value.get("times"))?;
            let tz = match value.get("tz").and_then(Value::as_str) {
                None | Some("local") => ScheduleTz::Local,
                Some("utc") => ScheduleTz::Utc,
                Some(other) => {
                    return Err(ToolError::invalid_args(format!("unknown tz '{other}'")));
                }
            };
            Ok(Schedule::Weekly { days, times, tz })
        }
        other => Err(ToolError::invalid_args(format!(
            "unknown schedule kind '{other}'"
        ))),
    }
}

fn parse_days(value: Option<&Value>) -> Result<Vec<Weekday>, ToolError> {
    let arr = value
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::invalid_args("weekly schedule requires 'days' array"))?;
    arr.iter()
        .map(|v| {
            let s = v
                .as_str()
                .ok_or_else(|| ToolError::invalid_args("'days' entries must be strings"))?;
            Weekday::parse(s).ok_or_else(|| ToolError::invalid_args(format!("unknown day '{s}'")))
        })
        .collect()
}

fn parse_times(value: Option<&Value>) -> Result<Vec<TimeOfDay>, ToolError> {
    let arr = value
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::invalid_args("weekly schedule requires 'times' array"))?;
    arr.iter()
        .map(|v| {
            let h = v
                .get("hour")
                .and_then(Value::as_u64)
                .ok_or_else(|| ToolError::invalid_args("'times' entry missing integer 'hour'"))?;
            let m = v
                .get("minute")
                .and_then(Value::as_u64)
                .ok_or_else(|| ToolError::invalid_args("'times' entry missing integer 'minute'"))?;
            TimeOfDay::new(h as u8, m as u8).map_err(|e| ToolError::invalid_args(format!("{e}")))
        })
        .collect()
}

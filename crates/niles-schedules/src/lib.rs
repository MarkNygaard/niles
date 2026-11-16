//! niles-schedules — each person's own agent tasks: a written
//! instruction the assistant answers on a schedule, as that person, in
//! their chat. Separate from automations, which are the household's
//! fixed rules with no model in them; the time they run at is matched by
//! the automations' own clock triggers. See ARCHITECTURE.md's
//! "Schedules".

mod error;
mod model;
mod store;

pub use error::{Error, Result};
pub use model::{
    MAX_INSTRUCTION, MAX_NAME, NewSchedule, Outcome, Run, Schedule, When, instruction, name,
};
pub use store::{MemorySchedules, RUNS_SHOWN, ScheduleStore};

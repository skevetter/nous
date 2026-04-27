use nous_core::cron_parser::CronExpr;
use nous_core::db::MemoryDb;
use nous_core::schedule_db::ScheduleDb;
use nous_core::types::{ActionType, Schedule};
use nous_shared::ids::MemoryId;

use crate::commands::{OutputFormat, print_json};
use crate::config::Config;

fn open_db(config: &Config) -> Result<MemoryDb, Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    Ok(MemoryDb::open(
        &config.memory.db_path,
        db_key.as_deref(),
        384,
    )?)
}

pub fn run_schedule_list(
    config: &Config,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let schedules = ScheduleDb::list(db.connection(), None, None, None)?;

    match format {
        OutputFormat::Json => print_json(&schedules)?,
        _ => {
            if schedules.is_empty() {
                println!("No schedules found.");
                return Ok(());
            }
            for s in &schedules {
                let status = if s.enabled { "enabled" } else { "paused" };
                println!(
                    "{id}  {name}  {cron}  {action}  {status}",
                    id = s.id,
                    name = s.name,
                    cron = s.cron_expr,
                    action = s.action_type,
                );
            }
        }
    }
    Ok(())
}

pub fn run_schedule_get(
    config: &Config,
    id: &str,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let schedule =
        ScheduleDb::get(db.connection(), id)?.ok_or_else(|| format!("schedule not found: {id}"))?;

    match format {
        OutputFormat::Json => print_json(&schedule)?,
        _ => {
            println!("ID:          {}", schedule.id);
            println!("Name:        {}", schedule.name);
            println!("Cron:        {}", schedule.cron_expr);
            println!("Timezone:    {}", schedule.timezone);
            println!("Enabled:     {}", schedule.enabled);
            println!("Action type: {}", schedule.action_type);
            println!("Payload:     {}", schedule.action_payload);
            if let Some(ref outcome) = schedule.desired_outcome {
                println!("Outcome:     {outcome}");
            }
            println!("Max retries: {}", schedule.max_retries);
            if let Some(t) = schedule.timeout_secs {
                println!("Timeout:     {t}s");
            }
        }
    }
    Ok(())
}

pub fn run_schedule_create(
    config: &Config,
    name: &str,
    cron_expr: &str,
    action_type: &str,
    action_payload: &str,
    timezone: Option<&str>,
    desired_outcome: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    CronExpr::parse(cron_expr)?;
    let action_type: ActionType = action_type
        .parse()
        .map_err(|e| format!("invalid action_type: {e}"))?;

    let db = open_db(config)?;
    let schedule = Schedule {
        id: MemoryId::new().to_string(),
        name: name.to_string(),
        cron_expr: cron_expr.to_string(),
        timezone: timezone.unwrap_or("UTC").to_string(),
        enabled: true,
        action_type,
        action_payload: action_payload.to_string(),
        desired_outcome: desired_outcome.map(String::from),
        max_retries: 3,
        timeout_secs: None,
        max_output_bytes: 65536,
        max_runs: 100,
        next_run_at: None,
        created_at: 0,
        updated_at: 0,
    };

    let id = ScheduleDb::create_on(db.connection(), &schedule)?;
    println!("{id}");
    Ok(())
}

pub fn run_schedule_delete(config: &Config, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let deleted = ScheduleDb::delete_on(db.connection(), id)?;
    if deleted {
        println!("deleted {id}");
    } else {
        eprintln!("schedule not found: {id}");
    }
    Ok(())
}

pub fn run_schedule_pause(config: &Config, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    db.connection().execute(
        "UPDATE schedules SET enabled = 0, updated_at = strftime('%s', 'now') WHERE id = ?1",
        rusqlite::params![id],
    )?;
    println!("paused {id}");
    Ok(())
}

pub fn run_schedule_resume(config: &Config, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    db.connection().execute(
        "UPDATE schedules SET enabled = 1, updated_at = strftime('%s', 'now') WHERE id = ?1",
        rusqlite::params![id],
    )?;
    println!("resumed {id}");
    Ok(())
}

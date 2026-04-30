use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::schedules::{self, SystemClock};

#[derive(Subcommand)]
pub enum ScheduleCommands {
    /// Create a new schedule
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        cron: String,
        #[arg(long)]
        action: String,
        #[arg(long)]
        payload: String,
        #[arg(long)]
        tz: Option<String>,
        #[arg(long)]
        timeout: Option<i32>,
        #[arg(long)]
        max_retries: Option<i32>,
        #[arg(long)]
        max_runs: Option<i32>,
        #[arg(long)]
        desired_outcome: Option<String>,
        /// One-shot: fire once at this Unix timestamp, then disable
        #[arg(long)]
        trigger_at: Option<i64>,
    },
    /// List schedules
    List {
        #[arg(long)]
        enabled: Option<bool>,
        #[arg(long)]
        action: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Get schedule details
    Get {
        /// Schedule ID
        id: String,
    },
    /// Update a schedule
    Update {
        /// Schedule ID
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        cron: Option<String>,
        /// Set one-shot trigger timestamp
        #[arg(long)]
        trigger_at: Option<i64>,
        /// Clear the one-shot trigger
        #[arg(long)]
        clear_trigger: bool,
        #[arg(long)]
        enabled: Option<bool>,
        #[arg(long)]
        action: Option<String>,
        #[arg(long)]
        payload: Option<String>,
        #[arg(long)]
        desired_outcome: Option<String>,
        /// Clear the desired outcome
        #[arg(long)]
        clear_desired_outcome: bool,
        #[arg(long)]
        max_retries: Option<i32>,
        #[arg(long)]
        timeout: Option<i32>,
    },
    /// Delete a schedule
    Delete {
        /// Schedule ID
        id: String,
    },
    /// List runs for a schedule
    Runs {
        /// Schedule ID
        id: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Show schedule health overview
    Health,
}

pub async fn run(cmd: ScheduleCommands) {
    if let Err(e) = execute(cmd).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(cmd: ScheduleCommands) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;
    config.ensure_dirs()?;
    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations().await?;
    let pool = &pools.fts;
    let clock = SystemClock;

    match cmd {
        ScheduleCommands::Create {
            name,
            cron,
            action,
            payload,
            tz,
            timeout,
            max_retries,
            max_runs,
            desired_outcome,
            trigger_at,
        } => {
            let schedule = schedules::create_schedule(
                pool,
                &name,
                &cron,
                trigger_at,
                tz.as_deref(),
                &action,
                &payload,
                desired_outcome.as_deref(),
                max_retries,
                timeout,
                None,
                max_runs,
                &clock,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&schedule)?);
        }
        ScheduleCommands::List {
            enabled,
            action,
            limit,
        } => {
            let list =
                schedules::list_schedules(pool, enabled, action.as_deref(), Some(limit)).await?;
            println!("{}", serde_json::to_string_pretty(&list)?);
        }
        ScheduleCommands::Get { id } => {
            let schedule = schedules::get_schedule(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&schedule)?);
        }
        ScheduleCommands::Update {
            id,
            name,
            cron,
            trigger_at,
            clear_trigger,
            enabled,
            action,
            payload,
            desired_outcome,
            clear_desired_outcome,
            max_retries,
            timeout,
        } => {
            let timeout_opt = timeout.map(Some);
            let trigger_at_opt = if clear_trigger {
                Some(None)
            } else {
                trigger_at.map(Some)
            };
            let desired_outcome_opt = if clear_desired_outcome {
                Some(None)
            } else {
                desired_outcome.as_deref().map(Some)
            };
            let schedule = schedules::update_schedule(
                pool,
                &id,
                name.as_deref(),
                cron.as_deref(),
                trigger_at_opt,
                enabled,
                action.as_deref(),
                payload.as_deref(),
                desired_outcome_opt,
                max_retries,
                timeout_opt,
                None,
                &clock,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&schedule)?);
        }
        ScheduleCommands::Delete { id } => {
            schedules::delete_schedule(pool, &id).await?;
            println!("{{\"deleted\": true}}");
        }
        ScheduleCommands::Runs { id, status, limit } => {
            let runs = schedules::list_runs(pool, &id, status.as_deref(), Some(limit)).await?;
            println!("{}", serde_json::to_string_pretty(&runs)?);
        }
        ScheduleCommands::Health => {
            let health = schedules::schedule_health(pool).await?;
            println!("{}", serde_json::to_string_pretty(&health)?);
        }
    }

    pools.close().await;
    Ok(())
}

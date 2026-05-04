use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::schedules::{self, SystemClock};

use super::output::{OutputFormat, parse_fields, print_list};

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
        /// Output format: json (default), table, csv
        #[arg(short, long, default_value = "json")]
        output: OutputFormat,
        /// Comma-separated fields to include (e.g. id,name,enabled)
        #[arg(long)]
        fields: Option<String>,
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

pub async fn run(cmd: ScheduleCommands, port: Option<u16>) {
    if let Err(e) = execute(cmd, port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(
    cmd: ScheduleCommands,
    port: Option<u16>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    if let Some(p) = port {
        config.port = p;
    }
    config.ensure_dirs()?;
    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations().await?;
    let pool = &pools.fts;
    let clock = SystemClock;

    dispatch(pool, &clock, cmd).await?;

    pools.close().await;
    Ok(())
}

async fn dispatch(
    pool: &sea_orm::DatabaseConnection,
    clock: &SystemClock,
    cmd: ScheduleCommands,
) -> Result<(), Box<dyn std::error::Error>> {
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
            cmd_create(
                pool,
                clock,
                CreateArgs {
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
                },
            )
            .await?;
        }
        ScheduleCommands::List { enabled, action, limit, output, fields } => {
            cmd_list(pool, ListArgs { enabled, action, limit, output, fields }).await?;
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
            cmd_update(
                pool,
                clock,
                UpdateArgs {
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
                },
            )
            .await?;
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
    Ok(())
}

struct CreateArgs {
    name: String,
    cron: String,
    action: String,
    payload: String,
    tz: Option<String>,
    timeout: Option<i32>,
    max_retries: Option<i32>,
    max_runs: Option<i32>,
    desired_outcome: Option<String>,
    trigger_at: Option<i64>,
}

async fn cmd_create(
    pool: &sea_orm::DatabaseConnection,
    clock: &SystemClock,
    args: CreateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let schedule = schedules::create_schedule(schedules::CreateScheduleParams {
        db: pool,
        name: &args.name,
        cron_expr: &args.cron,
        trigger_at: args.trigger_at,
        timezone: args.tz.as_deref(),
        action_type: &args.action,
        action_payload: &args.payload,
        desired_outcome: args.desired_outcome.as_deref(),
        max_retries: args.max_retries,
        timeout_secs: args.timeout,
        max_output_bytes: None,
        max_runs: args.max_runs,
        clock,
    })
    .await?;
    println!("{}", serde_json::to_string_pretty(&schedule)?);
    Ok(())
}

struct ListArgs {
    enabled: Option<bool>,
    action: Option<String>,
    limit: u32,
    output: OutputFormat,
    fields: Option<String>,
}

async fn cmd_list(
    pool: &sea_orm::DatabaseConnection,
    args: ListArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let list =
        schedules::list_schedules(pool, args.enabled, args.action.as_deref(), Some(args.limit))
            .await?;
    let val = serde_json::to_value(&list)?;
    let fields_override = args.fields.as_deref().map(parse_fields);
    print_list(
        &val,
        &args.output,
        &["id", "name", "cron_expr", "action_type", "enabled", "next_run_at"],
        fields_override.as_deref(),
    );
    Ok(())
}

struct UpdateArgs {
    id: String,
    name: Option<String>,
    cron: Option<String>,
    trigger_at: Option<i64>,
    clear_trigger: bool,
    enabled: Option<bool>,
    action: Option<String>,
    payload: Option<String>,
    desired_outcome: Option<String>,
    clear_desired_outcome: bool,
    max_retries: Option<i32>,
    timeout: Option<i32>,
}

async fn cmd_update(
    pool: &sea_orm::DatabaseConnection,
    clock: &SystemClock,
    args: UpdateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let timeout_opt = args.timeout.map(Some);
    let trigger_at_opt = if args.clear_trigger {
        Some(None)
    } else {
        args.trigger_at.map(Some)
    };
    let desired_outcome_opt = if args.clear_desired_outcome {
        Some(None)
    } else {
        args.desired_outcome.as_deref().map(Some)
    };
    let schedule = schedules::update_schedule(schedules::UpdateScheduleParams {
        db: pool,
        id: &args.id,
        name: args.name.as_deref(),
        cron_expr: args.cron.as_deref(),
        trigger_at: trigger_at_opt,
        enabled: args.enabled,
        action_type: args.action.as_deref(),
        action_payload: args.payload.as_deref(),
        desired_outcome: desired_outcome_opt,
        max_retries: args.max_retries,
        timeout_secs: timeout_opt,
        max_runs: None,
        clock,
    })
    .await?;
    println!("{}", serde_json::to_string_pretty(&schedule)?);
    Ok(())
}

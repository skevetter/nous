use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "schedules")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub cron_expr: String,
    pub trigger_at: Option<i64>,
    pub timezone: String,
    pub enabled: bool,
    pub action_type: String,
    pub action_payload: String,
    pub desired_outcome: Option<String>,
    pub max_retries: i32,
    pub timeout_secs: Option<i32>,
    pub max_output_bytes: i32,
    pub max_runs: i32,
    pub last_run_at: Option<i64>,
    pub next_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::schedule_runs::Entity")]
    Runs,
}

impl Related<super::schedule_runs::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Runs.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

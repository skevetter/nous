use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_processes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub agent_id: String,
    pub process_type: String,
    pub command: String,
    pub working_dir: Option<String>,
    pub env_json: Option<String>,
    pub pid: Option<i32>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub started_at: Option<String>,
    pub stopped_at: Option<String>,
    pub last_output: Option<String>,
    pub max_output_bytes: i32,
    pub timeout_secs: Option<i32>,
    pub sandbox_image: Option<String>,
    pub sandbox_cpus: Option<i32>,
    pub sandbox_memory_mib: Option<i32>,
    pub sandbox_network_policy: Option<String>,
    pub sandbox_volumes_json: Option<String>,
    pub sandbox_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::agents::Entity",
        from = "Column::AgentId",
        to = "super::agents::Column::Id"
    )]
    Agent,
}

impl Related<super::agents::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Agent.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_invocations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub agent_id: String,
    pub process_id: Option<String>,
    pub prompt: String,
    pub result: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub duration_ms: Option<i32>,
    pub metadata_json: Option<String>,
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::agents::Entity",
        from = "Column::AgentId",
        to = "super::agents::Column::Id"
    )]
    Agent,
    #[sea_orm(
        belongs_to = "super::agent_processes::Entity",
        from = "Column::ProcessId",
        to = "super::agent_processes::Column::Id"
    )]
    Process,
}

impl Related<super::agents::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Agent.def()
    }
}

impl Related<super::agent_processes::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Process.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

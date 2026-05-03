use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub parent_agent_id: Option<String>,
    pub namespace: String,
    pub status: String,
    pub room: Option<String>,
    pub last_seen_at: Option<String>,
    pub metadata_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub current_version_id: Option<String>,
    pub upgrade_available: bool,
    pub template_id: Option<String>,
    pub process_type: Option<String>,
    pub spawn_command: Option<String>,
    pub working_dir: Option<String>,
    pub auto_restart: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::agent_processes::Entity")]
    Processes,
    #[sea_orm(has_many = "super::agent_invocations::Entity")]
    Invocations,
    #[sea_orm(has_many = "super::agent_versions::Entity")]
    Versions,
}

impl Related<super::agent_processes::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Processes.def()
    }
}

impl Related<super::agent_invocations::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invocations.def()
    }
}

impl Related<super::agent_versions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Versions.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

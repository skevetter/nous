use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "memories")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub workspace_id: String,
    pub agent_id: Option<String>,
    pub title: String,
    pub content: String,
    pub memory_type: String,
    pub importance: String,
    pub topic_key: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
    pub embedding: Option<Vec<u8>>,
    pub session_id: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::memory_access_log::Entity")]
    AccessLog,
    #[sea_orm(
        belongs_to = "super::memory_sessions::Entity",
        from = "Column::SessionId",
        to = "super::memory_sessions::Column::Id"
    )]
    Session,
}

impl Related<super::memory_access_log::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::AccessLog.def()
    }
}

impl Related<super::memory_sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

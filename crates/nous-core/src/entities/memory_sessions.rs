use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "memory_sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub agent_id: Option<String>,
    pub project: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub summary: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::memories::Entity")]
    Memories,
}

impl Related<super::memories::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Memories.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

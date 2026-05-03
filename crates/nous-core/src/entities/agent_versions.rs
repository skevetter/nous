use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_versions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub agent_id: String,
    pub skill_hash: String,
    pub config_hash: String,
    pub skills_json: String,
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
}

impl Related<super::agents::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Agent.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

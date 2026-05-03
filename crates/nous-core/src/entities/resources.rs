use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "resources")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub resource_type: String,
    pub owner_agent_id: Option<String>,
    pub namespace: String,
    pub path: Option<String>,
    pub status: String,
    pub metadata: Option<String>,
    pub tags: String,
    pub ownership_policy: String,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::agents::Entity",
        from = "Column::OwnerAgentId",
        to = "super::agents::Column::Id"
    )]
    OwnerAgent,
}

impl Related<super::agents::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::OwnerAgent.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

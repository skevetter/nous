use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_workspace_access")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub agent_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub workspace_id: String,
    pub granted_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

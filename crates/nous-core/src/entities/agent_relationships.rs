use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_relationships")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub parent_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub child_id: String,
    pub relationship_type: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub namespace: String,
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::agents::Entity",
        from = "Column::ParentId",
        to = "super::agents::Column::Id"
    )]
    ParentAgent,
    #[sea_orm(
        belongs_to = "super::agents::Entity",
        from = "Column::ChildId",
        to = "super::agents::Column::Id"
    )]
    ChildAgent,
}

impl ActiveModelBehavior for ActiveModel {}

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "task_links")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub link_type: String,
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::tasks::Entity",
        from = "Column::SourceId",
        to = "super::tasks::Column::Id"
    )]
    SourceTask,
    #[sea_orm(
        belongs_to = "super::tasks::Entity",
        from = "Column::TargetId",
        to = "super::tasks::Column::Id"
    )]
    TargetTask,
}

impl ActiveModelBehavior for ActiveModel {}

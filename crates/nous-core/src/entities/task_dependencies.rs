use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "task_dependencies")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub task_id: String,
    pub depends_on_task_id: String,
    pub dep_type: String,
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::tasks::Entity",
        from = "Column::TaskId",
        to = "super::tasks::Column::Id"
    )]
    Task,
    #[sea_orm(
        belongs_to = "super::tasks::Entity",
        from = "Column::DependsOnTaskId",
        to = "super::tasks::Column::Id"
    )]
    DependsOnTask,
}

impl ActiveModelBehavior for ActiveModel {}

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "memory_relations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::memories::Entity",
        from = "Column::SourceId",
        to = "super::memories::Column::Id"
    )]
    SourceMemory,
    #[sea_orm(
        belongs_to = "super::memories::Entity",
        from = "Column::TargetId",
        to = "super::memories::Column::Id"
    )]
    TargetMemory,
}

impl ActiveModelBehavior for ActiveModel {}

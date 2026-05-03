use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "memory_access_log")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub memory_id: String,
    pub access_type: String,
    pub session_id: Option<String>,
    pub accessed_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::memories::Entity",
        from = "Column::MemoryId",
        to = "super::memories::Column::Id"
    )]
    Memory,
}

impl Related<super::memories::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Memory.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

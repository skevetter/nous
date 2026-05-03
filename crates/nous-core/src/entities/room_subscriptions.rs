use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "room_subscriptions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub room_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub agent_id: String,
    pub topics: Option<String>,
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::rooms::Entity",
        from = "Column::RoomId",
        to = "super::rooms::Column::Id"
    )]
    Room,
}

impl Related<super::rooms::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Room.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

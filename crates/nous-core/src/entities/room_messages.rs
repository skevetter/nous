use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "room_messages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub room_id: String,
    pub sender_id: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub metadata: Option<String>,
    pub message_type: String,
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

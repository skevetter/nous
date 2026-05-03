use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "rooms")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub purpose: Option<String>,
    pub metadata: Option<String>,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::room_messages::Entity")]
    RoomMessages,
    #[sea_orm(has_many = "super::room_subscriptions::Entity")]
    RoomSubscriptions,
    #[sea_orm(has_many = "super::message_cursors::Entity")]
    MessageCursors,
}

impl Related<super::room_messages::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::RoomMessages.def()
    }
}

impl Related<super::room_subscriptions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::RoomSubscriptions.def()
    }
}

impl Related<super::message_cursors::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MessageCursors.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

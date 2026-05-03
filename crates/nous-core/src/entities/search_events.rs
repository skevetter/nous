use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "search_events")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub query_text: String,
    pub search_type: String,
    pub result_count: i32,
    pub latency_ms: i32,
    pub workspace_id: Option<String>,
    pub agent_id: Option<String>,
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

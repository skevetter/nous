use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS room_subscriptions (\
             room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE, \
             agent_id TEXT NOT NULL, \
             topics TEXT, \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             PRIMARY KEY (room_id, agent_id)\
             )",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(RoomSubscriptions::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum RoomSubscriptions {
    Table,
}

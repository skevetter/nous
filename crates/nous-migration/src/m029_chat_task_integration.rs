use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS message_cursors (\
             room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE, \
             agent_id TEXT NOT NULL, \
             last_read_message_id TEXT NOT NULL, \
             last_read_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             PRIMARY KEY (room_id, agent_id)\
             ); \
             CREATE INDEX IF NOT EXISTS idx_cursors_agent ON message_cursors(agent_id); \
             CREATE TABLE IF NOT EXISTS notification_queue (\
             id TEXT NOT NULL PRIMARY KEY, \
             agent_id TEXT NOT NULL, \
             room_id TEXT NOT NULL, \
             message_id TEXT NOT NULL, \
             sender_id TEXT NOT NULL, \
             priority TEXT NOT NULL DEFAULT 'normal' CHECK(priority IN ('low','normal','high','urgent')), \
             topics TEXT NOT NULL DEFAULT '[]', \
             mentions TEXT NOT NULL DEFAULT '[]', \
             delivered INTEGER NOT NULL DEFAULT 0, \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
             ); \
             CREATE INDEX IF NOT EXISTS idx_notif_queue_agent ON notification_queue(agent_id, delivered, created_at); \
             CREATE INDEX IF NOT EXISTS idx_notif_queue_room ON notification_queue(room_id, created_at); \
             CREATE INDEX IF NOT EXISTS idx_room_messages_room_created ON room_messages(room_id, created_at); \
             CREATE INDEX IF NOT EXISTS idx_room_messages_reply_to ON room_messages(reply_to) WHERE reply_to IS NOT NULL; \
             CREATE INDEX IF NOT EXISTS idx_room_messages_sender ON room_messages(room_id, sender_id, created_at); \
             ALTER TABLE room_messages ADD COLUMN message_type TEXT NOT NULL DEFAULT 'user' CHECK(message_type IN ('user','system','task_event','command','handoff'))"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "DROP TABLE IF EXISTS notification_queue; \
             DROP TABLE IF EXISTS message_cursors;",
        )
        .await?;
        Ok(())
    }
}

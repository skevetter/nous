use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "CREATE TABLE IF NOT EXISTS task_dependencies (\
             id TEXT NOT NULL PRIMARY KEY, \
             task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
             depends_on_task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
             dep_type TEXT NOT NULL DEFAULT 'blocked_by' CHECK(dep_type IN ('blocked_by','blocks','waiting_on')), \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             UNIQUE(task_id, depends_on_task_id, dep_type)\
             )",
            "CREATE INDEX IF NOT EXISTS idx_task_deps_task ON task_dependencies(task_id)",
            "CREATE INDEX IF NOT EXISTS idx_task_deps_depends ON task_dependencies(depends_on_task_id)",
            "CREATE TABLE IF NOT EXISTS task_templates (\
             id TEXT NOT NULL PRIMARY KEY, \
             name TEXT NOT NULL UNIQUE, \
             title_pattern TEXT NOT NULL, \
             description_template TEXT, \
             default_priority TEXT NOT NULL DEFAULT 'medium' CHECK(default_priority IN ('critical','high','medium','low')), \
             default_labels TEXT NOT NULL DEFAULT '[]', \
             checklist TEXT NOT NULL DEFAULT '[]', \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
             )",
            "CREATE TRIGGER IF NOT EXISTS task_templates_au AFTER UPDATE ON task_templates WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE task_templates SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "DROP TRIGGER IF EXISTS task_templates_au",
            "DROP TABLE IF EXISTS task_templates",
            "DROP TABLE IF EXISTS task_dependencies",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}

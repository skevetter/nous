pub use sea_orm_migration::prelude::*;

mod m001_schema_version;
mod m002_rooms;
mod m003_room_messages;
mod m004_room_messages_fts;
mod m005_room_subscriptions;
mod m006_tasks;
mod m007_task_links;
mod m008_task_events;
mod m009_tasks_fts;
mod m010_worktrees;
mod m011_agents;
mod m012_agent_relationships_and_artifacts;
mod m013_agents_fts;
mod m014_schedules;
mod m015_inventory;
mod m016_memories;
mod m017_agent_lifecycle;
mod m018_memory_embeddings;
mod m019_task_dependencies_and_templates;
mod m020_memory_sessions;
mod m021_search_events;
mod m022_fts_rebuild;
mod m023_agent_processes;
mod m024_agent_invocations;
mod m025_agent_process_config;
mod m026_sandbox_support;
mod m027_resources;
mod m028_resources_data_migration;
mod m029_chat_task_integration;
mod m030_remove_agent_type;
mod m031_schedule_timestamps_iso;
mod m032_add_agent_fk_constraints;
mod m033_drop_deprecated_tables;
mod m034_add_fk_indexes;
mod m035_restore_agent_type;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m001_schema_version::Migration),
            Box::new(m002_rooms::Migration),
            Box::new(m003_room_messages::Migration),
            Box::new(m004_room_messages_fts::Migration),
            Box::new(m005_room_subscriptions::Migration),
            Box::new(m006_tasks::Migration),
            Box::new(m007_task_links::Migration),
            Box::new(m008_task_events::Migration),
            Box::new(m009_tasks_fts::Migration),
            Box::new(m010_worktrees::Migration),
            Box::new(m011_agents::Migration),
            Box::new(m012_agent_relationships_and_artifacts::Migration),
            Box::new(m013_agents_fts::Migration),
            Box::new(m014_schedules::Migration),
            Box::new(m015_inventory::Migration),
            Box::new(m016_memories::Migration),
            Box::new(m017_agent_lifecycle::Migration),
            Box::new(m018_memory_embeddings::Migration),
            Box::new(m019_task_dependencies_and_templates::Migration),
            Box::new(m020_memory_sessions::Migration),
            Box::new(m021_search_events::Migration),
            Box::new(m022_fts_rebuild::Migration),
            Box::new(m023_agent_processes::Migration),
            Box::new(m024_agent_invocations::Migration),
            Box::new(m025_agent_process_config::Migration),
            Box::new(m026_sandbox_support::Migration),
            Box::new(m027_resources::Migration),
            Box::new(m028_resources_data_migration::Migration),
            Box::new(m029_chat_task_integration::Migration),
            Box::new(m030_remove_agent_type::Migration),
            Box::new(m031_schedule_timestamps_iso::Migration),
            Box::new(m032_add_agent_fk_constraints::Migration),
            Box::new(m033_drop_deprecated_tables::Migration),
            Box::new(m034_add_fk_indexes::Migration),
            Box::new(m035_restore_agent_type::Migration),
        ]
    }
}

// Integration test for migration 026: sandbox support

use nous_core::agents::processes::{create_process, get_process_by_id, CreateProcessParams};
use nous_core::agents::{self, RegisterAgentRequest};
use nous_core::db::DbPools;
use sea_orm::{ConnectionTrait, Statement, TryGetable};
use tempfile::TempDir;
use uuid::Uuid;

#[tokio::test]
async fn migration_026_runs_on_fresh_db() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();

    // Verify agent_processes table has sandbox columns
    let rows = pools.fts.query_all(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        "SELECT name, type FROM pragma_table_info('agent_processes') WHERE name LIKE 'sandbox_%'",
    ))
    .await
    .unwrap();

    assert_eq!(rows.len(), 6, "should have 6 sandbox columns");

    let column_names: Vec<String> = rows
        .iter()
        .map(|r| String::try_get_by(r, 0usize).unwrap())
        .collect();
    assert!(column_names.contains(&"sandbox_image".to_string()));
    assert!(column_names.contains(&"sandbox_cpus".to_string()));
    assert!(column_names.contains(&"sandbox_memory_mib".to_string()));
    assert!(column_names.contains(&"sandbox_network_policy".to_string()));
    assert!(column_names.contains(&"sandbox_volumes_json".to_string()));
    assert!(column_names.contains(&"sandbox_name".to_string()));

    pools.close().await;
}

#[tokio::test]
async fn migration_026_runs_on_existing_db_with_data() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();

    // Run migrations including 026
    pools.run_migrations().await.unwrap();

    // Create test agent
    let agent = agents::register_agent(
        &pools.fts,
        RegisterAgentRequest {
            name: "existing-data-agent".into(),
            agent_type: None,
            parent_id: None,
            namespace: Some("default".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    // Create an old-style process (shell type, no sandbox fields)
    let old_process = create_process(CreateProcessParams {
        db: &pools.fts,
        agent_id: &agent.id,
        process_type: "shell",
        command: "echo existing",
        working_dir: Some("/tmp"),
        env_json: None,
        timeout_secs: None,
    })
    .await
    .unwrap();

    // Verify old process exists
    let count_row = pools
        .fts
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT COUNT(*) FROM agent_processes",
        ))
        .await
        .unwrap()
        .unwrap();
    let count: i64 = i64::try_get_by(&count_row, 0usize).unwrap();
    assert_eq!(count, 1, "should have 1 process");

    // Verify old process has NULL sandbox columns via the Process struct
    let fetched = get_process_by_id(&pools.fts, &old_process.id)
        .await
        .unwrap();
    assert_eq!(fetched.id, old_process.id);
    assert_eq!(fetched.process_type, "shell");
    assert_eq!(
        fetched.sandbox_image, None,
        "sandbox_image should be NULL for old process"
    );

    // Verify running migrations again is idempotent
    pools.run_migrations().await.unwrap();

    // Verify process still exists after re-running migrations
    let count_row2 = pools
        .fts
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT COUNT(*) FROM agent_processes",
        ))
        .await
        .unwrap()
        .unwrap();
    let count_after: i64 = i64::try_get_by(&count_row2, 0usize).unwrap();
    assert_eq!(count_after, 1, "process should still exist");

    pools.close().await;
}

#[tokio::test]
async fn process_struct_reads_and_writes_sandbox_fields() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();

    // Create test agent
    let agent = agents::register_agent(
        &pools.fts,
        RegisterAgentRequest {
            name: "sandbox-test-agent".into(),
            agent_type: None,
            parent_id: None,
            namespace: Some("default".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    // Create process with sandbox fields
    let process_id = Uuid::now_v7().to_string();
    pools
        .fts
        .execute(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            "INSERT INTO agent_processes (id, agent_id, process_type, command, \
         sandbox_image, sandbox_cpus, sandbox_memory_mib, sandbox_network_policy, \
         sandbox_volumes_json, sandbox_name) \
         VALUES (?, ?, 'sandbox', 'claude-code', 'ubuntu:24.04', 2, 512, 'isolated', \
         '[{\"guest_path\":\"/workspace\",\"readonly\":false}]', 'test-sandbox')",
            [process_id.clone().into(), agent.id.clone().into()],
        ))
        .await
        .unwrap();

    // Read back the process via Process struct
    let process = get_process_by_id(&pools.fts, &process_id).await.unwrap();

    assert_eq!(process.id, process_id);
    assert_eq!(process.agent_id, agent.id);
    assert_eq!(process.process_type, "sandbox");
    assert_eq!(process.command, "claude-code");
    assert_eq!(process.sandbox_image, Some("ubuntu:24.04".to_string()));
    assert_eq!(process.sandbox_cpus, Some(2));
    assert_eq!(process.sandbox_memory_mib, Some(512));
    assert_eq!(process.sandbox_network_policy, Some("isolated".to_string()));
    assert_eq!(
        process.sandbox_volumes_json,
        Some("[{\"guest_path\":\"/workspace\",\"readonly\":false}]".to_string())
    );
    assert_eq!(process.sandbox_name, Some("test-sandbox".to_string()));

    pools.close().await;
}

#[tokio::test]
async fn existing_process_types_still_work() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();

    // Create test agent
    let agent = agents::register_agent(
        &pools.fts,
        RegisterAgentRequest {
            name: "legacy-test-agent".into(),
            agent_type: None,
            parent_id: None,
            namespace: Some("default".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    // Test shell process
    let shell_process = create_process(CreateProcessParams {
        db: &pools.fts,
        agent_id: &agent.id,
        process_type: "shell",
        command: "echo hello",
        working_dir: Some("/tmp"),
        env_json: Some(r#"{"PATH":"/usr/bin"}"#),
        timeout_secs: Some(30),
    })
    .await
    .unwrap();

    assert_eq!(shell_process.process_type, "shell");
    assert_eq!(shell_process.command, "echo hello");
    assert_eq!(shell_process.working_dir, Some("/tmp".to_string()));
    assert_eq!(shell_process.sandbox_image, None);

    // Mark shell process as stopped to allow creating another active process
    pools
        .fts
        .execute(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            "UPDATE agent_processes SET status = 'stopped' WHERE id = ?",
            [shell_process.id.clone().into()],
        ))
        .await
        .unwrap();

    // Test claude process
    let claude_process = create_process(CreateProcessParams {
        db: &pools.fts,
        agent_id: &agent.id,
        process_type: "claude",
        command: "claude-code --model opus",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
    })
    .await
    .unwrap();

    assert_eq!(claude_process.process_type, "claude");
    assert_eq!(claude_process.command, "claude-code --model opus");
    assert_eq!(claude_process.sandbox_cpus, None);

    // Mark claude process as stopped to allow creating another active process
    pools
        .fts
        .execute(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            "UPDATE agent_processes SET status = 'stopped' WHERE id = ?",
            [claude_process.id.clone().into()],
        ))
        .await
        .unwrap();

    // Test http process
    let http_process = create_process(CreateProcessParams {
        db: &pools.fts,
        agent_id: &agent.id,
        process_type: "http",
        command: "http://localhost:8080",
        working_dir: None,
        env_json: None,
        timeout_secs: Some(60),
    })
    .await
    .unwrap();

    assert_eq!(http_process.process_type, "http");
    assert_eq!(http_process.command, "http://localhost:8080");
    assert_eq!(http_process.timeout_secs, Some(60));
    assert_eq!(http_process.sandbox_memory_mib, None);

    // Verify all three processes can be retrieved
    let retrieved_shell = get_process_by_id(&pools.fts, &shell_process.id)
        .await
        .unwrap();
    let retrieved_claude = get_process_by_id(&pools.fts, &claude_process.id)
        .await
        .unwrap();
    let retrieved_http = get_process_by_id(&pools.fts, &http_process.id)
        .await
        .unwrap();

    assert_eq!(retrieved_shell.process_type, "shell");
    assert_eq!(retrieved_claude.process_type, "claude");
    assert_eq!(retrieved_http.process_type, "http");

    pools.close().await;
}

#[tokio::test]
async fn sandbox_config_serde_roundtrip() {
    use nous_core::agents::sandbox::{SandboxConfig, SecretConfig, VolumeMount};

    let config = SandboxConfig {
        image: "alpine:latest".to_string(),
        cpus: Some(4),
        memory_mib: Some(1024),
        network_policy: Some("bridge".to_string()),
        volumes: Some(vec![
            VolumeMount {
                guest_path: "/data".to_string(),
                host_path: Some("/host/data".to_string()),
                readonly: true,
            },
            VolumeMount {
                guest_path: "/workspace".to_string(),
                host_path: None,
                readonly: false,
            },
        ]),
        secrets: Some(vec![SecretConfig {
            name: "API_KEY".to_string(),
            allowed_hosts: Some(vec!["api.example.com".to_string()]),
        }]),
        max_duration_secs: Some(7200),
        idle_timeout_secs: Some(600),
    };

    let json = serde_json::to_string(&config).unwrap();
    let deserialized: SandboxConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.image, "alpine:latest");
    assert_eq!(deserialized.cpus, Some(4));
    assert_eq!(deserialized.memory_mib, Some(1024));
    assert_eq!(deserialized.network_policy.as_deref(), Some("bridge"));

    let vols = deserialized.volumes.unwrap();
    assert_eq!(vols.len(), 2);
    assert_eq!(vols[0].guest_path, "/data");
    assert!(vols[0].readonly);
    assert_eq!(vols[1].guest_path, "/workspace");
    assert!(!vols[1].readonly);
    assert_eq!(vols[1].host_path, None);

    let secs = deserialized.secrets.unwrap();
    assert_eq!(secs.len(), 1);
    assert_eq!(secs[0].name, "API_KEY");
}

#[tokio::test]
async fn create_sandbox_process_sets_correct_fields() {
    use nous_core::agents::processes::{create_sandbox_process, CreateSandboxProcessParams};

    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();

    let agent = agents::register_agent(
        &pools.fts,
        RegisterAgentRequest {
            name: "create-sandbox-test".into(),
            agent_type: None,
            parent_id: None,
            namespace: Some("default".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let process = create_sandbox_process(CreateSandboxProcessParams {
        db: &pools.fts,
        agent_id: &agent.id,
        sandbox_image: "python:3.12",
        sandbox_cpus: Some(4),
        sandbox_memory_mib: Some(1024),
        sandbox_network_policy: Some("public-only"),
        sandbox_volumes_json: Some(
            r#"[{"guest_path":"/app","host_path":"/host/app","readonly":true}]"#,
        ),
        sandbox_name: Some("my-sandbox"),
        timeout_secs: Some(3600),
    })
    .await
    .unwrap();

    assert_eq!(process.process_type, "sandbox");
    assert_eq!(process.sandbox_image, Some("python:3.12".to_string()));
    assert_eq!(process.sandbox_cpus, Some(4));
    assert_eq!(process.sandbox_memory_mib, Some(1024));
    assert_eq!(
        process.sandbox_network_policy,
        Some("public-only".to_string())
    );
    assert_eq!(
        process.sandbox_volumes_json,
        Some(r#"[{"guest_path":"/app","host_path":"/host/app","readonly":true}]"#.to_string())
    );
    assert_eq!(process.sandbox_name, Some("my-sandbox".to_string()));
    assert_eq!(process.status, "pending");
    assert_eq!(process.command, "sandbox:python:3.12");
    assert_eq!(process.timeout_secs, Some(3600));

    pools.close().await;
}

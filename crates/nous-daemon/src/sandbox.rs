use std::collections::{HashMap, HashSet};

use nous_core::agents::sandbox::SandboxConfig;
use nous_core::error::NousError;
use tracing::info;

#[derive(Debug, Clone)]
pub struct SandboxHandle {
    pub name: String,
    pub agent_id: String,
    pub image: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct SandboxMetrics {
    pub cpu_usage_percent: f64,
    pub memory_used_mib: u64,
    pub disk_used_mib: u64,
}

#[derive(Debug, Clone)]
pub struct SandboxInfo {
    pub agent_id: String,
    pub name: String,
    pub image: String,
    pub status: String,
}

#[derive(Default)]
pub struct SandboxManager {
    sandboxes: HashMap<String, SandboxHandle>,
    known_live: HashSet<String>,
}

impl SandboxManager {
    pub fn new() -> Self {
        Self {
            sandboxes: HashMap::new(),
            known_live: HashSet::new(),
        }
    }

    pub async fn create(
        &mut self,
        config: &SandboxConfig,
        agent_id: &str,
    ) -> Result<String, NousError> {
        let name = format!("sandbox-{}-{}", agent_id, uuid::Uuid::now_v7());
        info!(agent_id, name = %name, image = %config.image, "creating sandbox");

        let handle = SandboxHandle {
            name: name.clone(),
            agent_id: agent_id.to_string(),
            image: config.image.clone(),
            status: "running".to_string(),
        };
        self.sandboxes.insert(agent_id.to_string(), handle);
        self.known_live.insert(name.clone());

        Ok(name)
    }

    pub async fn stop(&mut self, agent_id: &str) -> Result<(), NousError> {
        let handle = self
            .sandboxes
            .get_mut(agent_id)
            .ok_or_else(|| NousError::NotFound(format!("no sandbox for agent {}", agent_id)))?;

        info!(agent_id, name = %handle.name, "stopping sandbox");
        self.known_live.remove(&handle.name);
        handle.status = "stopped".to_string();
        Ok(())
    }

    pub async fn exec(
        &self,
        agent_id: &str,
        cmd: &str,
        args: &[&str],
    ) -> Result<String, NousError> {
        let _handle = self
            .sandboxes
            .get(agent_id)
            .ok_or_else(|| NousError::NotFound(format!("no sandbox for agent {}", agent_id)))?;

        Err(NousError::Internal(format!(
            "sandbox runtime not available (cmd: {} {:?})",
            cmd, args
        )))
    }

    pub async fn metrics(&self, agent_id: &str) -> Result<SandboxMetrics, NousError> {
        let _handle = self
            .sandboxes
            .get(agent_id)
            .ok_or_else(|| NousError::NotFound(format!("no sandbox for agent {}", agent_id)))?;

        Ok(SandboxMetrics {
            cpu_usage_percent: 0.0,
            memory_used_mib: 0,
            disk_used_mib: 0,
        })
    }

    pub fn list(&self) -> Vec<SandboxInfo> {
        self.sandboxes
            .values()
            .map(|h| SandboxInfo {
                agent_id: h.agent_id.clone(),
                name: h.name.clone(),
                image: h.image.clone(),
                status: h.status.clone(),
            })
            .collect()
    }

    pub fn get(&self, agent_id: &str) -> Option<&SandboxHandle> {
        self.sandboxes.get(agent_id)
    }

    pub fn is_sandbox_alive(&self, sandbox_name: &str) -> bool {
        self.known_live.contains(sandbox_name)
    }

    pub fn register_known_sandbox(&mut self, sandbox_name: &str) {
        self.known_live.insert(sandbox_name.to_string());
    }

    pub async fn reconnect(
        &mut self,
        agent_id: &str,
        sandbox_name: &str,
        image: &str,
    ) -> Result<bool, NousError> {
        info!(agent_id, sandbox_name, "attempting sandbox reconnect");

        if !self.is_sandbox_alive(sandbox_name) {
            info!(agent_id, sandbox_name, "sandbox not alive, cannot reconnect");
            return Ok(false);
        }

        let handle = SandboxHandle {
            name: sandbox_name.to_string(),
            agent_id: agent_id.to_string(),
            image: image.to_string(),
            status: "running".to_string(),
        };
        self.sandboxes.insert(agent_id.to_string(), handle);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SandboxConfig {
        SandboxConfig {
            image: "ubuntu:24.04".to_string(),
            cpus: Some(2),
            memory_mib: Some(512),
            network_policy: Some("none".to_string()),
            volumes: None,
            secrets: None,
            max_duration_secs: Some(3600),
            idle_timeout_secs: None,
        }
    }

    #[tokio::test]
    async fn create_returns_sandbox_name() {
        let mut mgr = SandboxManager::new();
        let config = test_config();
        let name = mgr.create(&config, "agent-1").await.unwrap();
        assert!(name.starts_with("sandbox-agent-1-"));
    }

    #[tokio::test]
    async fn get_returns_handle_after_create() {
        let mut mgr = SandboxManager::new();
        let config = test_config();
        mgr.create(&config, "agent-2").await.unwrap();

        let handle = mgr.get("agent-2").unwrap();
        assert_eq!(handle.agent_id, "agent-2");
        assert_eq!(handle.image, "ubuntu:24.04");
        assert_eq!(handle.status, "running");
    }

    #[tokio::test]
    async fn stop_sets_status_stopped() {
        let mut mgr = SandboxManager::new();
        let config = test_config();
        mgr.create(&config, "agent-3").await.unwrap();
        mgr.stop("agent-3").await.unwrap();

        let handle = mgr.get("agent-3").unwrap();
        assert_eq!(handle.status, "stopped");
    }

    #[tokio::test]
    async fn stop_nonexistent_returns_not_found() {
        let mut mgr = SandboxManager::new();
        let err = mgr.stop("missing").await.unwrap_err();
        assert!(matches!(err, NousError::NotFound(_)));
    }

    #[tokio::test]
    async fn exec_returns_runtime_not_available() {
        let mut mgr = SandboxManager::new();
        let config = test_config();
        mgr.create(&config, "agent-4").await.unwrap();

        let err = mgr.exec("agent-4", "ls", &["-la"]).await.unwrap_err();
        assert!(matches!(err, NousError::Internal(_)));
    }

    #[tokio::test]
    async fn metrics_returns_zeroed() {
        let mut mgr = SandboxManager::new();
        let config = test_config();
        mgr.create(&config, "agent-5").await.unwrap();

        let m = mgr.metrics("agent-5").await.unwrap();
        assert_eq!(m.cpu_usage_percent, 0.0);
        assert_eq!(m.memory_used_mib, 0);
        assert_eq!(m.disk_used_mib, 0);
    }

    #[tokio::test]
    async fn list_returns_all_sandboxes() {
        let mut mgr = SandboxManager::new();
        let config = test_config();
        mgr.create(&config, "agent-a").await.unwrap();
        mgr.create(&config, "agent-b").await.unwrap();

        let list = mgr.list();
        assert_eq!(list.len(), 2);
    }
}

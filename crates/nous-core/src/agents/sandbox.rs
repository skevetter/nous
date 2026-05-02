use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub image: String,
    pub cpus: Option<u32>,
    pub memory_mib: Option<u32>,
    pub network_policy: Option<String>,
    pub volumes: Option<Vec<VolumeMount>>,
    pub secrets: Option<Vec<SecretConfig>>,
    pub max_duration_secs: Option<u64>,
    pub idle_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMount {
    pub guest_path: String,
    pub host_path: Option<String>,
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretConfig {
    pub name: String,
    pub allowed_hosts: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let config = SandboxConfig {
            image: "ubuntu:24.04".to_string(),
            cpus: Some(2),
            memory_mib: Some(512),
            network_policy: Some("isolated".to_string()),
            volumes: Some(vec![VolumeMount {
                guest_path: "/workspace".to_string(),
                host_path: Some("/tmp/sandbox-ws".to_string()),
                readonly: false,
            }]),
            secrets: Some(vec![SecretConfig {
                name: "GITHUB_TOKEN".to_string(),
                allowed_hosts: Some(vec!["github.com".to_string()]),
            }]),
            max_duration_secs: Some(3600),
            idle_timeout_secs: Some(300),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: SandboxConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.image, "ubuntu:24.04");
        assert_eq!(deserialized.cpus, Some(2));
        assert_eq!(deserialized.memory_mib, Some(512));
        assert_eq!(deserialized.network_policy.as_deref(), Some("isolated"));
        assert_eq!(deserialized.max_duration_secs, Some(3600));
        assert_eq!(deserialized.idle_timeout_secs, Some(300));

        let volumes = deserialized.volumes.unwrap();
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].guest_path, "/workspace");
        assert_eq!(volumes[0].host_path.as_deref(), Some("/tmp/sandbox-ws"));
        assert!(!volumes[0].readonly);

        let secrets = deserialized.secrets.unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].name, "GITHUB_TOKEN");
        assert_eq!(
            secrets[0].allowed_hosts.as_ref().unwrap(),
            &vec!["github.com".to_string()]
        );
    }
}

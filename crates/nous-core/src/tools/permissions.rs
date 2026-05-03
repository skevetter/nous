use std::path::PathBuf;
use std::time::Duration;

use super::{NetworkPolicy, ResolvedPermissions, ToolContext};

pub fn resolve_permissions(
    allowed_tools: Option<Vec<String>>,
    denied_tools: Option<Vec<String>>,
    allowed_paths: Option<Vec<PathBuf>>,
    network_access: NetworkPolicy,
    max_output_bytes: Option<usize>,
) -> ResolvedPermissions {
    ResolvedPermissions {
        allowed_tools,
        denied_tools,
        allowed_paths,
        network_access,
        max_output_bytes: max_output_bytes.unwrap_or(1_048_576),
    }
}

pub fn build_default_context(
    agent_id: String,
    agent_name: String,
    namespace: String,
    permissions: ResolvedPermissions,
) -> ToolContext {
    ToolContext {
        agent_id,
        agent_name,
        namespace,
        workspace_dir: None,
        session_id: None,
        timeout: Duration::from_secs(30),
        permissions,
    }
}

pub fn is_tool_allowed(name: &str, perms: &ResolvedPermissions) -> bool {
    if let Some(ref denied) = perms.denied_tools {
        if denied.iter().any(|d| d == name) {
            return false;
        }
    }
    if let Some(ref allowed) = perms.allowed_tools {
        return allowed.iter().any(|a| a == name);
    }
    true
}

pub fn is_network_allowed(perms: &ResolvedPermissions) -> bool {
    !matches!(perms.network_access, NetworkPolicy::None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perms_with_allowlist(tools: Vec<&str>) -> ResolvedPermissions {
        ResolvedPermissions {
            allowed_tools: Some(tools.into_iter().map(String::from).collect()),
            denied_tools: None,
            allowed_paths: None,
            network_access: NetworkPolicy::Unrestricted,
            max_output_bytes: 1_048_576,
        }
    }

    fn perms_with_denylist(tools: Vec<&str>) -> ResolvedPermissions {
        ResolvedPermissions {
            allowed_tools: None,
            denied_tools: Some(tools.into_iter().map(String::from).collect()),
            allowed_paths: None,
            network_access: NetworkPolicy::Unrestricted,
            max_output_bytes: 1_048_576,
        }
    }

    fn perms_with_network(policy: NetworkPolicy) -> ResolvedPermissions {
        ResolvedPermissions {
            allowed_tools: None,
            denied_tools: None,
            allowed_paths: None,
            network_access: policy,
            max_output_bytes: 1_048_576,
        }
    }

    #[test]
    fn allowlist_blocks_unlisted_tool() {
        let perms = perms_with_allowlist(vec!["fs_read", "fs_write"]);
        assert!(is_tool_allowed("fs_read", &perms));
        assert!(!is_tool_allowed("shell_exec", &perms));
    }

    #[test]
    fn denylist_blocks_listed_tool() {
        let perms = perms_with_denylist(vec!["fs_delete", "shell_kill"]);
        assert!(!is_tool_allowed("fs_delete", &perms));
        assert!(is_tool_allowed("fs_read", &perms));
    }

    #[test]
    fn no_lists_allows_all() {
        let perms = ResolvedPermissions {
            allowed_tools: None,
            denied_tools: None,
            allowed_paths: None,
            network_access: NetworkPolicy::Unrestricted,
            max_output_bytes: 1_048_576,
        };
        assert!(is_tool_allowed("anything", &perms));
    }

    #[test]
    fn network_policy_none_blocks() {
        let perms = perms_with_network(NetworkPolicy::None);
        assert!(!is_network_allowed(&perms));
    }

    #[test]
    fn network_policy_unrestricted_allows() {
        let perms = perms_with_network(NetworkPolicy::Unrestricted);
        assert!(is_network_allowed(&perms));
    }

    #[test]
    fn network_policy_isolated_allows() {
        let perms = perms_with_network(NetworkPolicy::Isolated);
        assert!(is_network_allowed(&perms));
    }

    #[test]
    fn resolve_permissions_defaults_output_bytes() {
        let perms = resolve_permissions(None, None, None, NetworkPolicy::None, None);
        assert_eq!(perms.max_output_bytes, 1_048_576);
    }

    #[test]
    fn resolve_permissions_custom_output_bytes() {
        let perms = resolve_permissions(None, None, None, NetworkPolicy::None, Some(512));
        assert_eq!(perms.max_output_bytes, 512);
    }
}

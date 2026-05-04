use std::path::Path;

use nous_core::agents::definition::{load_definition, resolve_skills};
use nous_core::error::NousError;

fn examples_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

#[test]
fn parse_reviewer_example() {
    let path = examples_dir().join("agents/reviewer.toml");
    let def = load_definition(&path).expect("failed to parse reviewer.toml");

    assert_eq!(def.agent.name, "reviewer");
    assert_eq!(def.agent.version, "1.2.0");
    assert_eq!(def.agent.namespace.as_deref(), Some("eng"));
    assert_eq!(
        def.agent.description.as_deref(),
        Some("Performs code review on feature branches")
    );

    let process = def.process.expect("reviewer should have [process]");
    assert_eq!(process.r#type.as_deref(), Some("claude"));
    assert_eq!(
        process.spawn_command.as_deref(),
        Some("claude --model claude-sonnet-4-6")
    );
    assert_eq!(process.working_dir.as_deref(), Some("~"));
    assert_eq!(process.auto_restart, Some(false));

    let skills = def.skills.expect("reviewer should have [skills]");
    assert_eq!(skills.refs, vec!["code-review", "git-workflow"]);

    let meta = def.metadata.expect("reviewer should have [metadata]");
    assert_eq!(
        meta.model.as_deref(),
        Some("global.anthropic.claude-sonnet-4-6-v1")
    );
    assert_eq!(meta.timeout, Some(3600));
    assert_eq!(
        meta.tags.as_deref(),
        Some(vec!["review".to_string(), "quality".to_string()].as_slice())
    );
}

#[test]
fn parse_planner_example() {
    let path = examples_dir().join("agents/planner.toml");
    let def = load_definition(&path).expect("failed to parse planner.toml");

    assert_eq!(def.agent.name, "planner");
    assert_eq!(def.agent.version, "0.3.0");
    assert!(def.agent.namespace.is_none());
    assert_eq!(
        def.agent.description.as_deref(),
        Some("Decomposes epics into actionable tasks")
    );

    let process = def.process.expect("planner should have [process]");
    assert_eq!(process.r#type.as_deref(), Some("claude"));
    assert_eq!(
        process.spawn_command.as_deref(),
        Some("claude --model claude-opus-4-6")
    );
    assert!(process.working_dir.is_none());
    assert!(process.auto_restart.is_none());

    let skills = def.skills.expect("planner should have [skills]");
    assert_eq!(skills.refs, vec!["planning"]);

    let meta = def.metadata.expect("planner should have [metadata]");
    assert_eq!(
        meta.model.as_deref(),
        Some("global.anthropic.claude-opus-4-6-v1")
    );
    assert_eq!(meta.timeout, Some(7200));
    assert_eq!(
        meta.tags.as_deref(),
        Some(vec!["planning".to_string(), "decomposition".to_string()].as_slice())
    );
}

#[test]
fn resolve_reviewer_skills_all_present() {
    let skills_dir = examples_dir().join("skills");
    let refs = vec!["code-review".to_string(), "git-workflow".to_string()];

    let resolved = resolve_skills(&refs, &skills_dir).expect("all reviewer skills should resolve");
    assert_eq!(resolved.len(), 2);

    assert_eq!(resolved[0].name, "code-review");
    assert!(resolved[0].path.ends_with("code-review.md"));
    assert!(resolved[0].content.contains("# Code Review"));

    assert_eq!(resolved[1].name, "git-workflow");
    assert!(resolved[1].path.ends_with("git-workflow.md"));
    assert!(resolved[1].content.contains("# Git Workflow"));
}

#[test]
fn resolve_planner_skills() {
    let skills_dir = examples_dir().join("skills");
    let refs = vec!["planning".to_string()];

    let resolved = resolve_skills(&refs, &skills_dir).expect("planning skill should resolve");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].name, "planning");
    assert!(resolved[0].content.contains("# Planning"));
}

#[test]
fn resolve_missing_skill_returns_not_found() {
    let skills_dir = examples_dir().join("skills");
    let refs = vec!["nonexistent-skill".to_string()];

    let err = resolve_skills(&refs, &skills_dir).expect_err("should fail for missing skill");
    match err {
        NousError::NotFound(msg) => {
            assert!(
                msg.contains("nonexistent-skill"),
                "error should mention the missing skill name: {msg}"
            );
        }
        other => panic!("expected NotFound, got: {other:?}"),
    }
}

#[test]
fn resolve_partial_refs_fails_on_missing() {
    let skills_dir = examples_dir().join("skills");
    let refs = vec!["code-review".to_string(), "does-not-exist".to_string()];

    let err = resolve_skills(&refs, &skills_dir).expect_err("should fail when any ref is missing");
    match err {
        NousError::NotFound(msg) => {
            assert!(msg.contains("does-not-exist"));
        }
        other => panic!("expected NotFound, got: {other:?}"),
    }
}

#[test]
fn load_definition_and_resolve_roundtrip() {
    let def =
        load_definition(&examples_dir().join("agents/reviewer.toml")).expect("parse reviewer.toml");

    let skills_dir = examples_dir().join("skills");
    let skill_refs = def.skills.expect("reviewer has skills");
    let resolved =
        resolve_skills(&skill_refs.refs, &skills_dir).expect("all reviewer skills should resolve");

    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].name, "code-review");
    assert_eq!(resolved[1].name, "git-workflow");
}

//! Memory-isolation variables of `Profile::process_env` (round 5, R4).

use super::*;

fn names(env: &ProcessEnv) -> Vec<&str> {
    env.set.iter().map(|(name, _)| name.as_str()).collect()
}

#[test]
fn claude_gets_only_the_auto_memory_switch() {
    let env = Profile::new(AgentKind::Claude, "/p").process_env(None);
    assert_eq!(
        names(&env),
        vec!["CLAUDE_CONFIG_DIR", "CLAUDE_CODE_DISABLE_AUTO_MEMORY"]
    );
    assert_eq!(env.set[1].1, "1");
}

#[test]
fn codex_gets_only_the_config_json() {
    let env = Profile::new(AgentKind::Codex, "/p").process_env(None);
    assert_eq!(names(&env), vec!["CODEX_HOME", "CODEX_CONFIG"]);
    let json: serde_json::Value = serde_json::from_str(&env.set[1].1).expect("valid JSON");
    assert_eq!(json["features"]["memories"], false);
    assert_eq!(json["memories"]["use_memories"], false);
    assert_eq!(json["memories"]["generate_memories"], false);
    assert_eq!(json["history"]["persistence"], "none");
}

#[test]
fn antigravity_gets_nothing_new() {
    let env = Profile::new(AgentKind::Antigravity, "/p").process_env(None);
    assert_eq!(names(&env), vec!["GEMINI_HOME", "BROWSER", "TMPDIR"]);
}

#[test]
fn login_env_keeps_the_memory_variables() {
    let claude = Profile::new(AgentKind::Claude, "/p").login_env(None);
    assert!(names(&claude).contains(&"CLAUDE_CODE_DISABLE_AUTO_MEMORY"));
    let codex = Profile::new(AgentKind::Codex, "/p").login_env(None);
    assert!(names(&codex).contains(&"CODEX_CONFIG"));
}

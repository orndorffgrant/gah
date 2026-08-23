use gah_agent::build_agent;
use gah_core::{AgentConfig, ProviderKind};

fn openrouter_config() -> AgentConfig {
    AgentConfig {
        provider: ProviderKind::OpenRouter,
        model: "openrouter/auto".into(),
        api_key: String::new(),
        api_base_url: None,
        system_prompt: None,
    }
}

#[test]
fn empty_key_falls_back_to_env() {
    std::env::set_var("OPENROUTER_API_KEY", "sk-or-test");
    assert!(build_agent(&openrouter_config()).is_ok());
}

#[test]
fn empty_key_without_env_errors() {
    std::env::remove_var("OPENROUTER_API_KEY");
    let err = match build_agent(&openrouter_config()) {
        Err(e) => e,
        Ok(_) => panic!("expected an error when no key and no env var"),
    };
    assert!(
        err.to_string().contains("OPENROUTER_API_KEY"),
        "error should name the env var: {err}"
    );
}

#[test]
fn explicit_key_wins_over_missing_env() {
    std::env::remove_var("OPENROUTER_API_KEY");
    let mut config = openrouter_config();
    config.api_key = "sk-or-explicit".into();
    assert!(build_agent(&config).is_ok());
}

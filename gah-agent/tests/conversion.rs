use gah_core::{AgentConfig, ChatMessage, ProviderKind};

fn test_config() -> AgentConfig {
    AgentConfig {
        provider: ProviderKind::OpenAi,
        model: "gpt-4.1".into(),
        api_key: "sk-secret".into(),
        api_base_url: None,
        system_prompt: Some("You are helpful.".into()),
    }
}

#[test]
fn redacted_removes_api_key() {
    let config = test_config();
    let redacted = config.redacted();
    assert!(redacted.api_key.is_empty());
    assert_eq!(redacted.model, "gpt-4.1");
    assert_eq!(redacted.provider, ProviderKind::OpenAi);
    assert_eq!(redacted.system_prompt, config.system_prompt);
    // original untouched
    assert_eq!(config.api_key, "sk-secret");
}

#[test]
fn new_session_has_no_messages() {
    let session = gah_core::Session::new(test_config());
    assert!(session.messages.is_empty());
    assert!(!session.id.is_empty());
    assert_eq!(session.created_at, session.updated_at);
}

#[test]
fn message_roles_round_trip() {
    for role in ["system", "user", "assistant"] {
        let msg = ChatMessage {
            role: role.into(),
            content: format!("hello {role}"),
            tool_calls: None,
            tool_call_id: None,
        };
        let rig = gah_agent::to_rig(&msg).expect("conversion");
        let back = gah_agent::from_rig(&rig);
        assert_eq!(back.role, msg.role);
        assert_eq!(back.content, msg.content);
    }
}

#[test]
fn to_rig_rejects_unknown_role() {
    let msg = ChatMessage {
        role: "robot".into(),
        content: "beep".into(),
        tool_calls: None,
        tool_call_id: None,
    };
    assert!(gah_agent::to_rig(&msg).is_err());
}

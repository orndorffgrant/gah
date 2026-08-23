use gah_core::{AgentConfig, ChatMessage, ProviderKind, Session};

// Each test gets its own in-memory database; a shared pool would race.
// sqlite::memory: with a single connection keeps the database alive for the
// whole test, so cap max_connections at 1.
async fn store() -> gah_api::SqliteSessionStore {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let store = gah_api::SqliteSessionStore::new(pool);
    store.create_tables().await.unwrap();
    store
}

fn session() -> Session {
    Session::new(AgentConfig {
        provider: ProviderKind::Ollama,
        model: "qwen3".into(),
        api_key: String::new(),
        api_base_url: None,
        system_prompt: None,
    })
}

#[tokio::test]
async fn create_get_delete_round_trip() {
    let store = store().await;
    let s = session();
    store.create(&s).await.unwrap();

    let got = store.get(&s.id).await.unwrap().expect("session exists");
    assert_eq!(got.id, s.id);
    assert_eq!(got.config.model, "qwen3");
    assert!(got.messages.is_empty());

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: "hi".into(),
        tool_calls: None,
        tool_call_id: None,
    }];
    store.update_messages(&s.id, &messages).await.unwrap();

    let got = store.get(&s.id).await.unwrap().unwrap();
    assert_eq!(got.messages.len(), 1);
    assert_eq!(got.messages[0].content, "hi");

    assert!(store.delete(&s.id).await.unwrap());
    assert!(store.get(&s.id).await.unwrap().is_none());
    assert!(!store.delete(&s.id).await.unwrap());
}

#[tokio::test]
async fn list_orders_by_updated_desc() {
    let store = store().await;
    let a = session();
    let b = session();
    store.create(&a).await.unwrap();
    store.create(&b).await.unwrap();

    // touch a so it is most recently updated
    store.update_messages(&a.id, &[]).await.unwrap();

    let list = store.list().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, a.id);
    assert_eq!(list[1].id, b.id);
}

#[tokio::test]
async fn token_store_create_and_validate() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let store = gah_api::SqliteTokenStore::new(pool);
    store.create_table().await.unwrap();

    let created = store.create_token("test").await.unwrap();
    assert!(!created.token.is_empty());

    assert!(store.validate(&created.token).await.unwrap());
    assert!(!store.validate("wrong-token").await.unwrap());
    assert!(!store.validate("").await.unwrap());
}

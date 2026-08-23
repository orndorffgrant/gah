use gah_ui::SessionStore;

async fn store() -> SessionStore {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let store = SessionStore::new(pool);
    store.create_tables().await.unwrap();
    store
}

#[tokio::test]
async fn verify_login_and_change_password() {
    let store = store().await;
    store.create_user("alice", "password123", "creator").await.unwrap();

    // correct credentials
    let (uid, role) = store
        .verify_login("alice", "password123")
        .await
        .unwrap()
        .expect("login should succeed");
    assert_eq!(role, "creator");

    // wrong password
    assert!(store.verify_login("alice", "wrong").await.unwrap().is_none());
    // unknown user
    assert!(store.verify_login("bob", "password123").await.unwrap().is_none());

    // change password: wrong current fails
    assert!(!store
        .change_password(uid, "wrong", "newpassword1")
        .await
        .unwrap());
    // right current succeeds
    assert!(store
        .change_password(uid, "password123", "newpassword1")
        .await
        .unwrap());
    // old password no longer works
    assert!(store.verify_login("alice", "password123").await.unwrap().is_none());
    assert!(store
        .verify_login("alice", "newpassword1")
        .await
        .unwrap()
        .is_some());
    let _ = uid;
}

#[tokio::test]
async fn session_lifecycle() {
    let store = store().await;
    store.create_user("alice", "password123", "creator").await.unwrap();

    let sid = store
        .create_session(1, "alice", "creator")
        .await
        .unwrap();
    assert!(!sid.is_empty());

    let session = store
        .get_session(&sid)
        .await
        .unwrap()
        .expect("session should be valid");
    assert_eq!(session.username, "alice");
    assert_eq!(session.role, "creator");
    assert_eq!(session.user_id, 1);

    store.delete_session(&sid).await.unwrap();
    assert!(store.get_session(&sid).await.unwrap().is_none());

    // unknown session id
    assert!(store.get_session("garbage").await.unwrap().is_none());
}

#[tokio::test]
async fn user_management() {
    let store = store().await;
    store.create_user("alice", "password123", "creator").await.unwrap();
    store.create_user("bob", "password123", "admin").await.unwrap();

    let users = store.list_users().await.unwrap();
    assert_eq!(users.len(), 2);
    assert_eq!(users[0].1, "alice"); // ordered by username

    let alice_id = users[0].0;
    assert!(store.delete_user(alice_id).await.unwrap());
    assert!(!store.delete_user(alice_id).await.unwrap());
    assert_eq!(store.list_users().await.unwrap().len(), 1);
}

#[tokio::test]
async fn duplicate_username_rejected() {
    let store = store().await;
    store.create_user("alice", "password123", "creator").await.unwrap();
    assert!(store.create_user("alice", "other", "creator").await.is_err());
}

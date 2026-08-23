//! End-to-end test of the HTTP API: auth, session CRUD, redaction.

use serde_json::json;

async fn spawn_server() -> (String, String, dropshot::HttpServer<std::sync::Arc<gah_api::ApiState>>)
{
    let state = gah_api::run::build_state("sqlite::memory:")
        .await
        .unwrap_or_else(|e| panic!("build_state: {e}"));

    // Bypass the chicken-and-egg of the token endpoint requiring a token:
    // mint one directly through the store, like `gah create-token` does.
    let token = state
        .token_store
        .create_token("test")
        .await
        .unwrap()
        .token;

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
    let server = gah_api::run::start(addr, state)
        .await
        .unwrap_or_else(|e| panic!("start: {e}"));
    (format!("http://{}", server.local_addr()), token, server)
}

#[tokio::test]
async fn auth_and_session_crud() {
    let (base, token, server) = spawn_server().await;
    let client = reqwest::Client::new();

    // No auth -> 401
    let resp = client.get(format!("{base}/v1/sessions")).send().await.unwrap();
    assert_eq!(resp.status(), 401);

    // Bad token -> 401
    let resp = client
        .get(format!("{base}/v1/sessions"))
        .bearer_auth("nope")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Create a session
    let resp = client
        .post(format!("{base}/v1/sessions"))
        .bearer_auth(&token)
        .json(&json!({
            "config": {
                "provider": "ollama",
                "model": "qwen3",
                "api_key": "super-secret",
                "system_prompt": "be brief"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let created: serde_json::Value = resp.json().await.unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["message_count"], 0);

    // List contains it
    let resp = client
        .get(format!("{base}/v1/sessions"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let list: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(list["sessions"].as_array().unwrap().len(), 1);

    // Get redacts the api key
    let resp = client
        .get(format!("{base}/v1/sessions/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let got: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(got["config"]["api_key"], "");
    assert_eq!(got["config"]["model"], "qwen3");
    assert_eq!(got["config"]["system_prompt"], "be brief");

    // Unknown id -> 404
    let resp = client
        .get(format!("{base}/v1/sessions/does-not-exist"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Delete -> 204, then 404
    let resp = client
        .delete(format!("{base}/v1/sessions/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    let resp = client
        .get(format!("{base}/v1/sessions/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    server.close().await.unwrap();
}

#[tokio::test]
async fn token_endpoint_rejects_bad_bearer() {
    let (base, _token, server) = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/tokens"))
        .bearer_auth("wrong")
        .json(&json!({"label": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    server.close().await.unwrap();
}

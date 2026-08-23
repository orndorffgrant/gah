use askama::Template;
use askama_web::WebTemplate;
use gah_api_client::types::{ChatMessage, SessionResponse};

#[derive(Template, WebTemplate)]
#[template(path = "login.html")]
pub struct LoginPage {
    pub html_title: String,
    pub username: Option<String>,
    pub role: Option<String>,
    pub error: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "sessions/list.html")]
pub struct SessionListPage {
    pub html_title: String,
    pub username: Option<String>,
    pub role: Option<String>,
    pub sessions: Vec<SessionResponse>,
    pub error: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "sessions/new.html")]
pub struct NewSessionPage {
    pub html_title: String,
    pub username: Option<String>,
    pub role: Option<String>,
    pub error: Option<String>,
    pub provider: String,
    pub model: String,
    pub api_base_url: String,
    pub system_prompt: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "sessions/chat.html")]
pub struct ChatPage {
    pub html_title: String,
    pub session_id: String,
    pub messages: Vec<ChatMessage>,
    pub error: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "admin.html")]
pub struct AdminPage {
    pub html_title: String,
    pub username: Option<String>,
    pub role: Option<String>,
    pub users: Vec<UserView>,
    pub error: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "settings.html")]
pub struct SettingsPage {
    pub html_title: String,
    pub username: Option<String>,
    pub role: Option<String>,
    pub saved: bool,
    pub error: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "404.html")]
pub struct NotFoundPage {
    pub html_title: String,
    pub username: Option<String>,
    pub role: Option<String>,
}

pub struct UserView {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub created_at: String,
}

//! gah-agent: the rig-based agent loop for GAH.
//!
//! This is the only crate that touches rig. It translates between gah-core's
//! portable wire types and rig's provider-facing types, which keeps the
//! schemars 0.8 (dropshot) / 1.x (rig) version conflict out of the API crate.
//!
//! ponytail: history conversion is still text-only (tool calls and results are
//! flattened to text when persisted); keep structured tool history if a
//! provider ever needs the original tool-call round-trip replayed.

use futures::{Stream, StreamExt};
use gah_core::{AgentConfig, AgentEvent, ChatMessage, ProviderKind, UsageInfo};
use rig::agent::{Agent, AgentBuilder, ModelHandle, MultiTurnStreamItem, PromptResponse};
use rig::client::CompletionClient;
use rig::completion::message::{ToolResult, ToolResultContent, UserContent};
use rig::completion::{AssistantContent, Message as RigMessage};
use rig::prelude::StreamingChat;
use rig::providers::{anthropic, ollama, openai, openrouter};
use rig::streaming::{StreamedAssistantContent, StreamedUserContent};
use thiserror::Error;

pub mod tools;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("{0}")]
    Other(String),
}

fn other<E: std::fmt::Display>(e: E) -> AgentError {
    AgentError::Other(e.to_string())
}

/// Build a rig agent from a portable agent config.
pub fn build_agent(config: &AgentConfig) -> Result<Agent, AgentError> {
    let builder = AgentBuilder::new(model_handle_for(config)?)
        .default_max_turns(10);
    let builder = match &config.system_prompt {
        Some(p) if !p.is_empty() => builder.preamble(p),
        _ => builder,
    };
    Ok(builder.tool(tools::BashTool).build())
}

fn model_handle_for(config: &AgentConfig) -> Result<ModelHandle, AgentError> {
    let model = config.model.as_str();
    match config.provider {
        ProviderKind::OpenAi => {
            let client = openai::Client::new(config.api_key.clone()).map_err(other)?;
            Ok(ModelHandle::new(client.completion_model(model)))
        }
        ProviderKind::Anthropic => {
            let client = anthropic::Client::new(config.api_key.clone()).map_err(other)?;
            Ok(ModelHandle::new(client.completion_model(model)))
        }
        ProviderKind::OpenRouter => {
            // Empty key falls back to the server's $OPENROUTER_API_KEY.
            let key = if config.api_key.trim().is_empty() {
                std::env::var("OPENROUTER_API_KEY").map_err(|_| {
                    AgentError::Other(
                        "no OpenRouter API key: set one in the session or $OPENROUTER_API_KEY"
                            .into(),
                    )
                })?
            } else {
                config.api_key.clone()
            };
            let client = openrouter::Client::new(key).map_err(other)?;
            Ok(ModelHandle::new(client.completion_model(model)))
        }
        ProviderKind::Ollama => {
            // Empty key maps to OllamaApiKey::None (no auth header).
            let builder = ollama::Client::builder().api_key(config.api_key.clone());
            let builder = match &config.api_base_url {
                Some(url) => builder.base_url(url),
                None => builder,
            };
            let client = builder.build().map_err(other)?;
            Ok(ModelHandle::new(client.completion_model(model)))
        }
        // ponytail: Custom assumes an OpenAI-compatible endpoint; add
        // per-provider compat quirks if a target needs them.
        ProviderKind::Custom => {
            let base = config
                .api_base_url
                .as_deref()
                .ok_or_else(|| AgentError::Other("custom provider requires api_base_url".into()))?;
            let client = openai::Client::builder()
                .api_key(config.api_key.clone())
                .base_url(base)
                .build()
                .map_err(other)?;
            Ok(ModelHandle::new(client.completion_model(model)))
        }
    }
}

/// The result of a completed agent run.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub output: String,
    pub usage: UsageInfo,
    /// New messages produced by this run (the prompt and every assistant
    /// turn), to be appended to the session's existing history.
    pub messages: Vec<ChatMessage>,
}

/// Run a prompt to completion, returning the final output plus the new
/// messages the run produced (the prompt and every assistant turn).
pub async fn run_blocking(
    config: &AgentConfig,
    prompt: &str,
    history: &[ChatMessage],
) -> Result<RunOutcome, AgentError> {
    let agent = build_agent(config)?;
    let rig_history: Vec<RigMessage> = history.iter().map(to_rig).collect::<Result<_, _>>()?;
    let response = agent
        .stream_chat(prompt, rig_history)
        .await
        .filter_map(|item| {
            std::future::ready(match item {
                Ok(MultiTurnStreamItem::FinalResponse(resp)) => Some(Ok(resp)),
                Ok(_) => None,
                Err(e) => Some(Err(other(e))),
            })
        })
        .next()
        .await
        .transpose()?
        .ok_or_else(|| AgentError::Other("stream ended without a final response".into()))?;

    Ok(RunOutcome {
        output: response.output.clone(),
        usage: usage_from_response(&response),
        messages: messages_from_response(&response),
    })
}

/// Stream a prompt as agent events. The final event is
/// [`AgentEvent::Done`] carrying the new messages for persistence.
pub async fn run_stream(
    config: &AgentConfig,
    prompt: &str,
    history: &[ChatMessage],
) -> Result<impl Stream<Item = AgentEvent> + Send, AgentError> {
    let agent = build_agent(config)?;
    let rig_history: Vec<RigMessage> = history.iter().map(to_rig).collect::<Result<_, _>>()?;
    let stream = agent.stream_chat(prompt, rig_history).await;
    Ok(stream.filter_map(|item| {
        std::future::ready(match item {
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::Text(t),
            )) => Some(AgentEvent::TextDelta { text: t.text }),
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::ToolCall { tool_call, .. },
            )) => Some(AgentEvent::ToolCall {
                name: tool_call.function.name.clone(),
                arguments: tool_call.function.arguments.clone(),
                id: tool_call.id.to_string(),
            }),
            Ok(MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult {
                tool_result,
                ..
            })) => Some(AgentEvent::ToolResult {
                id: tool_result.call.to_string(),
                content: tool_result_text(&tool_result),
            }),
            Ok(MultiTurnStreamItem::FinalResponse(resp)) => {
                Some(AgentEvent::Done {
                    output: resp.output.clone(),
                    usage: usage_from_response(&resp),
                    messages: messages_from_response(&resp),
                })
            }
            Ok(_) => None,
            Err(e) => Some(AgentEvent::Error {
                message: e.to_string(),
            }),
        })
    }))
}

fn usage_from_response(resp: &PromptResponse) -> UsageInfo {
    UsageInfo {
        input_tokens: resp.usage.input_tokens,
        output_tokens: resp.usage.output_tokens,
    }
}

fn messages_from_response(resp: &PromptResponse) -> Vec<ChatMessage> {
    resp.messages
        .clone()
        .unwrap_or_default()
        .iter()
        .map(from_rig)
        .collect()
}

fn tool_result_text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match c {
            ToolResultContent::Text(t) => Some(t.text.clone()),
            ToolResultContent::Json { value } => Some(value.to_string()),
            ToolResultContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Convert a portable chat message into a rig message.
pub fn to_rig(msg: &ChatMessage) -> Result<RigMessage, AgentError> {
    match msg.role.as_str() {
        "system" => Ok(RigMessage::System {
            content: msg.content.clone(),
        }),
        "user" => Ok(RigMessage::User {
            content: vec![UserContent::text(msg.content.clone())],
        }),
        "assistant" => Ok(RigMessage::Assistant {
            id: None,
            content: vec![AssistantContent::text(msg.content.clone())],
        }),
        role => Err(AgentError::Other(format!("unsupported message role: {role}"))),
    }
}

/// Convert a rig message into a portable chat message.
pub fn from_rig(msg: &RigMessage) -> ChatMessage {
    match msg {
        RigMessage::System { content } => ChatMessage {
            role: "system".into(),
            content: content.clone(),
            tool_calls: None,
            tool_call_id: None,
        },
        RigMessage::User { content } => {
            let text = content
                .iter()
                .map(|c| match c {
                    UserContent::Text(t) => t.text.clone(),
                    UserContent::ToolResult(r) => tool_result_text(r),
                    _ => String::new(),
                })
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            ChatMessage {
                role: "user".into(),
                content: text,
                tool_calls: None,
                tool_call_id: None,
            }
        }
        RigMessage::Assistant { content, .. } => {
            let mut text = String::new();
            let mut tool_calls = Vec::new();
            for c in content {
                match c {
                    AssistantContent::Text(t) => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&t.text);
                    }
                    AssistantContent::ToolCall(call) => {
                        tool_calls.push(serde_json::json!({
                            "id": call.id.to_string(),
                            "name": call.function.name,
                            "arguments": call.function.arguments,
                        }));
                    }
                    _ => {}
                }
            }
            ChatMessage {
                role: "assistant".into(),
                content: text,
                tool_calls: (!tool_calls.is_empty()).then(|| serde_json::Value::Array(tool_calls)),
                tool_call_id: None,
            }
        }
    }
}

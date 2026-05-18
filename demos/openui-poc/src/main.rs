use axum::{
    Router,
    body::Body,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
};
use claude_wrapper::{Claude, OutputFormat, QueryCommand};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{env, path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::{cors::CorsLayer, services::ServeDir};

struct AppState {
    http: Client,
    system_prompt: String,
    mode: AiMode,
}

enum AiMode {
    /// Use the local `claude` CLI binary (no API key needed).
    ClaudeCli { binary: PathBuf },
    /// Proxy to any OpenAI-compatible API.
    OpenAiProxy {
        api_key: String,
        base_url: String,
        model: String,
    },
}

#[derive(Deserialize)]
struct ChatRequest {
    messages: Vec<ChatMessage>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
struct ChatMessage {
    role: String,
    content: serde_json::Value,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let prompt_path = env::var("SYSTEM_PROMPT_PATH")
        .unwrap_or_else(|_| "ui/src/generated/system-prompt.txt".into());
    let prompt_file = PathBuf::from(&prompt_path);
    let system_prompt = if prompt_file.exists() {
        std::fs::read_to_string(&prompt_file)
            .unwrap_or_else(|e| panic!("Failed to read {}: {}", prompt_path, e))
    } else {
        tracing::warn!(
            "System prompt file not found at {}, using fallback",
            prompt_path
        );
        include_str!("fallback-prompt.txt").to_string()
    };
    tracing::info!("System prompt loaded ({} chars)", system_prompt.len());

    // Decide which AI backend to use:
    // - If OPENAI_API_KEY is set, use the OpenAI proxy path.
    // - Otherwise, try to find the local `claude` CLI binary.
    let mode = if let Ok(api_key) = env::var("OPENAI_API_KEY") {
        let base_url =
            env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into());
        let model = env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
        tracing::info!("Mode: OpenAI proxy -> {} (model: {})", base_url, model);
        AiMode::OpenAiProxy {
            api_key,
            base_url,
            model,
        }
    } else {
        let binary = discover_claude_binary().expect(
            "No OPENAI_API_KEY set and no `claude` binary found. \
             Either set OPENAI_API_KEY or install Claude Code CLI.",
        );
        tracing::info!("Mode: Claude CLI -> {}", binary.display());
        AiMode::ClaudeCli { binary }
    };

    let state = Arc::new(AppState {
        http: Client::new(),
        system_prompt,
        mode,
    });

    let ui_dir = PathBuf::from("ui/dist");
    let serve_ui = if ui_dir.exists() {
        tracing::info!("Serving UI from ui/dist/");
        ServeDir::new(ui_dir).append_index_html_on_directories(true)
    } else {
        tracing::warn!("ui/dist/ not found; run 'cd ui && npm run build' first");
        ServeDir::new("ui/dist").append_index_html_on_directories(true)
    };

    let app = Router::new()
        .route("/api/chat", post(chat_handler))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .fallback_service(serve_ui);

    let addr = "0.0.0.0:3001";
    tracing::info!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// ---------------------------------------------------------------------------
// Unified handler — dispatches to Claude CLI or OpenAI proxy
// ---------------------------------------------------------------------------

async fn chat_handler(
    State(state): State<Arc<AppState>>,
    axum::Json(payload): axum::Json<ChatRequest>,
) -> impl IntoResponse {
    match &state.mode {
        AiMode::ClaudeCli { binary } => {
            handle_claude_cli(binary.clone(), &state.system_prompt, payload).await
        }
        AiMode::OpenAiProxy {
            api_key,
            base_url,
            model,
        } => {
            handle_openai_proxy(&state.http, api_key, base_url, model, &state.system_prompt, payload)
                .await
        }
    }
}

// ---------------------------------------------------------------------------
// Claude CLI path — uses claude-wrapper, converts events to OpenAI SSE format
// ---------------------------------------------------------------------------

async fn handle_claude_cli(
    binary: PathBuf,
    system_prompt: &str,
    payload: ChatRequest,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let claude = match Claude::builder().binary(binary).build() {
        Ok(c) => Arc::new(c),
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to init Claude CLI: {}", e),
            ));
        }
    };

    // Build the user prompt from the last user message in the conversation.
    let user_prompt = payload
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| match &m.content {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();

    let cmd = QueryCommand::new(&user_prompt)
        .output_format(OutputFormat::StreamJson)
        .system_prompt(system_prompt);

    let (tx, rx) = mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(64);
    let chat_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        let chat_id_inner = chat_id;

        rt.block_on(async {
            let tx2 = tx.clone();
            let chat_id2 = chat_id_inner.clone();

            let result = claude_wrapper::streaming::stream_query(&claude, &cmd, |ev| {
                let etype = ev.event_type().unwrap_or("");
                match etype {
                    "assistant" => {
                        if let Some(blocks) = ev.data["message"]["content"].as_array() {
                            for block in blocks {
                                if block["type"].as_str() == Some("text") {
                                    let text = block["text"].as_str().unwrap_or("");
                                    if text.is_empty() {
                                        continue;
                                    }
                                    // Convert to OpenAI SSE chunk format
                                    let chunk = serde_json::json!({
                                        "id": chat_id2,
                                        "object": "chat.completion.chunk",
                                        "choices": [{
                                            "index": 0,
                                            "delta": { "content": text },
                                            "finish_reason": null
                                        }]
                                    });
                                    let sse_line =
                                        format!("data: {}\n\n", serde_json::to_string(&chunk).unwrap());
                                    let _ = tx2.try_send(Ok(bytes::Bytes::from(sse_line)));
                                }
                            }
                        }
                    }
                    "result" => {
                        // Send the final stop chunk
                        let stop_chunk = serde_json::json!({
                            "id": chat_id2,
                            "object": "chat.completion.chunk",
                            "choices": [{
                                "index": 0,
                                "delta": {},
                                "finish_reason": "stop"
                            }]
                        });
                        let sse_line =
                            format!("data: {}\n\n", serde_json::to_string(&stop_chunk).unwrap());
                        let _ = tx2.try_send(Ok(bytes::Bytes::from(sse_line)));
                        let _ = tx2.try_send(Ok(bytes::Bytes::from("data: [DONE]\n\n")));
                    }
                    _ => {}
                }
            })
            .await;

            if let Err(e) = result {
                let err_chunk = format!(
                    "data: {}\n\n",
                    serde_json::json!({"error": e.to_string()})
                );
                let _ = tx.try_send(Ok(bytes::Bytes::from(err_chunk)));
            }
        });
    });

    let body = Body::from_stream(ReceiverStream::new(rx));
    let response = axum::response::Response::builder()
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache, no-transform")
        .header("Connection", "keep-alive")
        .body(body)
        .unwrap();
    Ok(response)
}

// ---------------------------------------------------------------------------
// OpenAI proxy path — passthrough SSE from any OpenAI-compatible API
// ---------------------------------------------------------------------------

async fn handle_openai_proxy(
    http: &Client,
    api_key: &str,
    base_url: &str,
    model: &str,
    system_prompt: &str,
    payload: ChatRequest,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let mut messages = vec![serde_json::json!({
        "role": "system",
        "content": system_prompt,
    })];
    for msg in &payload.messages {
        messages.push(serde_json::json!({
            "role": msg.role,
            "content": msg.content,
        }));
    }

    let url = format!("{}/chat/completions", base_url);

    let resp = match http
        .post(&url)
        .bearer_auth(api_key)
        .json(&serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": true,
        }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("OpenAI request failed: {}", e);
            return Err((
                StatusCode::BAD_GATEWAY,
                format!("Upstream request failed: {}", e),
            ));
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::error!("OpenAI returned {}: {}", status, body);
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("Upstream error {}: {}", status, body),
        ));
    }

    let body = Body::from_stream(resp.bytes_stream());
    let response = axum::response::Response::builder()
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache, no-transform")
        .header("Connection", "keep-alive")
        .body(body)
        .unwrap();
    Ok(response)
}

// ---------------------------------------------------------------------------
// Claude binary discovery (from ai-runner/src/runners/claude.rs)
// ---------------------------------------------------------------------------

fn discover_claude_binary() -> Option<PathBuf> {
    if let Ok(v) = env::var("CLAUDE_BINARY") {
        let v = v.trim();
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }

    if let Some(p) = find_on_path("claude") {
        return Some(p);
    }

    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        let candidates = [
            home.join(".local/bin/claude"),
            home.join(".bun/bin/claude"),
            home.join(".npm-global/bin/claude"),
            home.join(".config/npm/global/bin/claude"),
        ];
        for c in &candidates {
            if c.is_file() {
                return Some(c.clone());
            }
        }
        // Editor-shipped copies (VS Code / Cursor)
        for root in [
            home.join(".vscode/extensions"),
            home.join(".vscode-server/extensions"),
            home.join(".cursor/extensions"),
        ] {
            if let Some(p) = scan_vscode_extensions(&root) {
                return Some(p);
            }
        }
    }
    for sys in ["/opt/homebrew/bin/claude", "/usr/local/bin/claude"] {
        let p = PathBuf::from(sys);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let full = dir.join(name);
        if is_executable_file(&full) {
            return Some(full);
        }
    }
    None
}

fn is_executable_file(p: &std::path::Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn scan_vscode_extensions(root: &std::path::Path) -> Option<PathBuf> {
    let rd = std::fs::read_dir(root).ok()?;
    let mut best: Option<(String, PathBuf)> = None;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("anthropic.claude-code-") {
            continue;
        }
        let bin = entry.path().join("resources/native-binary/claude");
        if !is_executable_file(&bin) {
            continue;
        }
        if best.as_ref().map(|(n, _)| name > *n).unwrap_or(true) {
            best = Some((name, bin));
        }
    }
    best.map(|(_, p)| p)
}

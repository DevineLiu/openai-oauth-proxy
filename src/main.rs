mod auth;
mod cli;
mod pkce;
mod token;

pub use token::{auth_file_path, load_openai_browser_access_token};

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderName, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Json, Router,
};
use base64::Engine;
use clap::Parser;
use reqwest::Method;
use serde_json::Value;
use std::env;
use std::net::SocketAddr;
use std::time::Duration;
use std::time::Instant;
use tower_http::trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::{debug, error, Level};
use tracing_subscriber::EnvFilter;

const OPENAI_PROXY_UPSTREAM: &str = "https://chatgpt.com/backend-api";
const TOKEN_EXPIRY_SKEW_SECS: u64 = 60;

fn oauth_responses_path(upstream_base: &str) -> &'static str {
    if upstream_base
        .trim_end_matches('/')
        .ends_with("/backend-api")
    {
        "/codex/responses"
    } else {
        "/backend-api/codex/responses"
    }
}

fn read_env_token(key: &str) -> Option<String> {
    let value = env::var(key).ok()?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn current_unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn jwt_exp_unix(token: &str) -> Option<u64> {
    let payload_b64 = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let payload = serde_json::from_slice::<Value>(&bytes).ok()?;
    payload.get("exp").and_then(Value::as_u64)
}

fn is_expired_jwt(token: &str) -> bool {
    let now = current_unix_time();
    jwt_exp_unix(token)
        .map(|exp| exp <= now + TOKEN_EXPIRY_SKEW_SECS)
        .unwrap_or(false)
}

fn is_oauth_upstream(base: &str) -> bool {
    base.contains("chatgpt.com")
}

fn is_proxy_placeholder(value: &str) -> bool {
    let v = value.trim();
    v.is_empty() || v.eq_ignore_ascii_case("proxy")
}

fn load_bearer_token(upstream_base: &str) -> Result<String, String> {
    if let Some(v) = read_env_token("OPENAI_PROXY_BEARER_TOKEN") {
        debug_log("token source: OPENAI_PROXY_BEARER_TOKEN");
        return Ok(v);
    }

    if is_oauth_upstream(upstream_base) {
        let oauth_env = read_env_token("OPENAI_OAUTH_TOKEN");
        if let Some(v) = oauth_env.as_deref() {
            if !is_expired_jwt(v) {
                debug_log("token source: OPENAI_OAUTH_TOKEN");
                return Ok(v.to_string());
            }
            debug_log("OPENAI_OAUTH_TOKEN appears expired");
        }
        if let Some(token) = load_openai_browser_access_token() {
            debug_log("token source: local auth file (auto-refreshed if needed)");
            return Ok(token);
        }
        if oauth_env.as_deref().is_some_and(is_expired_jwt) {
            return Err("OPENAI_OAUTH_TOKEN appears expired and local auth file could not refresh. Run: cargo run -- auth"
                .to_string());
        }
        if let Some(v) = read_env_token("OPENAI_API_KEY") {
            if !is_proxy_placeholder(&v) {
                debug_log("token source: OPENAI_API_KEY (oauth upstream)");
                return Ok(v);
            }
            debug_log("OPENAI_API_KEY is placeholder (proxy/empty), fallback to oauth token logic");
        }
        return Err(
            "Missing browser bearer token. Set OPENAI_OAUTH_TOKEN or run: cargo run -- auth"
                .to_string(),
        );
    }

    if let Some(v) = read_env_token("OPENAI_API_KEY") {
        if !is_proxy_placeholder(&v) {
            debug_log("token source: OPENAI_API_KEY");
            return Ok(v);
        }
        debug_log("OPENAI_API_KEY is placeholder (proxy/empty)");
    }
    if let Some(v) = read_env_token("OPENAI_OAUTH_TOKEN") {
        debug_log("token source: OPENAI_OAUTH_TOKEN");
        return Ok(v);
    }
    if let Some(token) = load_openai_browser_access_token() {
        debug_log("token source: local auth file (auto-refreshed if needed)");
        return Ok(token);
    }
    Err("Missing bearer token. Set OPENAI_PROXY_BEARER_TOKEN, OPENAI_API_KEY, OPENAI_OAUTH_TOKEN or run: cargo run -- auth"
        .to_string())
}

fn extract_chatgpt_account_id(token: &str) -> Option<String> {
    let payload_b64 = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn openai_messages_to_input_text(messages: &[Value]) -> String {
    messages
        .iter()
        .filter_map(|m| {
            let role = m.get("role")?.as_str()?;
            let content = match m.get("content") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str).map(String::from))
                    .collect::<Vec<_>>()
                    .join(""),
                _ => String::new(),
            };
            Some(format!("[{}] {}", role, content))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct CodexTransformResult {
    body: Vec<u8>,
    client_stream: bool,
    requested_model: String,
}

fn openai_to_codex_request(body: &[u8]) -> Result<CodexTransformResult, String> {
    let req: Value = serde_json::from_slice(body).map_err(|e| format!("invalid json: {}", e))?;
    let messages = req
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing messages".to_string())?;
    let input_text = openai_messages_to_input_text(messages);
    let model = req
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("gpt-5.2-codex");
    let model = if !model.to_lowercase().contains("codex") {
        "gpt-5.2-codex"
    } else {
        model
    };
    let client_stream = req.get("stream").and_then(Value::as_bool).unwrap_or(false);

    let codex_body = serde_json::json!({
        "model": model,
        "input": [{
            "role": "user",
            "content": [{ "type": "input_text", "text": input_text }]
        }],
        "stream": true,
        "store": false,
        "instructions": "You are a helpful coding assistant. Answer directly and clearly."
    });

    let body = serde_json::to_vec(&codex_body).map_err(|e| format!("serialize: {}", e))?;
    Ok(CodexTransformResult {
        body,
        client_stream,
        requested_model: model.to_string(),
    })
}

fn extract_browser_text(payload: &Value) -> Option<String> {
    if let Some(text) = payload.get("output_text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    let output = payload.get("output")?.as_array()?;
    let mut chunks = Vec::new();
    for item in output {
        let content = item.get("content").and_then(Value::as_array);
        if let Some(content) = content {
            for block in content {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    chunks.push(text.to_string());
                }
            }
        }
    }
    if chunks.is_empty() {
        None
    } else {
        Some(chunks.join("\n"))
    }
}

fn parse_codex_sse_text(raw: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("data:") {
            continue;
        }
        let data = trimmed.trim_start_matches("data:").trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let value: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(text) = value.get("output_text").and_then(Value::as_str) {
            chunks.push(text.to_string());
            continue;
        }
        if let Some(delta) = value.get("delta").and_then(Value::as_str) {
            if !delta.is_empty() {
                chunks.push(delta.to_string());
            }
            continue;
        }
        if let Some(text) = extract_browser_text(&value) {
            if !text.is_empty() {
                chunks.push(text);
            }
        }
    }
    chunks
}

fn codex_sse_to_openai_sse(raw: &str) -> String {
    let chunks = parse_codex_sse_text(raw);
    let mut out = String::new();
    for (i, text) in chunks.iter().enumerate() {
        if text.is_empty() {
            continue;
        }
        let chunk = serde_json::json!({
            "id": format!("chatcmpl-{}", i + 1),
            "choices": [{"index": 0, "delta": {"content": text}, "finish_reason": null}],
        });
        out.push_str(&format!("data: {}\n\n", chunk));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

fn codex_json_to_openai_chat(raw: &str, requested_model: &str) -> Vec<u8> {
    let value = serde_json::from_str::<Value>(raw).ok();
    let assistant_text = if let Some(v) = value.as_ref() {
        extract_browser_text(v).unwrap_or_default()
    } else {
        parse_codex_sse_text(raw).join("")
    };
    let id = value
        .as_ref()
        .and_then(|v| v.get("id").and_then(Value::as_str))
        .unwrap_or("chatcmpl-proxy")
        .to_string();
    let model = value
        .as_ref()
        .and_then(|v| v.get("model").and_then(Value::as_str))
        .unwrap_or(requested_model)
        .to_string();

    let payload = serde_json::json!({
        "id": id,
        "object": "chat.completion",
        "created": current_unix_time(),
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": assistant_text,
            },
            "finish_reason": "stop"
        }]
    });
    serde_json::to_vec(&payload).unwrap_or_else(|_| {
        b"{\"id\":\"chatcmpl-proxy\",\"object\":\"chat.completion\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":\"stop\"}]}".to_vec()
    })
}

fn upstream_error_to_openai(status: u16, body: &[u8]) -> String {
    let raw = String::from_utf8_lossy(body);
    let msg = if let Ok(v) = serde_json::from_str::<Value>(&raw) {
        v.get("error")
            .and_then(|e| e.get("message").and_then(Value::as_str))
            .or_else(|| v.get("message").and_then(Value::as_str))
            .map(String::from)
            .unwrap_or_else(|| {
                let s: String = raw.chars().take(500).collect();
                if s.is_empty() {
                    format!("upstream error {}", status)
                } else {
                    s
                }
            })
    } else {
        let s: String = raw.chars().take(500).collect();
        if s.is_empty() {
            format!("upstream error {}", status)
        } else {
            format!("upstream {}: {}", status, s)
        }
    };

    serde_json::json!({
        "error": {
            "message": msg,
            "type": "invalid_request_error",
            "code": null,
        }
    })
    .to_string()
}

fn openai_error_value(message: impl Into<String>) -> Value {
    serde_json::json!({
        "error": {
            "message": message.into(),
            "type": "invalid_request_error",
            "code": null,
        }
    })
}

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    upstream_base: String,
}

async fn proxy_v1(
    State(state): State<AppState>,
    req: Request,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let raw_path = req.uri().path().to_string();
    let path = if raw_path == "/chat/completions" {
        "/v1/chat/completions".to_string()
    } else {
        raw_path
    };
    let query = req.uri().query().map(String::from);
    let method = req.method().clone();
    let incoming_headers = req.headers().clone();

    debug_log(&format!("incoming: method={} path={}", method, path));

    let body = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(openai_error_value("failed to read body")),
            )
        })?;

    let token = load_bearer_token(&state.upstream_base)
        .map_err(|e| (StatusCode::UNAUTHORIZED, Json(openai_error_value(e))))?;

    let is_oauth = is_oauth_upstream(&state.upstream_base);
    let is_chat_completions = path == "/v1/chat/completions";
    let mut client_stream = false;
    let mut requested_model = String::new();

    let (upstream_path, body_bytes) = if is_oauth && is_chat_completions {
        let transformed = openai_to_codex_request(&body)
            .map_err(|e| (StatusCode::BAD_REQUEST, Json(openai_error_value(e))))?;
        client_stream = transformed.client_stream;
        requested_model = transformed.requested_model;
        let p = oauth_responses_path(&state.upstream_base).to_string();
        debug_log(&format!(
            "transform: chat/completions -> {} model={} client_stream={}",
            p, requested_model, client_stream
        ));
        (p, transformed.body)
    } else {
        (path.clone(), body.to_vec())
    };

    let mut url = format!(
        "{}{}",
        state.upstream_base.trim_end_matches('/'),
        upstream_path
    );
    if let Some(q) = query {
        if !q.is_empty() {
            url.push('?');
            url.push_str(&q);
        }
    }
    debug_log(&format!("upstream request target={}", url));

    let reqwest_method = Method::from_bytes(method.as_str().as_bytes()).map_err(|_| {
        (
            StatusCode::METHOD_NOT_ALLOWED,
            Json(openai_error_value("unsupported HTTP method")),
        )
    })?;

    let mut builder = state
        .client
        .request(reqwest_method, &url)
        .bearer_auth(&token);

    for (k, v) in &incoming_headers {
        let name = k.as_str();
        if name.eq_ignore_ascii_case("authorization")
            || name.eq_ignore_ascii_case("host")
            || name.eq_ignore_ascii_case("content-length")
        {
            continue;
        }
        if let Ok(vs) = v.to_str() {
            builder = builder.header(name, vs);
        }
    }

    if is_oauth {
        builder = builder
            .header("OpenAI-Beta", "responses=experimental")
            .header("originator", "codex_cli_rs")
            .header("accept", "text/event-stream");
        if let Some(account_id) = extract_chatgpt_account_id(&token) {
            builder = builder.header("chatgpt-account-id", account_id);
        }
    }

    let upstream_response = builder.body(body_bytes).send().await.map_err(|e| {
        error!(
            status = 502,
            method = %method,
            path = %path,
            upstream_url = %url,
            error = %e,
            "upstream send failed"
        );
        (
            StatusCode::BAD_GATEWAY,
            Json(openai_error_value(format!(
                "upstream request failed: {}",
                e
            ))),
        )
    })?;

    let upstream_status_u16 = upstream_response.status().as_u16();
    debug_log(&format!("upstream response status={}", upstream_status_u16));
    let upstream_status =
        StatusCode::from_u16(upstream_status_u16).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let upstream_headers = upstream_response.headers().clone();
    let upstream_body = upstream_response.bytes().await.map_err(|e| {
        error!(
            status = 502,
            method = %method,
            path = %path,
            upstream_url = %url,
            error = %e,
            "upstream body read failed"
        );
        (
            StatusCode::BAD_GATEWAY,
            Json(openai_error_value(format!(
                "failed to read upstream response: {}",
                e
            ))),
        )
    })?;

    if upstream_status_u16 == 502 {
        error!(
            status = 502,
            method = %method,
            path = %path,
            upstream_url = %url,
            body_preview = %String::from_utf8_lossy(&upstream_body).chars().take(300).collect::<String>(),
            "upstream returned 502"
        );
    }

    let out_bytes = if upstream_status.is_success() {
        if is_oauth && client_stream {
            let raw = String::from_utf8_lossy(&upstream_body);
            codex_sse_to_openai_sse(&raw).into_bytes()
        } else if is_oauth && is_chat_completions {
            let raw = String::from_utf8_lossy(&upstream_body);
            codex_json_to_openai_chat(&raw, &requested_model)
        } else {
            upstream_body.to_vec()
        }
    } else {
        debug_log("mapping upstream error to OpenAI error envelope");
        upstream_error_to_openai(upstream_status_u16, &upstream_body).into_bytes()
    };

    let body_len = out_bytes.len();
    let mut response = Response::new(Body::from(out_bytes));
    *response.status_mut() = upstream_status;
    let headers = response.headers_mut();
    const SKIP: [&str; 4] = [
        "content-length",
        "transfer-encoding",
        "connection",
        "content-encoding",
    ];

    if !is_oauth && upstream_status.is_success() {
        for (k, v) in &upstream_headers {
            let name = k.as_str().to_lowercase();
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            if let (Ok(header_name), Ok(header_val)) = (
                HeaderName::try_from(k.as_str()),
                HeaderValue::try_from(v.as_bytes()),
            ) {
                headers.insert(header_name, header_val);
            }
        }
    }

    headers.insert(
        axum::http::header::CONTENT_LENGTH,
        body_len
            .to_string()
            .parse()
            .unwrap_or(HeaderValue::from_static("0")),
    );
    if upstream_status.is_success() && is_oauth && client_stream {
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
    } else if (upstream_status.is_success() && is_oauth) || !upstream_status.is_success() {
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
    }

    Ok(response)
}

async fn handle_request(State(state): State<AppState>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().as_str().to_string();

    if path == "/healthz" && method == "GET" {
        return (StatusCode::OK, "ok").into_response();
    }

    if !path.starts_with("/v1/") && path != "/chat/completions" {
        return (
            StatusCode::NOT_FOUND,
            Json(openai_error_value("only /v1/* proxied")),
        )
            .into_response();
    }

    match proxy_v1(State(state), req).await {
        Ok(r) => r,
        Err((code, json)) => (code, json).into_response(),
    }
}

fn start_proxy_server(host: &str, port: u16) -> Result<(), anyhow::Error> {
    let upstream =
        env::var("OPENAI_PROXY_UPSTREAM").unwrap_or_else(|_| OPENAI_PROXY_UPSTREAM.to_string());
    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid host:port {}:{}: {}", host, port, e))?;

    let client = {
        let mut b = reqwest::Client::builder().timeout(Duration::from_secs(120));
        if std::env::var("OPENAI_OAUTH_NO_PROXY").as_deref() == Ok("1") {
            b = b.no_proxy();
        }
        b.build()
            .map_err(|e| anyhow::anyhow!("failed to build proxy upstream client: {}", e))?
    };

    println!("Proxy listening on http://{}", addr);
    println!("Forwarding to {}", upstream);
    debug_log(&format!(
        "proxy debug enabled; bind={}, upstream={}",
        addr, upstream
    ));
    println!("\nClient configuration:");
    println!("  OPENAI_BASE_URL=http://{}/v1", addr);
    println!("  OPENAI_API_KEY=proxy");
    println!("\nCurl example:");
    println!("  curl -s http://{}/v1/chat/completions \\", addr);
    println!("    -H \"Content-Type: application/json\" \\");
    println!("    -H \"Authorization: Bearer proxy\" \\");
    println!(
        "    -d '{{\"model\":\"gpt-5.2-codex\",\"messages\":[{{\"role\":\"user\",\"content\":\"hello\"}}],\"stream\":false}}'"
    );

    let state = AppState {
        client,
        upstream_base: upstream,
    };
    let app = Router::new()
        .fallback(handle_request)
        .with_state(state)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<Body>| {
                    tracing::debug_span!(
                        "http",
                        method = %request.method(),
                        uri = %request.uri(),
                        version = ?request.version()
                    )
                })
                .on_request(DefaultOnRequest::new().level(Level::DEBUG))
                .on_response(DefaultOnResponse::new().level(Level::DEBUG))
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
        .layer(middleware::from_fn(debug_request_middleware));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build tokio runtime: {}", e))?;

    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| anyhow::anyhow!("bind failed on {}: {}", addr, e))?;
        axum::serve(listener, app)
            .await
            .map_err(|e| anyhow::anyhow!("serve failed: {}", e))
    })
}

fn debug_enabled() -> bool {
    matches!(
        env::var("OPENAI_OAUTH_PROXY_DEBUG").ok().as_deref(),
        Some("1")
            | Some("true")
            | Some("TRUE")
            | Some("yes")
            | Some("YES")
            | Some("on")
            | Some("ON")
    )
}

fn debug_log(message: &str) {
    debug!("{}", message);
}

fn init_tracing() {
    let default_filter = if debug_enabled() {
        "openai_oauth_proxy=debug,tower_http=debug"
    } else {
        "openai_oauth_proxy=info,tower_http=info"
    };
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .try_init();
}

async fn debug_request_middleware(req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let start = Instant::now();

    debug!(
        method = %method,
        path = %path,
        query = %query,
        "middleware request start"
    );

    let response = next.run(req).await;
    let elapsed_ms = start.elapsed().as_millis() as u64;
    let status = response.status().as_u16();
    debug!(
        method = %method,
        path = %path,
        status = status,
        elapsed_ms = elapsed_ms,
        "middleware request end"
    );

    response
}

fn main() {
    let cli = cli::Cli::parse();
    cli::apply_cli_overrides(&cli);
    init_tracing();
    debug_log(&format!(
        "startup: command={:?}, auth_file={}",
        cli.command,
        auth_file_path().display()
    ));

    let (run_auth_command, serve_host, serve_port) = match &cli.command {
        Some(cli::Command::Auth) => (true, "127.0.0.1".to_string(), 8788),
        Some(command) => match command.serve_values() {
            Some((proxy_host, proxy_port)) => (false, proxy_host.to_string(), proxy_port),
            None => (false, "127.0.0.1".to_string(), 8788),
        },
        None => (false, "127.0.0.1".to_string(), 8788),
    };

    let has_explicit_action =
        cli.command.is_some() || cli.print_auth_file || cli.list_models || cli.print_access_token;

    if cli.print_auth_file {
        println!("{}", auth_file_path().display());
        return;
    }

    if cli.list_models {
        let models = auth::list_models();
        if models.is_empty() {
            eprintln!("No built-in models configured.");
            std::process::exit(1);
        }
        for model in models {
            println!("{}", model);
        }
        return;
    }

    if cli.print_access_token {
        match load_openai_browser_access_token() {
            Some(token) => {
                println!("{}", token);
                return;
            }
            None => {
                eprintln!("No valid token found. Run with auth first.");
                std::process::exit(1);
            }
        }
    }

    if run_auth_command {
        if !auth::login_openai_browser() {
            std::process::exit(1);
        }
        return;
    }

    let explicit_serve = cli
        .command
        .as_ref()
        .is_some_and(|command| command.serve_values().is_some());

    if explicit_serve || !has_explicit_action {
        if load_openai_browser_access_token().is_some() {
            println!("Loaded existing token from {}", auth_file_path().display());
            debug_log("default flow: token exists, skip auth and start proxy");
        } else if !auth::login_openai_browser() {
            std::process::exit(1);
        }

        if let Err(e) = start_proxy_server(&serve_host, serve_port) {
            eprintln!("Failed to start proxy: {:#}", e);
            std::process::exit(1);
        }
    }
}

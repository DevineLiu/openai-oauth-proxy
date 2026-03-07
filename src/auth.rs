use crate::pkce;
use crate::token;
use reqwest::blocking::Client as BlockingClient;
use std::env;
use std::error::Error;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tracing::debug;

const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const OPENAI_SCOPE: &str =
    "openid profile email offline_access model.request api.model.read api.responses.write";
const HARD_CODED_MODELS: [&str; 4] = [
    "gpt-4o-mini",
    "gpt-5.1-codex",
    "gpt-5.2-codex",
    "gpt-5.3-codex",
];

fn debug_log(message: &str) {
    debug!("{}", message);
}

fn running_in_container() -> bool {
    std::path::Path::new("/.dockerenv").exists()
        || env::var("container").is_ok()
        || env::var("KUBERNETES_SERVICE_HOST").is_ok()
}

fn browser_open_enabled() -> bool {
    !matches!(
        env::var("OPENAI_OAUTH_NO_BROWSER").ok().as_deref(),
        Some("1")
            | Some("true")
            | Some("TRUE")
            | Some("yes")
            | Some("YES")
            | Some("on")
            | Some("ON")
    )
}

pub fn list_models() -> Vec<String> {
    HARD_CODED_MODELS
        .iter()
        .map(|m| (*m).to_string())
        .collect::<Vec<String>>()
}

pub fn login_openai_browser() -> bool {
    debug_log("oauth login: begin browser flow");
    let redirect_uri =
        env::var("OPENAI_OAUTH_REDIRECT_URI").unwrap_or_else(|_| OPENAI_REDIRECT_URI.to_string());
    let auth_url =
        env::var("OPENAI_OAUTH_AUTH_URL").unwrap_or_else(|_| OPENAI_AUTHORIZE_URL.to_string());
    let token_url =
        env::var("OPENAI_OAUTH_TOKEN_URL").unwrap_or_else(|_| OPENAI_TOKEN_URL.to_string());
    let client_id =
        env::var("OPENAI_OAUTH_CLIENT_ID").unwrap_or_else(|_| OPENAI_CLIENT_ID.to_string());
    let scope = env::var("OPENAI_OAUTH_SCOPE").unwrap_or_else(|_| OPENAI_SCOPE.to_string());

    let (code_verifier, code_challenge) = pkce::generate_pkce_pair();
    let state = pkce::generate_state();

    let authorize_url = build_authorize_url(
        &client_id,
        &redirect_uri,
        &scope,
        &code_challenge,
        &state,
        &auth_url,
    );

    println!("Open browser for login:");
    println!("{}", authorize_url);
    debug_log(&format!(
        "oauth authorize url ready, redirect_uri={}",
        redirect_uri
    ));
    if running_in_container() || !browser_open_enabled() {
        println!(
            "Browser auto-open disabled. Open the URL manually, then paste redirect URL below."
        );
    } else if let Err(e) = open_url(&authorize_url) {
        eprintln!("Warning: failed to open browser: {}", e);
    }

    let result = start_callback_server(&redirect_uri);
    let result = result.or_else(|| prompt_manual_input(&state));

    let (code, got_state) = match result {
        Some(r) => r,
        None => {
            eprintln!("Login cancelled or failed.");
            return false;
        }
    };

    if got_state != state {
        eprintln!("OAuth state mismatch.");
        return false;
    }

    let client = {
        let mut b = BlockingClient::builder().timeout(Duration::from_secs(30));
        if std::env::var("OPENAI_OAUTH_NO_PROXY").as_deref() == Ok("1") {
            b = b.no_proxy();
        }
        match b.build() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Token exchange failed: {}", e);
                return false;
            }
        }
    };

    match exchange_code(
        &client,
        &token_url,
        &client_id,
        &redirect_uri,
        &code,
        &code_verifier,
    ) {
        Ok(token) => {
            if token::save_token(&token).is_err() {
                eprintln!("Failed to save token.");
                return false;
            }
            println!(
                "Login success. Token saved to {}",
                token::auth_file_path().display()
            );
            debug_log("oauth login: token saved");
            true
        }
        Err(e) => {
            eprintln!("Token exchange failed: {:#}", e);
            if e.to_string().contains("Connect") {
                eprintln!("\nTip: auth.openai.com may be unreachable from your network.");
                eprintln!("  - If in a blocked region: set HTTPS_PROXY to a working proxy");
                eprintln!("  - If proxy causes issues: set OPENAI_OAUTH_NO_PROXY=1");
            }
            false
        }
    }
}

fn build_authorize_url(
    client_id: &str,
    redirect_uri: &str,
    scope: &str,
    code_challenge: &str,
    state: &str,
    auth_url: &str,
) -> String {
    let params = [
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("scope", scope),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "codex_cli_rs"),
    ];
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect();
    format!("{}?{}", auth_url.trim_end_matches('/'), query.join("&"))
}

fn parse_redirect_uri(uri: &str) -> Option<(String, u16, String)> {
    let url = url::Url::parse(uri).ok()?;
    let host = url.host_str().unwrap_or("127.0.0.1").to_string();
    let port = url.port().unwrap_or(1455);
    let path = url.path().to_string();
    let path = if path.is_empty() {
        "/auth/callback".to_string()
    } else {
        path
    };
    Some((host, port, path))
}

fn start_callback_server(redirect_uri: &str) -> Option<(String, String)> {
    let (host, port, path) = parse_redirect_uri(redirect_uri)?;

    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).ok()?;
    listener.set_nonblocking(false).ok()?;

    let (tx, rx) = mpsc::channel();
    let listener_clone = listener.try_clone().ok()?;
    thread::spawn(move || {
        if let Ok((stream, _)) = listener_clone.accept() {
            let _ = tx.send(Some(stream));
        }
    });
    let stream = rx.recv_timeout(Duration::from_secs(180)).ok()??;
    let mut stream = stream;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;

    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).ok()?;
    let data = String::from_utf8_lossy(&buf[..n]);
    let first_line = data.lines().next()?;
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let req_path = parts[1];
    let (req_path_only, query) = match req_path.find('?') {
        Some(i) => {
            let (p, q) = req_path.split_at(i);
            (p, &q[1..])
        }
        None => (req_path, ""),
    };

    if req_path_only != path {
        return None;
    }

    let code = extract_param(query, "code")?;
    let state = extract_param(query, "state")?;

    let body = "Authentication complete. You can close this tab.";
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());

    Some((code.to_string(), state.to_string()))
}

fn extract_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            return Some(urlencoding::decode(v).ok()?.into_owned());
        }
    }
    None
}

fn prompt_manual_input(expected_state: &str) -> Option<(String, String)> {
    println!("Browser callback not captured automatically.");
    println!("Paste the full redirect URL from the browser address bar (or code#state).\n");

    let mut raw = String::new();
    if io::stdin().read_line(&mut raw).is_err() {
        return None;
    }
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    if raw.contains('?') || raw.starts_with("http") {
        if let Ok(url) = url::Url::parse(raw) {
            let query = url.query().unwrap_or("");
            if let Some(code) = extract_param(query, "code") {
                let state =
                    extract_param(query, "state").unwrap_or_else(|| expected_state.to_string());
                return Some((code, state));
            }
        }
    }

    if let Some((code, state)) = raw.split_once('#') {
        let code = code.trim();
        let state = state.trim();
        if !code.is_empty() && !state.is_empty() {
            return Some((code.to_string(), state.to_string()));
        }
    }

    Some((raw.to_string(), expected_state.to_string()))
}

fn exchange_code(
    client: &BlockingClient,
    token_url: &str,
    client_id: &str,
    redirect_uri: &str,
    code: &str,
    code_verifier: &str,
) -> Result<token::TokenFile, anyhow::Error> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", client_id),
        ("code_verifier", code_verifier),
        ("redirect_uri", redirect_uri),
    ];

    let resp = client.post(token_url).header("Content-Type", "application/x-www-form-urlencoded").form(&params).send().map_err(|e| {
        let mut msg = format!("request to {} failed: {}", token_url, e);
        if let Some(source) = e.source() {
            msg.push_str(&format!("\n  cause: {}", source));
        }
        if std::env::var("HTTPS_PROXY").is_ok() || std::env::var("https_proxy").is_ok() {
            msg.push_str("\n  (HTTPS_PROXY is set - try OPENAI_OAUTH_NO_PROXY=1 if proxy breaks this)");
        } else {
            msg.push_str("\n  (auth.openai.com may be unreachable - try HTTPS_PROXY if in blocked region)");
        }
        anyhow::anyhow!("{}", msg)
    })?;

    let status = resp.status();
    let text = resp
        .text()
        .map_err(|e| anyhow::anyhow!("failed to read response body: {}", e))?;

    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "Token exchange HTTP {}: {}",
            status,
            if text.len() > 500 {
                format!("{}...", &text[..500])
            } else {
                text
            }
        ));
    }

    let payload: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("invalid JSON response: {}", e))?;
    token::parse_token_payload(&payload, "").map_err(|e| anyhow::anyhow!("{}", e))
}

fn open_url(url: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    opener::open(url)?;
    Ok(())
}

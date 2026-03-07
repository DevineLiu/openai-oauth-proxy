use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_AUTH_FILE: &str = "~/.config/openai-oauth-proxy/aopenai-browser-token.json";
const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const EXPIRY_SKEW_SECS: u64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenFile {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(rename = "expires_at_unix")]
    pub expires_at_unix: u64,
}

pub fn auth_file_path() -> PathBuf {
    let raw = env::var("AGENT_AUTH_FILE").unwrap_or_else(|_| DEFAULT_AUTH_FILE.to_string());
    PathBuf::from(shellexpand::tilde(&raw).into_owned())
}

fn load_token_raw() -> Option<TokenFile> {
    let path = auth_file_path();
    let data = fs::read_to_string(&path).ok()?;
    serde_json::from_str::<TokenFile>(&data).ok()
}

pub fn save_token(token: &TokenFile) -> std::io::Result<()> {
    let path = auth_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(token).unwrap())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn current_unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn parse_token_payload(
    payload: &serde_json::Value,
    refresh_fallback: &str,
) -> Result<TokenFile, Box<dyn std::error::Error + Send + Sync>> {
    let access = payload
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or("token response missing access_token")?;
    let refresh = payload
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(refresh_fallback);
    if refresh.is_empty() {
        return Err("token response missing refresh_token".into());
    }
    let expires_in: u64 = payload
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(3600);

    Ok(TokenFile {
        access_token: access.to_string(),
        refresh_token: refresh.to_string(),
        token_type: payload
            .get("token_type")
            .and_then(|v| v.as_str())
            .map(String::from),
        expires_at_unix: current_unix_time() + expires_in,
    })
}

fn refresh_token_inner(client: &Client, refresh_token: &str) -> Option<TokenFile> {
    let token_url =
        env::var("OPENAI_OAUTH_TOKEN_URL").unwrap_or_else(|_| OPENAI_TOKEN_URL.to_string());
    let client_id =
        env::var("OPENAI_OAUTH_CLIENT_ID").unwrap_or_else(|_| OPENAI_CLIENT_ID.to_string());

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id.as_str()),
    ];

    let resp = client
        .post(&token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let payload: serde_json::Value = resp.json().ok()?;
    let token = parse_token_payload(&payload, refresh_token).ok()?;
    save_token(&token).ok()?;
    Some(token)
}

pub fn load_openai_browser_access_token() -> Option<String> {
    let mut token = load_token_raw()?;
    let now = current_unix_time();

    if token.expires_at_unix <= now + EXPIRY_SKEW_SECS {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .ok()?;
        token = refresh_token_inner(&client, &token.refresh_token)?;
    }

    Some(token.access_token)
}

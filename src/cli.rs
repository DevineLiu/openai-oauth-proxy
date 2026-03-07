use clap::{Parser, Subcommand};
use std::env;

#[derive(Parser, Debug)]
#[command(
    name = "openai-oauth-proxy",
    version,
    about = "OpenAI OAuth 2.0 PKCE browser login and token loader"
)]
pub struct Cli {
    #[arg(long, help = "Enable debug logs")]
    pub debug: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
    #[arg(
        long,
        help = "Print access token from local token file (auto-refresh if expired)"
    )]
    pub print_access_token: bool,
    #[arg(long, help = "Print resolved token file path and exit")]
    pub print_auth_file: bool,
    #[arg(long, help = "List built-in supported models")]
    pub list_models: bool,
    #[arg(long, value_name = "PATH", help = "Override token file path")]
    pub auth_file: Option<String>,
    #[arg(long, value_name = "URL", help = "Override OAuth authorize URL")]
    pub auth_url: Option<String>,
    #[arg(long, value_name = "URL", help = "Override OAuth token URL")]
    pub token_url: Option<String>,
    #[arg(long, value_name = "ID", help = "Override OAuth client ID")]
    pub client_id: Option<String>,
    #[arg(long, value_name = "URI", help = "Override OAuth redirect URI")]
    pub redirect_uri: Option<String>,
    #[arg(long, value_name = "SCOPES", help = "Override OAuth scopes")]
    pub scope: Option<String>,
    #[arg(long, help = "Disable proxy for OAuth HTTP requests")]
    pub no_proxy: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Auth,
    Serve {
        #[arg(
            long,
            value_name = "HOST",
            default_value = "127.0.0.1",
            help = "Proxy bind host"
        )]
        proxy_host: String,
        #[arg(
            long,
            value_name = "PORT",
            default_value_t = 8788,
            help = "Proxy bind port"
        )]
        proxy_port: u16,
    },
}

impl Command {
    pub fn serve_values(&self) -> Option<(&str, u16)> {
        match self {
            Command::Serve {
                proxy_host,
                proxy_port,
            } => Some((proxy_host.as_str(), *proxy_port)),
            _ => None,
        }
    }
}

pub fn apply_cli_overrides(cli: &Cli) {
    if let Some(v) = &cli.auth_file {
        env::set_var("AGENT_AUTH_FILE", v);
    }
    if let Some(v) = &cli.auth_url {
        env::set_var("OPENAI_OAUTH_AUTH_URL", v);
    }
    if let Some(v) = &cli.token_url {
        env::set_var("OPENAI_OAUTH_TOKEN_URL", v);
    }
    if let Some(v) = &cli.client_id {
        env::set_var("OPENAI_OAUTH_CLIENT_ID", v);
    }
    if let Some(v) = &cli.redirect_uri {
        env::set_var("OPENAI_OAUTH_REDIRECT_URI", v);
    }
    if let Some(v) = &cli.scope {
        env::set_var("OPENAI_OAUTH_SCOPE", v);
    }
    if cli.no_proxy {
        env::set_var("OPENAI_OAUTH_NO_PROXY", "1");
    }
    if cli.debug {
        env::set_var("OPENAI_OAUTH_PROXY_DEBUG", "1");
    }
}

# openai-oauth-proxy

【[English Guide](README.md), [中文说明](README.zh.md)】

This is a local proxy that bridges API-key-only OpenAI-compatible clients to ChatGPT OAuth authentication.

## Purpose

Many Agent/SDK tools only support:

- `OPENAI_BASE_URL=https://api.openai.com/v1`
- `OPENAI_API_KEY=<api_key>`

But in Teams/Business setups, users often authenticate with browser OAuth instead of directly using an OpenAI platform API key.

This project helps you:

- keep existing clients on OpenAI v1 + API key style configuration
- route requests to an OAuth-backed upstream session locally
- reuse OAuth accounts in current agent workflows

## How It Works

High-level path:

`Agent/Client -> OpenAI v1 request -> local openai-oauth-proxy -> OAuth token -> ChatGPT/Codex upstream`

In detail:

1. Client sends requests to local `/v1/*` with a placeholder `Authorization: Bearer proxy`.
2. Proxy resolves token source by priority:
   - `OPENAI_PROXY_BEARER_TOKEN`
   - `OPENAI_OAUTH_TOKEN` / `OPENAI_API_KEY`
   - local token file (auto-refresh supported)
3. For `chatgpt.com/backend-api` + `/v1/chat/completions`, request/response is transformed through Codex responses format.
4. Client still receives OpenAI-style responses.

## Local Install

### Requirements

- Rust (stable recommended)
- Network access to `auth.openai.com`

### Install

```bash
cargo install --git https://github.com/liuhongru/openai-oauth-proxy --branch main
```

### Authenticate and Start

```bash
# 1) First-time OAuth login
openai-oauth-proxy auth

# 2) Start local proxy (default 127.0.0.1:8788)
openai-oauth-proxy serve
```

## Docker Install

### Pull Published Image (GHCR)

```bash
docker pull ghcr.io/devineliu/openai-oauth-proxy:latest
```

### Build Image

```bash
docker build -t openai-oauth-proxy .
```

### Authenticate in Container (no browser auto-open)

```bash
docker run --rm -it \
  -e OPENAI_OAUTH_NO_BROWSER=1 \
  -v "$HOME/.config/openai-oauth-proxy:/home/appuser/.config/openai-oauth-proxy" \
  openai-oauth-proxy auth
```

### Start Proxy

```bash
docker run --rm -p 8788:8788 \
  -e OPENAI_PROXY_UPSTREAM=https://chatgpt.com/backend-api \
  -v "$HOME/.config/openai-oauth-proxy:/home/appuser/.config/openai-oauth-proxy" \
  openai-oauth-proxy
```

## Usage

### 0) Preferred for agent workflows (highest priority)

When working in Chat with coding agents, first paste this branch file URL:

`https://raw.githubusercontent.com/DevineLiu/openai-oauth-proxy/main/LLM_INSTALL.md`

Then let the agent follow that document end-to-end.

### 1) Start proxy (no subcommand required)

Default first-run flow:

1. Run `openai-oauth-proxy` (no subcommand).
2. If a local token exists, it starts proxy directly.
3. If no token exists, it runs OAuth login first, then starts proxy.
By default, `openai-oauth-proxy serve` listens on `127.0.0.1:8788` and uses upstream `https://chatgpt.com/backend-api`.
It reads credentials in this order: `OPENAI_PROXY_BEARER_TOKEN` -> `OPENAI_OAUTH_TOKEN` / `OPENAI_API_KEY` -> local auth file (`AGENT_AUTH_FILE`).

You can still use explicit commands if needed: `openai-oauth-proxy auth` and `openai-oauth-proxy serve`.

### 2) Configure your OpenAI-compatible client

```bash
export OPENAI_BASE_URL=http://127.0.0.1:8788/v1
export OPENAI_API_KEY=proxy
```

### 3) Health check

```bash
curl -s http://127.0.0.1:8788/healthz
```

### 4) Common commands

```bash
openai-oauth-proxy auth
openai-oauth-proxy serve
openai-oauth-proxy serve --proxy-host 0.0.0.0 --proxy-port 8788
openai-oauth-proxy --print-auth-file
openai-oauth-proxy --print-access-token
openai-oauth-proxy --list-models
```

## Config Examples

### Example A: local + default upstream

```bash
export OPENAI_BASE_URL=http://127.0.0.1:8788/v1
export OPENAI_API_KEY=proxy
openai-oauth-proxy serve
```

### Example B: Docker + persisted token

```bash
docker run --rm -p 8788:8788 \
  -e OPENAI_PROXY_UPSTREAM=https://chatgpt.com/backend-api \
  -e OPENAI_OAUTH_NO_BROWSER=1 \
  -v "$HOME/.config/openai-oauth-proxy:/home/appuser/.config/openai-oauth-proxy" \
  openai-oauth-proxy
```

### Example C: explicit bearer token

```bash
export OPENAI_BASE_URL=http://127.0.0.1:8788/v1
export OPENAI_API_KEY=proxy
export OPENAI_PROXY_BEARER_TOKEN="<your_oauth_or_bearer_token>"
openai-oauth-proxy serve
```

## Environment Variables

- `OPENAI_PROXY_UPSTREAM`: upstream URL (default `https://chatgpt.com/backend-api`)
- `OPENAI_PROXY_BEARER_TOKEN`: explicit bearer token for forwarding
- `OPENAI_API_KEY`: client compatibility field; can be fallback token source
- `OPENAI_OAUTH_TOKEN`: manually provided OAuth token
- `AGENT_AUTH_FILE`: token file path (default `~/.config/openai-oauth-proxy/aopenai-browser-token.json`)
- `OPENAI_OAUTH_AUTH_URL`: OAuth authorize URL override
- `OPENAI_OAUTH_TOKEN_URL`: OAuth token URL override
- `OPENAI_OAUTH_CLIENT_ID`: OAuth client id override
- `OPENAI_OAUTH_REDIRECT_URI`: OAuth redirect URI override
- `OPENAI_OAUTH_SCOPE`: OAuth scopes override
- `OPENAI_OAUTH_NO_PROXY=1`: bypass system proxy for OAuth/upstream HTTP calls
- `OPENAI_OAUTH_NO_BROWSER=1`: disable browser auto-open and use manual login flow
- `OPENAI_OAUTH_PROXY_DEBUG=1`: enable debug logs

## Open Source Readiness

- Default branch: `main`
- Image publishing branch: pushes to `main` publish `ghcr.io/devineliu/openai-oauth-proxy:latest` (multi-arch `linux/amd64,linux/arm64`)
- Security policy: `SECURITY.md`
- Security fix support: latest `main` branch
- Private vulnerability reporting: `https://github.com/DevineLiu/openai-oauth-proxy/security/advisories/new`
- License: MIT (`LICENSE`)
- CI workflow: `.github/workflows/ci.yml`
- Security workflow (cargo-audit + CodeQL): `.github/workflows/security.yml`
- Dependency updates: `.github/dependabot.yml`

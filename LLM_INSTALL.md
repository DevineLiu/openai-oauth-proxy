# LLM_INSTALL

This guide is for coding agents/LLMs to set up and run `openai-oauth-proxy` quickly and safely.

## Agent Skill Layout

### Skill: openai-oauth-proxy-setup

#### Goal

Set up a working local OpenAI-compatible endpoint backed by ChatGPT OAuth.

#### Inputs

- Repo path
- Preferred mode: local binary or Docker
- Optional custom upstream (`OPENAI_PROXY_UPSTREAM`)

#### Required Outputs

- Running proxy endpoint: `http://127.0.0.1:8788/v1`
- Health check response from `/healthz`
- Minimal env sample for OpenAI-compatible clients

## Quickest Path (Recommended for Agents)

1. Install binary:

```bash
cargo install --path .
```

2. Run OAuth login once:

```bash
openai-oauth-proxy auth
```

3. Start the proxy:

```bash
openai-oauth-proxy serve
```

4. Verify service:

```bash
curl -s http://127.0.0.1:8788/healthz
```

5. Configure client:

```bash
export OPENAI_BASE_URL=http://127.0.0.1:8788/v1
export OPENAI_API_KEY=proxy
```

## Docker Path

### Pull image

```bash
docker pull ghcr.io/devineliu/openai-oauth-proxy:latest
```

### Login and run

```bash
docker run --rm -it \
  -e OPENAI_OAUTH_NO_BROWSER=1 \
  -v "$HOME/.config/openai-oauth-proxy:/home/appuser/.config/openai-oauth-proxy" \
  ghcr.io/devineliu/openai-oauth-proxy:latest auth

docker run --rm -p 8788:8788 \
  -e OPENAI_PROXY_UPSTREAM=https://chatgpt.com/backend-api \
  -v "$HOME/.config/openai-oauth-proxy:/home/appuser/.config/openai-oauth-proxy" \
  ghcr.io/devineliu/openai-oauth-proxy:latest
```

## Operational Notes for Agents

- Do not print or commit OAuth tokens.
- Prefer `OPENAI_PROXY_BEARER_TOKEN` only for explicit token forwarding scenarios.
- If login cannot open browser, set `OPENAI_OAUTH_NO_BROWSER=1` and use manual URL flow.
- Keep user clients configured with OpenAI-compatible fields only (`OPENAI_BASE_URL` + `OPENAI_API_KEY`).

## Troubleshooting

- `401/403`: rerun `openai-oauth-proxy auth` to refresh credentials.
- `connection refused`: confirm service is running on `127.0.0.1:8788`.
- upstream errors: verify `OPENAI_PROXY_UPSTREAM` and local network access.

## Copy/Paste Prompt for Chat Agents

Use this when delegating setup to an agent:

```text
Read and follow this file first: <branch-url>/LLM_INSTALL.md
Then perform setup, run health checks, and return exact env vars for my client.
```

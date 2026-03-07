#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROXY_URL="http://127.0.0.1:8788"
TMP_DIR="$(mktemp -d /tmp/rig-proxy-test.XXXXXX)"

cleanup() {
  if [[ -n "${PROXY_PID:-}" ]] && kill -0 "$PROXY_PID" 2>/dev/null; then
    kill "$PROXY_PID" 2>/dev/null || true
    wait "$PROXY_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

cd "$ROOT_DIR"
cargo run -- serve --proxy-host 127.0.0.1 --proxy-port 8788 >/tmp/openai-oauth-proxy-rig-test.log 2>&1 &
PROXY_PID=$!

for _ in $(seq 1 30); do
  if curl -fsS "$PROXY_URL/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

cat >"$TMP_DIR/Cargo.toml" <<'EOF'
[package]
name = "rig-proxy-smoke"
version = "0.1.0"
edition = "2021"

[dependencies]
rig-core = "0.31"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
anyhow = "1"
EOF

mkdir -p "$TMP_DIR/src"
cat >"$TMP_DIR/src/main.rs" <<'EOF'
use rig::{client::{CompletionClient, ProviderClient}, completion::Prompt, providers::openai};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let client = openai::Client::from_env();
    let agent = client
        .completion_model("gpt-5.2-codex")
        .completions_api()
        .into_agent_builder()
        .preamble("You are a test assistant. Reply with exact token RIG_PROXY_OK.")
        .build();
    let out = agent.prompt("Reply with: RIG_PROXY_OK").await?;
    println!("{}", out);
    Ok(())
}
EOF

set +e
(cd "$TMP_DIR" && OPENAI_BASE_URL="http://127.0.0.1:8788" OPENAI_API_KEY="proxy" cargo run >/tmp/openai-oauth-proxy-rig-client.log 2>&1)
RC=$?
set -e

if [[ $RC -ne 0 ]]; then
  echo "rig client failed"
  echo "==== proxy log ===="
  cat /tmp/openai-oauth-proxy-rig-test.log || true
  echo "==== rig client log ===="
  cat /tmp/openai-oauth-proxy-rig-client.log || true
  exit 1
fi

if ! grep -q "RIG_PROXY_OK" /tmp/openai-oauth-proxy-rig-client.log; then
  echo "rig output does not contain expected marker"
  echo "==== rig client log ===="
  cat /tmp/openai-oauth-proxy-rig-client.log || true
  exit 1
fi

echo "PASS: rig connected via proxy at $PROXY_URL"

#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROXY_URL="http://127.0.0.1:8788"

cleanup() {
  if [[ -n "${PROXY_PID:-}" ]] && kill -0 "$PROXY_PID" 2>/dev/null; then
    kill "$PROXY_PID" 2>/dev/null || true
    wait "$PROXY_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

cd "$ROOT_DIR"
cargo run -- serve --proxy-host 127.0.0.1 --proxy-port 8788 >/tmp/openai-oauth-proxy-test.log 2>&1 &
PROXY_PID=$!

for _ in $(seq 1 30); do
  if curl -fsS "$PROXY_URL/healthz" >/tmp/openai-oauth-proxy-healthz.txt 2>/dev/null; then
    break
  fi
  sleep 1
done

HEALTHZ_BODY="$(cat /tmp/openai-oauth-proxy-healthz.txt 2>/dev/null || true)"
if [[ "$HEALTHZ_BODY" != "ok" ]]; then
  echo "healthz failed for $PROXY_URL (got: '$HEALTHZ_BODY')"
  echo "proxy log:"
  cat /tmp/openai-oauth-proxy-test.log || true
  exit 1
fi

UPSTREAM_STATUS="$(curl -s -o /dev/null -w '%{http_code}' "$PROXY_URL/v1/models")"
if [[ "$UPSTREAM_STATUS" == "000" ]]; then
  echo "proxy did not respond on /v1/models"
  exit 1
fi

echo "PASS: proxy reachable at $PROXY_URL"

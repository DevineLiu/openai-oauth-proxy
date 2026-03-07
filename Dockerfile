FROM rust:1.86-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/openai-oauth-proxy /usr/local/bin/openai-oauth-proxy

RUN useradd --create-home --uid 10001 appuser
USER appuser

ENV OPENAI_OAUTH_NO_BROWSER=1
ENV AGENT_AUTH_FILE=/home/appuser/.config/openai-oauth-proxy/aopenai-browser-token.json

EXPOSE 8788

ENTRYPOINT ["openai-oauth-proxy"]
CMD ["serve", "--proxy-host", "0.0.0.0", "--proxy-port", "8788"]

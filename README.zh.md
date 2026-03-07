# openai-oauth-proxy

【[English Guide](README.md), [中文说明](README.zh.md)】

这是一个本地代理工具，帮你把只支持 OpenAI API Key 的客户端 桥接到  ChatGPT OAuth  认证上。

## 目的

目前生态里很多 Agent/SDK 工具只支持以下连接方式：

- `OPENAI_BASE_URL=https://api.openai.com/v1`
- `OPENAI_API_KEY=<api_key>`

但很多团队用户（例如 ChatGPT Teams/Business）常见的是浏览器 OAuth 登录，不一定直接使用平台 API Key。

本工具的目标是：

- 保持客户端仍按 `OpenAI v1 + API Key` 方式接入
- 在本地把请求转发到 OAuth 会话可用的上游
- 让 OAuth 账号也能被现有 Agent 工具链复用

## 原理

高层链路如下：

`Agent/Client -> OpenAI v1 协议请求 -> 本地 openai-oauth-proxy -> OAuth Token -> ChatGPT/Codex 上游`

更具体地说：

1. 客户端把请求发到本地 `/v1/*`，并带上占位 `Authorization: Bearer proxy`（或任意非空值）。
2. 代理按优先级读取 token：
   - `OPENAI_PROXY_BEARER_TOKEN`
   - `OPENAI_OAUTH_TOKEN` / `OPENAI_API_KEY`
   - 本地 token 文件（会自动 refresh）
3. 当上游是 `chatgpt.com/backend-api` 且路径是 `/v1/chat/completions` 时，会转换为 Codex responses 请求并回转为 OpenAI 风格响应。
4. 客户端依然看到 OpenAI 风格接口，不需要理解 OAuth 细节。

## 本机安装

### 前置要求

- Rust（建议 stable）
- 可访问 `auth.openai.com`

### 安装

```bash
cargo install --path .
```

### 认证并启动

```bash
# 1) 首次 OAuth 登录
openai-oauth-proxy auth

# 2) 启动本地代理（默认 127.0.0.1:8788）
openai-oauth-proxy serve
```

## Docker 安装

### 构建镜像

```bash
docker build -t openai-oauth-proxy .
```

### 容器内认证（不会尝试自动打开浏览器）

```bash
docker run --rm -it \
  -e OPENAI_OAUTH_NO_BROWSER=1 \
  -v "$HOME/.config/openai-oauth-proxy:/home/appuser/.config/openai-oauth-proxy" \
  openai-oauth-proxy auth
```

### 启动代理

```bash
docker run --rm -p 8788:8788 \
  -e OPENAI_PROXY_UPSTREAM=https://chatgpt.com/backend-api \
  -v "$HOME/.config/openai-oauth-proxy:/home/appuser/.config/openai-oauth-proxy" \
  openai-oauth-proxy
```

## 使用方法

### 1) 在客户端里配置 OpenAI 兼容地址

```bash
export OPENAI_BASE_URL=http://127.0.0.1:8788/v1
export OPENAI_API_KEY=proxy
```

### 2) 健康检查

```bash
curl -s http://127.0.0.1:8788/healthz
```

### 3) 常用命令

```bash
# 启动 OAuth 认证流程
openai-oauth-proxy auth

# 启动代理
openai-oauth-proxy serve

# 自定义监听地址
openai-oauth-proxy serve --proxy-host 0.0.0.0 --proxy-port 8788

# 查看 token 文件路径
openai-oauth-proxy --print-auth-file

# 打印 access token（必要时会自动刷新）
openai-oauth-proxy --print-access-token

# 查看内置模型列表
openai-oauth-proxy --list-models
```

## 配置样例

### 样例 A：本机 + 默认上游

```bash
export OPENAI_BASE_URL=http://127.0.0.1:8788/v1
export OPENAI_API_KEY=proxy
openai-oauth-proxy serve
```

### 样例 B：Docker + 持久化 token

```bash
docker run --rm -p 8788:8788 \
  -e OPENAI_PROXY_UPSTREAM=https://chatgpt.com/backend-api \
  -e OPENAI_OAUTH_NO_BROWSER=1 \
  -v "$HOME/.config/openai-oauth-proxy:/home/appuser/.config/openai-oauth-proxy" \
  openai-oauth-proxy
```

### 样例 C：显式指定 bearer token

```bash
export OPENAI_BASE_URL=http://127.0.0.1:8788/v1
export OPENAI_API_KEY=proxy
export OPENAI_PROXY_BEARER_TOKEN="<your_oauth_or_bearer_token>"
openai-oauth-proxy serve
```

## 环境变量

- `OPENAI_PROXY_UPSTREAM`：上游地址（默认 `https://chatgpt.com/backend-api`）
- `OPENAI_PROXY_BEARER_TOKEN`：显式指定转发用 bearer token
- `OPENAI_API_KEY`：兼容客户端字段；可作为 token 兜底来源
- `OPENAI_OAUTH_TOKEN`：手动注入 OAuth token
- `AGENT_AUTH_FILE`：token 文件路径（默认 `~/.config/openai-oauth-proxy/aopenai-browser-token.json`）
- `OPENAI_OAUTH_AUTH_URL`：OAuth authorize URL
- `OPENAI_OAUTH_TOKEN_URL`：OAuth token URL
- `OPENAI_OAUTH_CLIENT_ID`：OAuth client id
- `OPENAI_OAUTH_REDIRECT_URI`：OAuth redirect URI
- `OPENAI_OAUTH_SCOPE`：OAuth scopes
- `OPENAI_OAUTH_NO_PROXY=1`：OAuth/上游请求不走系统代理
- `OPENAI_OAUTH_NO_BROWSER=1`：不自动打开浏览器，改为手动复制 URL 登录
- `OPENAI_OAUTH_PROXY_DEBUG=1`：开启 debug 日志

## 开源就绪

- 许可证：MIT（`LICENSE`）
- 安全策略：`SECURITY.md`
- CI 工作流：`.github/workflows/ci.yml`
- 安全工作流（cargo-audit + CodeQL）：`.github/workflows/security.yml`
- 依赖更新：`.github/dependabot.yml`

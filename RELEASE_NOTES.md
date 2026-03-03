## 🚀 AG Proxy Manager v0.1.0 — Initial Release / 首个正式版本

> 🇨🇳 [中文](#-中文说明) | 🇺🇸 [English](#-english)

---

## 🇺🇸 English

A high-performance desktop proxy manager for AI coding assistants, built with **Tauri v2 + Rust**.
Acts as a local HTTPS proxy between your IDE and the upstream Google Cloud Code API.

### ✨ Features

- 🔑 **Multi-Account Management** — Import via JSON file, Refresh Token, or Google OAuth2
- 📊 **Quota Monitoring** — Real-time Claude / Gemini quota visualization with color-coded progress bars
- 🔄 **Intelligent Routing** — Fill mode / Round-robin + automatic capacity failover (`claude-opus` ↔ `claude-sonnet`)
- 🌐 **Dual Transport** — Proxy mode & Gateway mode (official LS Bridge) with automatic fallback
- 🔧 **IDE Integration** — One-click patch + self-signed CA certificate management
- 📈 **Token Statistics** — Multi-format SSE parsing (Gemini / Claude / OpenAI), persistent storage
- 🔍 **Request Flow Tracing** — Hop-by-hop visualization with forward & return direction display
- 🛡️ **AI Provider Management** — OpenAI / Gemini / Claude protocols with model name mapping

### 📦 Build from Source

```bash
git clone https://github.com/Ethan-W20/ag-proxy-tools.git
cd ag-proxy-tools
npm install
npx tauri build
```

> ⚠️ This release is source-only. Pre-built installers will be provided in future releases.

---

## 🇨🇳 中文说明

基于 **Tauri v2 + Rust** 构建的高性能桌面 AI 编程助手代理管理器。
在 IDE 与上游 Google Cloud Code API 之间建立本地 HTTPS 代理，支持多账号轮换与智能路由。

### ✨ 主要功能

- 🔑 **多账号管理** — JSON 文件 / Refresh Token / Google OAuth2 三种方式导入
- 📊 **配额监控** — 实时查询 Claude / Gemini 各模型组配额，可视化进度条
- 🔄 **智能路由** — Fill 填充模式 / Round-robin 轮询模式 + 容量故障转移（`claude-opus` ↔ `claude-sonnet`）
- 🌐 **双传输架构** — 代理模式 & 网关模式（官方 LS Bridge），网关失败自动回退直连
- 🔧 **IDE 集成** — 一键补丁管理 + 自签名 CA 证书自动安装（Windows certutil）
- 📈 **Token 用量统计** — 自动解析 Gemini / Claude / OpenAI 三种 SSE 格式，持久化存储
- 🔍 **请求链路追踪** — 逐跳可视化，正向请求 → 与反向响应 ← 完整展示
- 🛡️ **第三方 AI 供应商** — 支持 OpenAI / Gemini / Claude 协议，模型名称自动映射

### 📦 从源码编译

```bash
git clone https://github.com/Ethan-W20/ag-proxy-tools.git
cd ag-proxy-tools
npm install
npx tauri build
```

> ⚠️ 此版本为源码发布，需自行编译。预编译安装包将在后续版本提供。

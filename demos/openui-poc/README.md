# OpenUI + Rust Server POC

A proof-of-concept demonstrating [OpenUI](https://github.com/thesysdev/openui)
(generative UI framework) with a Rust axum backend as the AI proxy.

## Architecture

```
Browser ──POST /api/chat──> Rust (axum :3001) ──stream──> OpenAI API
   ^                              |
   └──── SSE stream (OpenUI Lang tokens) ────┘
```

- **Rust server** (`src/main.rs`): axum server that proxies chat requests to
  any OpenAI-compatible API and streams SSE responses back. Also serves the
  built frontend as static files.
- **React frontend** (`ui/`): Vite + React app using `@openuidev/react-ui`
  which parses OpenUI Lang tokens from the stream and renders live UI components
  (charts, tables, forms, cards, etc.).

## Prerequisites

- Rust 1.78+
- Node.js 18+
- An OpenAI API key (or any OpenAI-compatible provider)

## Quick start

```bash
# 1. Build the frontend
cd ui
npm install
npm run build
cd ..

# 2a. Run with Claude CLI (no API key needed — uses `claude` binary auth)
cargo run

# 2b. OR run with OpenAI-compatible API
OPENAI_API_KEY=sk-your-key-here cargo run

# 3. Open http://localhost:3001
```

## AI backend selection

The server auto-selects the AI backend:

- **No `OPENAI_API_KEY` set** -> uses the local `claude` CLI binary
  (same approach as `ai-runner/src/runners/claude.rs`). The `claude`
  binary manages its own auth (`claude auth login`). No API key needed.
- **`OPENAI_API_KEY` set** -> proxies to any OpenAI-compatible API.

The Claude CLI path discovers the binary via: `CLAUDE_BINARY` env,
`PATH`, `~/.local/bin`, `~/.bun/bin`, VS Code/Cursor extensions, etc.

## Development mode (hot reload)

```bash
# Terminal 1: Rust server
OPENAI_API_KEY=sk-your-key cargo run

# Terminal 2: Vite dev server (proxies /api to :3001)
cd ui && npm run dev
# Open http://localhost:5173
```

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `OPENAI_API_KEY` | (required) | Your API key |
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | Base URL for OpenAI-compatible API |
| `OPENAI_MODEL` | `gpt-4o-mini` | Model name |
| `SYSTEM_PROMPT_PATH` | `ui/src/generated/system-prompt.txt` | Path to the generated system prompt |

### Using alternative providers

```bash
# OpenRouter
OPENAI_API_KEY=sk-or-v1-... \
OPENAI_BASE_URL=https://openrouter.ai/api/v1 \
OPENAI_MODEL=openai/gpt-4o-mini \
cargo run

# Anthropic (via OpenAI-compatible proxy)
OPENAI_API_KEY=sk-ant-... \
OPENAI_BASE_URL=https://api.anthropic.com/v1 \
OPENAI_MODEL=claude-sonnet-4-20250514 \
cargo run
```

## How the AI hooks up

1. **System prompt generation**: `npm run generate:prompt` in `ui/` uses the
   OpenUI CLI to scan the React component library and produce
   `ui/src/generated/system-prompt.txt`. This prompt teaches the LLM the
   OpenUI Lang syntax and available components.

2. **Rust proxy**: The axum server reads the system prompt at startup, prepends
   it to every chat request, and streams the response from OpenAI back as SSE.

3. **Frontend rendering**: The `@openuidev/react-ui` `<FullScreen>` component
   uses `openAIAdapter()` to consume the SSE stream and progressively render
   OpenUI Lang tokens into React components as they arrive.

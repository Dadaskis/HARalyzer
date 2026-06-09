<image width="64px" src="public/icon.svg"/>

# HARalyzer

*Official, polite version*: Desktop app for analyzing HAR (HTTP Archive) files with LLM assistance via [OpenRouter](https://openrouter.ai/).

*Cooler version*: A kick-ass HAR viewer with agentic AI that can produce impactful AI slop to learn more about ANY HAR exports. Truly revolutionary, made by me for myself <3

<image width="500px" src="public/screenshot.png"/>

## Features

### HAR analysis

- **Stream-parse huge HAR files** — entries are read from disk incrementally, never loaded entirely into memory
- **Virtualized entry table** — browse 100k+ requests with search and filters (method, status, URL)
- **Chunked LLM analysis** — map-reduce pipeline with **parallel chunk processing** (configurable concurrency)
- **Preserve chunk results** — re-run **Analyze** without losing completed chunks; **Reset** clears summaries; **Report** generates only the final synthesis
- **JavaScript analysis** — detects fetch/XHR/axios/WebSocket patterns and sends JS sources to the LLM
- **Entry inspector** — headers, bodies, JS insights; **Ask AI** about a specific request
- **Session history** — SQLite stores past analyses for resume and review
- **Markdown export** — export full analysis reports (`.md`)

### AI chat (agent mode)

- **Tool-backed answers** — the assistant queries real HAR data instead of guessing:
  - `list_entries` / `get_entry` / `get_js_analysis`
  - `get_session_overview` / `get_chunk_summaries`
  - `generate_curl` / `execute_request` (live HTTP replay for testing)
- **Multi-step agent loop** — multiple LLM rounds with live **tool activity** in the UI
- **Configurable step limit** — max tool rounds per batch (Settings); **Continue** / **Stop** when the limit is hit
- **Thinking mode** — optional reasoning model; reasoning is collapsed under a **Reasoning** dropdown
- **Clear chat** — wipe the conversation for the current session

### UI

- Bloomy dark theme, custom scrollbars, syntax-highlighted markdown
- Copy button on code blocks; text wrapping for long URLs and HAR filenames
- Startup splash screen; stick-to-bottom chat scroll with **Jump to latest**
- Lazy-loaded analysis tabs for better performance on large sessions

## Tech stack

| Layer | Stack |
|-------|--------|
| Frontend | React 19, TypeScript, Vite, Tailwind CSS 4, shadcn/ui |
| Desktop | Tauri 2 |
| Backend | Rust — streaming HAR parser, OpenRouter client, SQLite, agent tools |

## Prerequisites (development & building only)

These are **not** required for end users who receive a built installer from `export/`.

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://www.rust-lang.org/tools/install)
- [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS

## Setup

```bash
# Windows
setup.bat

# macOS (first time — installs npm deps, checks Xcode CLT / Node / Rust)
chmod +x setup-macos.sh && ./setup-macos.sh

# Linux (Arch example in script)
chmod +x setup.sh && ./setup.sh

# Or manually (any OS)
npm install
```

**macOS notes:** You need [Xcode Command Line Tools](https://developer.apple.com/xcode/resources/) (`xcode-select --install`), [Node.js](https://nodejs.org/) 18+, and [Rust](https://rustup.rs/). [Homebrew](https://brew.sh/) is optional but convenient (`brew install node`).

## Development

```bash
npm run tauri dev
```

## Build (local release)

```bash
npm run tauri build
```

Installers appear under `src-tauri/target/release/bundle/` (NSIS/MSI on Windows, AppImage/deb on Linux, dmg on macOS).

## Export shareable installers

Build a redistributable package into **`export/`** — ready to share without Node.js or Rust on the recipient machine.

```bash
# Windows
export.bat

# macOS (must run on a Mac — produces .dmg + .app)
chmod +x export-macos.sh && ./export-macos.sh

# Linux
chmod +x export.sh && ./export.sh
```

| Platform | Script | Typical output in `export/` | Notes |
|----------|--------|----------------------------|--------|
| Windows | `export.bat` | `HARalyzer_*_x64-setup.exe` | NSIS installer; embeds WebView2 bootstrapper |
| macOS | `export-macos.sh` | `*.dmg`, `HARalyzer.app` | Run on macOS only; drag-and-drop install from `.dmg` |
| Linux | `export.sh` | `*.AppImage` (and optionally `.deb`) | AppImage needs no install |

Each export run also writes `export/README.txt` with platform-specific instructions.

**macOS distribution:** Share the `.dmg` from `export/`. Recipients open it and drag HARalyzer to Applications. For unsigned local builds, the first launch may require **right-click → Open** to bypass Gatekeeper.

The `export/` folder is gitignored — commit the scripts, not the binaries.

## Configuration

Open **Settings** (gear icon):

| Setting | Description |
|---------|-------------|
| **OpenRouter API Key** | From [openrouter.ai/keys](https://openrouter.ai/keys) |
| **Default Model** | Chat and analysis (e.g. `openai/gpt-4o-mini`) |
| **Thinking Model** | Used when **Thinking** toggle is on in chat |
| **Chat agent tool steps** | Max LLM tool rounds per batch before **Continue** prompt (default: 10) |
| **Chunk Max Tokens** | Entries per LLM chunk (default: 3000) |
| **Parallel LLM Requests** | Concurrent chunk analysis (default: 4) |
| **Filter Static Assets** | Skip images, fonts, CSS when chunking |
| **Analyze JavaScript** | Regex scan + LLM pass for JS entries |

## Usage

1. **Open HAR** — select a `.har` file (large files stream-parse with progress)
2. Browse entries — search, filter, click a row for full request/response details
3. **Analyze** — parallel chunked LLM analysis (skips already-done chunks on re-run)
4. **Report** — generate final map-reduce summary when chunks are ready
5. **Chat** — ask follow-ups; the agent uses tools to look up real entry data
6. **Export** — save the markdown report from the analysis panel

### Chat tips

- Pin an entry via **Ask AI** in the entry panel for focused context
- Enable **Thinking** in chat if you configured a reasoning model
- Use **Clear chat** to start a fresh conversation
- If the agent hits the step limit, click **Continue** for another batch of tool calls

## Project layout

```
HARalyzer/
├── src/                      # React frontend
├── src-tauri/                # Rust backend (Tauri)
├── export.bat                # Windows → export/
├── export-macos.sh           # macOS → export/ (.dmg + .app)
├── export.sh                 # Linux → export/
├── setup.bat                 # Windows dev setup
├── setup-macos.sh            # macOS dev setup
├── setup.sh                  # Linux dev setup (Arch example)
└── export/                   # Built installers (generated, gitignored)
```

## License

MIT License! Happiness to everyone!

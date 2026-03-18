# SOC Terminal

A terminal UI for browsing Wazuh/OpenSearch alerts, correlating them with LLMs, and running SOC skills.

```
 ◈ SOC Terminal   Select entries with Space, then press A to analyse
 I Index: wazuh-alerts-4.x-2026.03.15   +/- Level ≥ 3   Last 24h   L LLM: openrouter:sonnet4.6
┌ Entries (50 loaded / 1243 total) ────────────────────────────────────────────┐
│   Timestamp            Lvl  Agent                  Rule     Description      │
│ ▶ □ 2026-03-15 08:12   12   98452937408.local       5710     SSH brute force  │
│   ■ 2026-03-15 08:11    5   98452937408.local       1002     …                │
└──────────────────────────────────────────────────────────────────────────────┘
┌ Analysis ────────────────────────────────────────────────────────────────────┐
│ Two entries from the same host show an SSH brute-force pattern followed by   │
│ a successful login from an unusual source IP.                                │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Requirements

- Rust (stable)
- OpenSearch / Wazuh running and accessible
- **OpenRouter** API key (routes to Claude, GPT, Gemini, DeepSeek, Qwen, etc.) **or** Ollama running locally

## Setup

```bash
cp config.yaml.example config.yaml
# edit config.yaml with your credentials
```

`config.yaml` is git-ignored — never commit it.

### OpenRouter

Sign up at [openrouter.ai](https://openrouter.ai), create an API key, and add it to `config.yaml` under `llm.openrouter.api_key`, or export:

```bash
export OPENROUTER_API_KEY=sk-or-...
```

### Ollama (local)

Set `llm.provider: ollama` and configure models under `llm.ollama.models`. The app uses `/api/chat` with streaming.

## Build & run

```bash
cargo build --release
./target/release/siem
```

Override the config path:

```bash
SIEM_CONFIG=/path/to/config.yaml ./target/release/siem
```

## Usage

1. On launch the app connects to OpenSearch and loads the most recent `wazuh-alerts` index.
2. Browse entries with `↑↓` or `j/k`.
3. Press `Space` to select entries you want correlated.
4. Press `A` to send them to the LLM — the analysis streams in at the bottom.
5. Press `L` to cycle through configured models (OpenRouter + local).
6. Press `S` to open the skill picker (e.g. Daily Activity Report).

## Keybindings

| Key | Action |
|-----|--------|
| `↑↓` / `j k` | Navigate entries |
| `Space` | Toggle select entry |
| `Enter` | Open detail view |
| `A` | Analyse selected entries with LLM |
| `S` | Open skill picker |
| `L` | Cycle LLM model |
| `I` | Open index picker |
| `T` | Open time range picker |
| `F` | Filter by agent name |
| `+` / `-` | Increase / decrease minimum alert level |
| `E` | Export selection + analysis to JSON |
| `R` | Reload entries |
| `N` / `→` | Next page |
| `P` / `←` | Previous page |
| `C` | Clear all selections |
| `Tab` | Cycle through analysis history |
| `[` / `]` | Scroll analysis panel up / down |
| `H` / `?` | Help |
| `Q` / `Ctrl-C` | Quit |

## Config reference

```yaml
opensearch:
  url: https://localhost:9200
  username: admin
  password: YOUR_PASSWORD
  verify_ssl: false

llm:
  provider: openrouter          # "openrouter" or "ollama"
  openrouter:
    api_key: ""                 # or use OPENROUTER_API_KEY env var
    models:
      - id: anthropic/claude-sonnet-4-6
        tag: sonnet4.6
      - id: anthropic/claude-opus-4-6
        tag: opus4.6
      - id: google/gemini-2.5-pro-preview
        tag: gemini-pro
      - id: openai/gpt-4.1
        tag: gpt4.1
      - id: deepseek/deepseek-r1
        tag: deepseek-r1
  ollama:
    url: http://localhost:11434
    models:
      - id: qwen3.5:0.8b
        tag: qwen3.5

ui:
  page_size: 50
```

## Project layout

```
src/
├── main.rs          — entry point, event loop, message handler
├── app.rs           — App state and AppMessage enum
├── config.rs        — Config structs (config.yaml)
├── input.rs         — Keyboard input handlers for all views
├── tasks.rs         — Background task spawners (load, analyse, export, report)
├── opensearch.rs    — OpenSearch query client + aggregations
├── llm.rs           — OpenRouter + Ollama streaming, prompt builders
├── skills.rs        — Skill registry (extensible)
├── report.rs        — Markdown report builder (daily activity report)
└── ui.rs            — Ratatui rendering (layout, table, popups)
```

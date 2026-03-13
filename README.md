# SOC Terminal

A terminal UI for browsing Wazuh/OpenSearch alerts and correlating them with an LLM.

```
 ◈ SOC Terminal   Select entries with Space, then press A to analyse
 I Index: wazuh-alerts-4.x-2026.03.13   +/- Level ≥ 3   Last 24h   L LLM: claude
┌ Entries (50 loaded / 1243 total) ────────────────────────────────────────────┐
│   Timestamp            Lvl  Agent                  Rule     Description      │
│ ▶ □ 2026-03-13 08:12   12   98452937408.local       5710     SSH brute force  │
│   ■ 2026-03-13 08:11    5   98452937408.local       1002     …                │
└──────────────────────────────────────────────────────────────────────────────┘
┌ Analysis ────────────────────────────────────────────────────────────────────┐
│ Two entries from the same host show an SSH brute-force pattern followed by   │
│ a successful login from an unusual source IP. The timing and rule sequence   │
│ suggest an active intrusion attempt worth investigating further.             │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Requirements

- Rust (stable)
- OpenSearch / Wazuh running and accessible
- Claude API key **or** Ollama running locally

## Setup

```bash
cp config.yaml.example config.yaml
# edit config.yaml with your credentials
```

`config.yaml` is git-ignored — never commit it.

### Claude

Set your API key in `config.yaml` under `llm.claude.api_key`, or export it as an environment variable:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

### Ollama

Set `llm.provider: ollama` and configure the model under `llm.ollama`. The app uses `/api/chat` with streaming.

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
5. Press `L` to switch between Claude and Ollama without restarting.

## Keybindings

| Key | Action |
|-----|--------|
| `↑↓` / `j k` | Navigate entries |
| `Space` | Toggle select entry |
| `A` | Analyse selected entries with LLM |
| `L` | Toggle LLM provider (Claude ↔ Ollama) |
| `I` | Open index picker |
| `+` / `-` | Increase / decrease minimum alert level |
| `R` | Reload entries |
| `N` / `→` | Next page |
| `P` / `←` | Previous page |
| `C` | Clear all selections |
| `[` / `]` | Scroll analysis panel up / down |
| `H` / `?` | Help |
| `Q` / `Ctrl-C` | Quit |

## Config reference

```yaml
opensearch:
  url: https://localhost:9200
  username: admin
  password: YOUR_PASSWORD
  verify_ssl: false          # set true in production

llm:
  provider: claude           # "claude" or "ollama"
  claude:
    api_key: ""              # or use ANTHROPIC_API_KEY env var
    model: claude-sonnet-4-6
  ollama:
    url: http://localhost:11434
    model: qwen3.5:0.8b

ui:
  page_size: 50
```

## Project layout

```
src/
├── main.rs          — event loop, key handler, async task spawners
├── app.rs           — App state and AppMessage enum
├── config.rs        — Config structs (config.yaml)
├── opensearch.rs    — OpenSearch query client
├── llm.rs           — Claude + Ollama streaming, prompt builder
└── ui.rs            — Ratatui rendering
```

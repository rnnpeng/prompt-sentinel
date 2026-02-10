# ⚡ Prompt Sentinel

**High-performance CLI for LLM prompt regression testing.**

Run unit tests against your prompts. Catch regressions before they hit production.

[![Rust](https://img.shields.io/badge/rust-2021%20edition-orange)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

## Why Prompt Sentinel?

Every time you tweak a system prompt, change models, or update temperatures, **something can break**. Prompt Sentinel runs structured tests against LLM APIs so you catch problems instantly — not from user complaints.

- 🚀 **Parallel execution** — tests run concurrently with configurable limits
- 🔄 **Retry with backoff** — handles rate limits and transient failures automatically
- 📸 **Snapshot testing** — detect output drift across prompt changes
- 📊 **CSV Data Loading** — run tests against large datasets
- ✅ **8 assertion types** — contains, regex, JSON validation, length bounds, latency
- 💰 **Cost tracking** — per-test token usage and USD cost estimates
- 📊 **HTML reports** — shareable dark-themed reports with pass rates and latency
- 🔗 **Webhooks** — support for custom LLM providers via HTTP endpoints

## Quick Start

```bash
# Install
cargo install --path .

# Initialize a new project
sentinel init

# Add your API key
echo "OPENAI_API_KEY=sk-your-key" > .env

# Validate your config
sentinel validate

# Run tests
sentinel run
```

## Configuration (`tests.yaml`)

```yaml
version: "1.0"
defaults:
  provider: "openai"
  model: "gpt-4o-mini"

tests:
  - id: "welcome-email"
    prompt: "Write a short welcome email for {{name}}."
    cases:
      - input: { name: "Alice" }
        assert:
          - type: "contains"
            value: "Alice"
```

## CSV Data Loading

For testing against large datasets (e.g. 50+ rows), use `cases_file`.

**`tests.yaml`**:
```yaml
tests:
  - id: "csv-bulk-test"
    prompt: "Summarize this review: {{review_text}}"
    cases_file: "data/reviews.csv"
    assertions:
      - type: "contains"
        value: "{{expected_sentiment}}"
```

**`data/reviews.csv`**:
```csv
review_text,expected_sentiment
"Great product!",Positive
"Terrible service.",Negative
```

Each row in the CSV is treated as a test case. Assertions can use `{{column_name}}` templates to validate dynamic expectations.

## GitHub Action

Run Prompt Sentinel in your CI pipeline to catch regressions on every PR.

Create `.github/workflows/sentinel.yml`:

```yaml
name: Prompt Sentinel
on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: promptsentinel/sentinel@v1  # Installs the CLI
      
      - name: Run Tests
        run: sentinel run --json
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
```

## Assertion Types

| Type | Value | Description |
|---|---|---|
| `contains` | `"text"` | Output contains string |
| `not-contains` | `"text"` | Output does NOT contain string |
| `latency_max` | `5000` | Response time under N ms |
| `min_length` | `50` | Output ≥ N chars |
| `max_length` | `1000` | Output ≤ N chars |
| `regex` | `"pattern"` | Matches regex |
| `json_valid` | `true` | Valid JSON |
| `snapshot` | `true` | Matches golden file |

## CLI Reference

```bash
sentinel run --file tests.yaml
sentinel run --filter welcome     # Run subset of tests
sentinel run --report             # Generate HTML report
sentinel run --verbose            # Show full LLM output
sentinel run --quiet              # Summary only
sentinel run --json               # JSON output for CI

# Watch Mode (Inner Dev Loop)
sentinel watch                    # Re-run tests on file save

```

## Custom Providers (Webhooks)

Run against local models (Ollama, vLLM) or private APIs:
```bash
export WEBHOOK_URL="http://localhost:11434/v1/chat/completions"
# Set provider: "webhook" in tests.yaml
```

## License

MIT

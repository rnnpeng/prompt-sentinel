//! Integration tests for Prompt Sentinel.
//!
//! Uses wiremock to mock LLM API responses so no real API keys are needed.

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Re-export modules for testing
mod common {
    // We test via the binary's public API by importing the library modules.
    // Since this is a binary crate, we use subprocess tests + unit-style tests
    // for the core logic.
}

/// Helper: create a mock OpenAI-compatible server that returns a fixed response.
async fn setup_mock_openai(response_text: &str) -> MockServer {
    let server = MockServer::start().await;

    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": response_text,
            }
        }],
        "usage": {
            "prompt_tokens": 15,
            "completion_tokens": 25,
            "total_tokens": 40,
        }
    });

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    server
}

/// Helper: create a failing mock server (429 rate limit).
async fn setup_rate_limited_server() -> MockServer {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
        .mount(&server)
        .await;

    server
}

/// Helper: create a webhook mock server.
async fn setup_mock_webhook(response_text: &str) -> MockServer {
    let server = MockServer::start().await;

    let body = serde_json::json!({
        "text": response_text,
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 20,
            "total_tokens": 30,
        }
    });

    Mock::given(method("POST"))
        .and(path("/complete"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    server
}

// ─── Provider Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod provider_tests {
    use super::*;

    #[tokio::test]
    async fn test_openai_provider_parses_response() {
        let server = setup_mock_openai("Hello, Alice!").await;

        // Create provider pointing at mock server
        let provider = prompt_sentinel::providers::OpenAiProvider::with_base_url(
            "test-key".to_string(),
            server.uri(),
        );

        let result = prompt_sentinel::providers::LlmProvider::complete(
            &provider,
            "Say hello to Alice",
            "gpt-4o-mini",
            0.7,
        )
        .await
        .unwrap();

        assert_eq!(result.text, "Hello, Alice!");
        assert_eq!(result.usage.prompt_tokens, 15);
        assert_eq!(result.usage.completion_tokens, 25);
        assert_eq!(result.usage.total_tokens, 40);
    }

    #[tokio::test]
    async fn test_webhook_provider() {
        let server = setup_mock_webhook("Webhook response!").await;

        let provider =
            prompt_sentinel::providers::WebhookProvider::new(format!("{}/complete", server.uri()));

        let result = prompt_sentinel::providers::LlmProvider::complete(
            &provider,
            "Hello",
            "custom-model",
            0.5,
        )
        .await
        .unwrap();

        assert_eq!(result.text, "Webhook response!");
        assert_eq!(result.usage.total_tokens, 30);
    }

    #[tokio::test]
    async fn test_openai_error_handling() {
        let server = setup_rate_limited_server().await;

        let provider = prompt_sentinel::providers::OpenAiProvider::with_base_url(
            "test-key".to_string(),
            server.uri(),
        );

        let result = prompt_sentinel::providers::LlmProvider::complete(
            &provider,
            "Hello",
            "gpt-4o-mini",
            0.7,
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("429"), "Expected 429 error, got: {}", err);
    }
}

// ─── Cost Calculation Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod cost_tests {
    use prompt_sentinel::providers::{calculate_cost, cost_per_million_tokens, TokenUsage};

    #[test]
    fn test_gpt4o_mini_cost() {
        let usage = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 200,
            total_tokens: 300,
        };
        let cost = calculate_cost("gpt-4o-mini", &usage);
        // input: 100/1M * 0.15 = 0.000015
        // output: 200/1M * 0.60 = 0.000120
        // total = 0.000135
        let expected = 0.000135;
        assert!(
            (cost - expected).abs() < 1e-9,
            "Expected ~{}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_unknown_model_zero_cost() {
        let usage = TokenUsage {
            prompt_tokens: 1000,
            completion_tokens: 1000,
            total_tokens: 2000,
        };
        let cost = calculate_cost("unknown-model-xyz", &usage);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn test_known_model_pricing_exists() {
        let known = vec![
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4",
            "gpt-3.5-turbo",
            "claude-3-5-sonnet-20241022",
            "claude-3-5-haiku-20241022",
        ];
        for model in known {
            let (input, output) = cost_per_million_tokens(model);
            assert!(input > 0.0, "Expected non-zero input price for {}", model);
            assert!(output > 0.0, "Expected non-zero output price for {}", model);
        }
    }
}

// ─── Assertion Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod assertion_tests {
    use prompt_sentinel::assertions::check_assertion;
    use prompt_sentinel::config::AssertionKind;
    use std::path::PathBuf;

    #[test]
    fn test_contains_pass() {
        let kind = AssertionKind::Contains("hello".to_string());
        let result = check_assertion(&kind, "Hello World", 100, "test", &PathBuf::new(), false);
        assert!(result.passed);
    }

    #[test]
    fn test_contains_fail() {
        let kind = AssertionKind::Contains("goodbye".to_string());
        let result = check_assertion(&kind, "Hello World", 100, "test", &PathBuf::new(), false);
        assert!(!result.passed);
    }

    #[test]
    fn test_not_contains_pass() {
        let kind = AssertionKind::NotContains("goodbye".to_string());
        let result = check_assertion(&kind, "Hello World", 100, "test", &PathBuf::new(), false);
        assert!(result.passed);
    }

    #[test]
    fn test_not_contains_fail() {
        let kind = AssertionKind::NotContains("hello".to_string());
        let result = check_assertion(&kind, "Hello World", 100, "test", &PathBuf::new(), false);
        assert!(!result.passed);
    }

    #[test]
    fn test_latency_max_pass() {
        let kind = AssertionKind::LatencyMax(5000);
        let result = check_assertion(&kind, "output", 3000, "test", &PathBuf::new(), false);
        assert!(result.passed);
    }

    #[test]
    fn test_latency_max_fail() {
        let kind = AssertionKind::LatencyMax(1000);
        let result = check_assertion(&kind, "output", 3000, "test", &PathBuf::new(), false);
        assert!(!result.passed);
    }

    #[test]
    fn test_regex_pass() {
        let kind = AssertionKind::Regex(r"\d{3}-\d{4}".to_string());
        let result = check_assertion(&kind, "Call 555-1234", 100, "test", &PathBuf::new(), false);
        assert!(result.passed);
    }

    #[test]
    fn test_regex_fail() {
        let kind = AssertionKind::Regex(r"^\d+$".to_string());
        let result = check_assertion(&kind, "not a number", 100, "test", &PathBuf::new(), false);
        assert!(!result.passed);
    }

    #[test]
    fn test_json_valid_pass() {
        let kind = AssertionKind::JsonValid;
        let result = check_assertion(
            &kind,
            r#"{"name": "Alice"}"#,
            100,
            "test",
            &PathBuf::new(),
            false,
        );
        assert!(result.passed);
    }

    #[test]
    fn test_json_valid_fail() {
        let kind = AssertionKind::JsonValid;
        let result = check_assertion(
            &kind,
            "not json at all",
            100,
            "test",
            &PathBuf::new(),
            false,
        );
        assert!(!result.passed);
    }

    #[test]
    fn test_min_length_pass() {
        let kind = AssertionKind::MinLength(5);
        let result = check_assertion(&kind, "Hello World", 100, "test", &PathBuf::new(), false);
        assert!(result.passed);
    }

    #[test]
    fn test_min_length_fail() {
        let kind = AssertionKind::MinLength(100);
        let result = check_assertion(&kind, "short", 100, "test", &PathBuf::new(), false);
        assert!(!result.passed);
    }

    #[test]
    fn test_max_length_pass() {
        let kind = AssertionKind::MaxLength(100);
        let result = check_assertion(&kind, "short", 100, "test", &PathBuf::new(), false);
        assert!(result.passed);
    }

    #[test]
    fn test_max_length_fail() {
        let kind = AssertionKind::MaxLength(3);
        let result = check_assertion(&kind, "too long", 100, "test", &PathBuf::new(), false);
        assert!(!result.passed);
    }
}

// ─── Config Validation Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod config_tests {
    use prompt_sentinel::config::{load_config, validate_config};

    #[test]
    fn test_valid_config() {
        let yaml = r#"
version: "1.0"
defaults:
  provider: "openai"
  model: "gpt-4o-mini"
  temperature: 0.7
tests:
  - id: "test-1"
    prompt: "Hello {{name}}"
    cases:
      - input:
          name: "Alice"
        assert:
          - type: "contains"
            value: "Alice"
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(tmp.path(), yaml).unwrap();
        let cfg = load_config(tmp.path().to_str().unwrap()).unwrap();
        let issues = validate_config(&cfg);
        assert!(issues.is_empty(), "Expected no issues, got: {:?}", issues);
    }

    #[test]
    fn test_unknown_provider() {
        let yaml = r#"
version: "1.0"
defaults:
  provider: "unknown-llm"
  model: "test"
  temperature: 0.7
tests:
  - id: "test-1"
    prompt: "Hello"
    cases:
      - input: {}
        assert:
          - type: "contains"
            value: "hello"
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(tmp.path(), yaml).unwrap();
        let cfg = load_config(tmp.path().to_str().unwrap()).unwrap();
        let issues = validate_config(&cfg);
        assert!(!issues.is_empty());
        assert!(issues[0].contains("Unknown default provider"));
    }

    #[test]
    fn test_duplicate_test_ids() {
        let yaml = r#"
version: "1.0"
defaults:
  provider: "openai"
  model: "gpt-4o-mini"
tests:
  - id: "same-id"
    prompt: "Hello"
    cases:
      - input: {}
        assert:
          - type: "contains"
            value: "hello"
  - id: "same-id"
    prompt: "World"
    cases:
      - input: {}
        assert:
          - type: "contains"
            value: "world"
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(tmp.path(), yaml).unwrap();
        let cfg = load_config(tmp.path().to_str().unwrap()).unwrap();
        let issues = validate_config(&cfg);
        assert!(issues.iter().any(|i| i.contains("Duplicate test ID")));
    }

    #[test]
    fn test_typo_suggestion() {
        let yaml = r#"
version: "1.0"
defaults:
  provider: "openai"
  model: "gpt-4o-mini"
tests:
  - id: "test-1"
    prompt: "Hello"
    cases:
      - input: {}
        assert:
          - type: "contians"
            value: "hello"
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(tmp.path(), yaml).unwrap();
        let cfg = load_config(tmp.path().to_str().unwrap()).unwrap();
        let issues = validate_config(&cfg);
        assert!(issues.iter().any(|i| i.contains("Did you mean")));
    }

    #[test]
    fn test_unresolved_template_variable() {
        let yaml = r#"
version: "1.0"
defaults:
  provider: "openai"
  model: "gpt-4o-mini"
tests:
  - id: "test-1"
    prompt: "Hello {{name}} and {{other}}"
    cases:
      - input:
          name: "Alice"
        assert:
          - type: "contains"
            value: "Alice"
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(tmp.path(), yaml).unwrap();
        let cfg = load_config(tmp.path().to_str().unwrap()).unwrap();
        let issues = validate_config(&cfg);
        assert!(issues.iter().any(|i| i.contains("unresolved template")));
    }

    #[test]
    fn test_webhook_provider_is_valid() {
        let yaml = r#"
version: "1.0"
defaults:
  provider: "webhook"
  model: "custom"
tests:
  - id: "test-1"
    prompt: "Hello"
    cases:
      - input: {}
        assert:
          - type: "contains"
            value: "hello"
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(tmp.path(), yaml).unwrap();
        let cfg = load_config(tmp.path().to_str().unwrap()).unwrap();
        let issues = validate_config(&cfg);
        // webhook is a known provider — should not show "Unknown provider" error
        assert!(!issues
            .iter()
            .any(|i| i.contains("Unknown default provider")));
    }
}

// ─── Template Rendering Tests ────────────────────────────────────────────────

#[cfg(test)]
mod template_tests {
    use prompt_sentinel::config::render_prompt;
    use std::collections::HashMap;

    #[test]
    fn test_basic_render() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Alice".to_string());
        let result = render_prompt("Hello {{name}}!", &vars);
        assert_eq!(result, "Hello Alice!");
    }

    #[test]
    fn test_multiple_vars() {
        let mut vars = HashMap::new();
        vars.insert("first".to_string(), "Jane".to_string());
        vars.insert("last".to_string(), "Doe".to_string());
        let result = render_prompt("{{first}} {{last}}", &vars);
        assert_eq!(result, "Jane Doe");
    }

    #[test]
    fn test_no_vars() {
        let vars = HashMap::new();
        let result = render_prompt("No variables here", &vars);
        assert_eq!(result, "No variables here");
    }

    #[test]
    fn test_repeated_var() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), "42".to_string());
        let result = render_prompt("{{x}} + {{x}} = ?", &vars);
        assert_eq!(result, "42 + 42 = ?");
    }
}

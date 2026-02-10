use crate::assertions::{check_assertion, AssertionResult};
use crate::config::{render_prompt, AssertionKind, Config};
use crate::providers::{self, LlmProvider, TokenUsage};

use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration, Instant};

/// Output verbosity level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verbosity {
    /// Only show summary line (pass/fail counts)
    Quiet,
    /// Default: per-test status + assertions
    Normal,
    /// Show everything including full LLM output
    Verbose,
}

/// The result of running a single test case.
#[derive(Debug, Serialize)]
pub struct CaseResult {
    pub test_id: String,
    pub input_label: String,
    pub passed: bool,
    pub latency_ms: u64,
    pub assertions: Vec<AssertionDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub retries: u32,
    pub tokens: TokenUsage,
    pub cost_usd: f64,
    #[serde(skip)]
    #[allow(dead_code)]
    pub model: String,
    /// Full LLM output (included in JSON, shown in --verbose)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AssertionDetail {
    pub label: String,
    pub passed: bool,
    pub detail: String,
}

impl From<AssertionResult> for AssertionDetail {
    fn from(r: AssertionResult) -> Self {
        Self {
            label: r.label,
            passed: r.passed,
            detail: r.detail,
        }
    }
}

/// Max retry attempts for transient API errors.
const MAX_RETRIES: u32 = 3;
/// Base delay for exponential backoff (doubles each retry: 500ms → 1s → 2s).
const BASE_RETRY_DELAY_MS: u64 = 500;

/// Attempt an LLM completion with retry + exponential backoff + timeout.
async fn complete_with_retry(
    provider: &dyn LlmProvider,
    prompt: &str,
    model: &str,
    temperature: f64,
    timeout_ms: u64,
) -> (Result<providers::CompletionResult, anyhow::Error>, u32) {
    let mut retries = 0;
    let timeout_dur = Duration::from_millis(timeout_ms);

    loop {
        let attempt =
            time::timeout(timeout_dur, provider.complete(prompt, model, temperature)).await;

        let result = match attempt {
            Ok(inner) => inner,
            Err(_) => Err(anyhow::anyhow!("request timed out after {}ms", timeout_ms)),
        };

        match result {
            Ok(output) => return (Ok(output), retries),
            Err(e) => {
                let err_msg = e.to_string();
                let is_transient = err_msg.contains("429")
                    || err_msg.contains("500")
                    || err_msg.contains("502")
                    || err_msg.contains("503")
                    || err_msg.contains("timeout")
                    || err_msg.contains("timed out")
                    || err_msg.contains("connection");

                if is_transient && retries < MAX_RETRIES {
                    retries += 1;
                    let delay = BASE_RETRY_DELAY_MS * 2u64.pow(retries - 1);
                    time::sleep(Duration::from_millis(delay)).await;
                    continue;
                }

                return (Err(e), retries);
            }
        }
    }
}

/// Run all tests from the config in parallel (bounded by concurrency limit).
pub async fn run_all_tests(
    config: &Config,
    provider: Arc<dyn LlmProvider>,
    concurrency: usize,
    verbosity: Verbosity,
    json_mode: bool,
    update_snapshots: bool,
    timeout_ms: u64,
    filter: Option<&str>,
) -> Vec<CaseResult> {
    // Filter tests by ID if --filter is specified
    let tests: Vec<_> = config
        .tests
        .iter()
        .filter(|t| match filter {
            Some(pattern) => t.id.contains(pattern),
            None => true,
        })
        .collect();

    let total_cases: usize = tests.iter().map(|t| t.cases.len()).sum();

    // Show progress bar only in Normal/Verbose mode (not quiet, not json)
    let show_progress = !json_mode && verbosity != Verbosity::Quiet;
    let pb = if show_progress && total_cases > 0 {
        let pb = ProgressBar::new(total_cases as u64);
        pb.set_style(
            ProgressStyle::with_template(
                "  {spinner:.cyan} [{bar:30.green/dim}] {pos}/{len} tests ({eta} remaining)",
            )
            .unwrap()
            .progress_chars("█▓░"),
        );
        pb.enable_steady_tick(Duration::from_millis(120));
        Some(pb)
    } else {
        None
    };

    let pb_arc = pb.as_ref().map(|p| Arc::new(p.clone()));

    let mut handles: Vec<JoinHandle<CaseResult>> = Vec::new();
    let semaphore = Arc::new(Semaphore::new(concurrency));

    let default_model = config.defaults.model.clone();
    let default_temp = config.defaults.temperature;
    let snapshot_dir = PathBuf::from(".snapshots");

    for test in &tests {
        let test_id = test.id.clone();
        let prompt_template = test.prompt.clone();
        let model = test.model.clone().unwrap_or_else(|| default_model.clone());

        for (ci, case) in test.cases.iter().enumerate() {
            let provider = Arc::clone(&provider);
            let semaphore = Arc::clone(&semaphore);
            let pb_arc = pb_arc.clone();
            let test_id = test_id.clone();
            let prompt_template = prompt_template.clone();
            let model = model.clone();
            let input = case.input.clone();
            let raw_assertions = case.assertions.clone();
            let temperature = default_temp;
            let snapshot_dir = snapshot_dir.clone();
            let snapshot_key = format!("{}_case{}", test_id, ci);

            let handle = tokio::spawn(async move {
                let _permit = semaphore.acquire().await.expect("semaphore closed");

                let rendered_prompt = render_prompt(&prompt_template, &input);
                let input_label = input
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ");

                let parsed_assertions: Vec<AssertionKind> = raw_assertions
                    .iter()
                    .filter_map(|a| AssertionKind::from_raw(&a.kind, &a.value).ok())
                    .collect();

                let start = Instant::now();
                let (result, retries) = complete_with_retry(
                    &*provider,
                    &rendered_prompt,
                    &model,
                    temperature,
                    timeout_ms,
                )
                .await;
                let latency_ms = start.elapsed().as_millis() as u64;

                let case_result = match result {
                    Ok(completion) => {
                        let cost = providers::calculate_cost(&model, &completion.usage);
                        let output_text = completion.text.clone();

                        let assertion_results: Vec<AssertionDetail> = parsed_assertions
                            .iter()
                            .map(|kind| {
                                check_assertion(
                                    kind,
                                    &completion.text,
                                    latency_ms,
                                    &snapshot_key,
                                    &snapshot_dir,
                                    update_snapshots,
                                )
                                .into()
                            })
                            .collect();

                        let all_passed = assertion_results.iter().all(|a| a.passed);

                        CaseResult {
                            test_id,
                            input_label,
                            passed: all_passed,
                            latency_ms,
                            assertions: assertion_results,
                            error: None,
                            retries,
                            tokens: completion.usage,
                            cost_usd: cost,
                            model,
                            output: Some(output_text),
                        }
                    }
                    Err(e) => CaseResult {
                        test_id,
                        input_label,
                        passed: false,
                        latency_ms,
                        assertions: vec![],
                        error: Some(e.to_string()),
                        retries,
                        tokens: TokenUsage::default(),
                        cost_usd: 0.0,
                        model,
                        output: None,
                    },
                };

                if let Some(ref pb) = pb_arc {
                    pb.inc(1);
                }

                case_result
            });

            handles.push(handle);
        }
    }

    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(case_result) => results.push(case_result),
            Err(e) => results.push(CaseResult {
                test_id: "unknown".to_string(),
                input_label: "unknown".to_string(),
                passed: false,
                latency_ms: 0,
                assertions: vec![],
                error: Some(format!("Task join error: {}", e)),
                retries: 0,
                tokens: TokenUsage::default(),
                cost_usd: 0.0,
                model: "unknown".to_string(),
                output: None,
            }),
        }
    }

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }

    results
}

// ─── Printing Logic (moved from main.rs) ────────────────────────────────────

pub fn print_results(results: &[CaseResult], verbosity: Verbosity) {
    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = total - passed;
    let total_cost: f64 = results.iter().map(|r| r.cost_usd).sum();
    let total_tokens: u32 = results.iter().map(|r| r.tokens.total_tokens).sum();

    if verbosity == Verbosity::Quiet {
        // Quiet mode: one-liner summary only
        let status = if failed == 0 {
            "✓".green().bold()
        } else {
            "✗".red().bold()
        };
        let cost_str = if total_cost > 0.0 {
            format!(" · ${:.6}", total_cost)
        } else {
            String::new()
        };
        println!("  {} {}/{} passed{}", status, passed, total, cost_str);
        return;
    }

    // Normal and Verbose modes
    println!(
        "{}",
        "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_black()
    );
    // Don't print the header title here ("Prompt Sentinel — Test Results")
    // because watch mode prints its own header.
    // Or we keep it. Let's keep it simple.

    for result in results {
        let status = if result.passed {
            "PASS".green().bold()
        } else {
            "FAIL".red().bold()
        };

        let retry_info = if result.retries > 0 {
            format!(" ({}x retried)", result.retries)
        } else {
            String::new()
        };

        let cost_info = if result.cost_usd > 0.0 {
            format!(" · ${:.5}", result.cost_usd)
        } else {
            String::new()
        };

        let token_info = if result.tokens.total_tokens > 0 {
            format!(" · {}tok", result.tokens.total_tokens)
        } else {
            String::new()
        };

        println!(
            "  {} │ {} │ {} │ {}ms{}{}{}",
            status,
            result.test_id.bold(),
            result.input_label.bright_black(),
            result.latency_ms,
            retry_info.yellow(),
            token_info.bright_black(),
            cost_info.bright_black()
        );

        if let Some(ref err) = result.error {
            println!("       {} {}", "error:".red(), err);
        }

        for assertion in &result.assertions {
            let icon = if assertion.passed {
                "✓".green()
            } else {
                "✗".red()
            };
            println!(
                "       {} {} — {}",
                icon,
                assertion.label.dimmed(),
                assertion.detail
            );
        }

        // Verbose mode: show full LLM output
        if verbosity == Verbosity::Verbose {
            if let Some(ref output) = result.output {
                println!(
                    "       {} {}",
                    "output:".bright_cyan().bold(),
                    "─".repeat(40).bright_black()
                );
                for line in output.lines() {
                    println!("       │ {}", line.bright_black());
                }
                println!("       {}", "─".repeat(48).bright_black());
            }
        }

        println!();
    }

    println!(
        "{}",
        "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_black()
    );
    println!(
        "  {} {} passed, {} {} failed, {} total",
        "●".green(),
        passed,
        "●".red(),
        failed,
        total
    );
    if total_tokens > 0 || total_cost > 0.0 {
        println!(
            "  {} {} tokens · ${:.6} estimated cost",
            "💰".bright_yellow(),
            total_tokens,
            total_cost
        );
    }
    println!(
        "{}",
        "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_black()
    );
    println!();
}

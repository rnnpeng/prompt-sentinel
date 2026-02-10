mod assertions;
mod config;
mod providers;
mod report;
mod runner;
mod watch;

use clap::{Parser, Subcommand};
use colored::*;
use runner::Verbosity;
use serde::Serialize;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "sentinel",
    about = "Prompt Sentinel — LLM prompt regression testing CLI",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run prompt regression tests
    Run {
        /// Path to the YAML test file (default: tests.yaml)
        #[arg(short, long, default_value = "tests.yaml")]
        file: String,

        /// Output results as JSON instead of colored text
        #[arg(long, default_value_t = false)]
        json: bool,

        /// Upload results to Prompt Sentinel dashboard
        #[arg(long, default_value_t = false)]
        upload: bool,

        /// API token for dashboard authentication (or set SENTINEL_TOKEN env var)
        #[arg(long)]
        token: Option<String>,

        /// Max number of concurrent API requests (default: 5)
        #[arg(short, long, default_value_t = 5)]
        concurrency: usize,

        /// Per-request timeout in milliseconds (default: 30000)
        #[arg(short, long, default_value_t = 30000)]
        timeout: u64,

        /// Update all snapshot files to match current output
        #[arg(long, default_value_t = false)]
        update_snapshots: bool,

        /// Skip config validation before running
        #[arg(long, default_value_t = false)]
        no_validate: bool,

        /// Only run tests whose ID contains this pattern
        #[arg(long)]
        filter: Option<String>,

        /// Generate an HTML report file
        #[arg(long)]
        report: Option<Option<String>>,

        /// Show full LLM output for each test
        #[arg(short, long, default_value_t = false)]
        verbose: bool,

        /// Only show summary (no per-test output)
        #[arg(short, long, default_value_t = false)]
        quiet: bool,
    },

    /// Watch for file changes and re-run tests automatically
    Watch {
        /// Path to the YAML test file (default: tests.yaml)
        #[arg(short, long, default_value = "tests.yaml")]
        file: String,

        /// Output results as JSON
        #[arg(long, default_value_t = false)]
        json: bool,

        /// Upload results to Prompt Sentinel dashboard (default: false)
        #[arg(long, default_value_t = false)]
        upload: bool,

        /// API token for dashboard authentication
        #[arg(long)]
        token: Option<String>,

        /// Max number of concurrent API requests (default: 5)
        #[arg(short, long, default_value_t = 5)]
        concurrency: usize,

        /// Per-request timeout in milliseconds (default: 30000)
        #[arg(short, long, default_value_t = 30000)]
        timeout: u64,

        /// Update snapshots on every run (careful!)
        #[arg(long, default_value_t = false)]
        update_snapshots: bool,

        /// Skip config validation
        #[arg(long, default_value_t = false)]
        no_validate: bool,

        /// Only run tests whose ID contains this pattern
        #[arg(long)]
        filter: Option<String>,

        /// Generate an HTML report file
        #[arg(long)]
        report: Option<Option<String>>,

        /// Show full LLM output for each test
        #[arg(short, long, default_value_t = false)]
        verbose: bool,

        /// Only show summary
        #[arg(short, long, default_value_t = false)]
        quiet: bool,
    },

    /// Validate a test configuration file without running any tests
    Validate {
        /// Path to the YAML test file (default: tests.yaml)
        #[arg(short, long, default_value = "tests.yaml")]
        file: String,
    },

    /// Initialize a new Prompt Sentinel project in the current directory
    Init,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            file,
            json,
            upload,
            token,
            concurrency,
            timeout,
            update_snapshots,
            no_validate,
            filter,
            report: report_flag,
            verbose,
            quiet,
        } => {
            // Resolve verbosity
            let verbosity = if quiet {
                Verbosity::Quiet
            } else if verbose {
                Verbosity::Verbose
            } else {
                Verbosity::Normal
            };

            // 1. Load config
            let cfg = config::load_config(&file)?;

            // 2. Auto-validate (unless --no-validate)
            if !no_validate {
                let issues = config::validate_config(&cfg);
                if !issues.is_empty() {
                    if !json {
                        eprintln!(
                            "\n  {} Config validation found {} issue(s):\n",
                            "✗".red().bold(),
                            issues.len()
                        );
                        for issue in &issues {
                            eprintln!("    {} {}", "•".red(), issue);
                        }
                        eprintln!(
                            "\n  {} Fix these issues or use {} to skip.\n",
                            "→".bright_cyan(),
                            "--no-validate".bold()
                        );
                    }
                    std::process::exit(1);
                }
            }

            // 3. Create provider
            let provider_name = cfg.defaults.provider.as_str();
            let provider = providers::create_provider(provider_name)?;
            let provider = Arc::from(provider);

            // 4. Show filter info + run tests
            let filter_ref = filter.as_deref();

            if !json && verbosity != Verbosity::Quiet {
                let all_tests: usize = cfg.tests.iter().map(|t| t.cases.len()).sum();
                let filtered_tests: usize = cfg
                    .tests
                    .iter()
                    .filter(|t| match filter_ref {
                        Some(p) => t.id.contains(p),
                        None => true,
                    })
                    .map(|t| t.cases.len())
                    .sum();

                if let Some(ref pat) = filter {
                    println!(
                        "\n  {} Filtering tests by '{}': {} of {} case(s) matched",
                        "🔍".bright_cyan(),
                        pat.bold(),
                        filtered_tests,
                        all_tests
                    );
                }

                println!(
                    "\n  {} Running {} test case(s) with concurrency={}, timeout={}ms...\n",
                    "⚡".bright_yellow(),
                    filtered_tests,
                    concurrency,
                    timeout
                );
            }

            let results = runner::run_all_tests(
                &cfg,
                provider,
                concurrency,
                verbosity,
                json,
                update_snapshots,
                timeout,
                filter_ref,
            )
            .await;

            // 5. Output results
            if json {
                let json_output = serde_json::to_string_pretty(&results)?;
                println!("{}", json_output);
            } else {
                runner::print_results(&results, verbosity);
            }

            // 6. Generate HTML report
            if let Some(report_path) = report_flag {
                let path = report_path.unwrap_or_else(|| "report.html".to_string());
                let path = std::path::Path::new(&path);
                let generated = report::generate_report(&results, path)?;
                if !json {
                    println!(
                        "  {} HTML report saved to {}",
                        "📊".bright_cyan(),
                        generated.bold()
                    );
                    println!();
                }
            }

            // 7. Upload
            if upload {
                let resolved_token = token
                    .or_else(|| std::env::var("SENTINEL_TOKEN").ok())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Upload requires a token. Use --token <TOKEN> or set SENTINEL_TOKEN env var."
                        )
                    })?;
                upload_results(&results, &resolved_token).await?;
            }

            // 8. Exit code
            let all_passed = results.iter().all(|r| r.passed);
            if !all_passed {
                std::process::exit(1);
            }
        }

        Commands::Watch {
            file,
            json,
            upload,
            token,
            concurrency,
            timeout,
            update_snapshots,
            no_validate,
            filter,
            report: report_flag,
            verbose,
            quiet,
        } => {
            let verbosity = if quiet {
                Verbosity::Quiet
            } else if verbose {
                Verbosity::Verbose
            } else {
                Verbosity::Normal
            };

            watch::run_watch_loop(
                &file,
                json,
                upload,
                token,
                concurrency,
                timeout,
                update_snapshots,
                no_validate,
                filter,
                report_flag,
                verbosity,
            )
            .await?;
        }

        Commands::Validate { file } => {
            run_validate(&file)?;
        }

        Commands::Init => {
            run_init()?;
        }
    }

    Ok(())
}

// ─── sentinel validate ──────────────────────────────────────────────────────

fn run_validate(file: &str) -> anyhow::Result<()> {
    println!();
    println!(
        "  {} {} {}",
        "⚡".bright_yellow(),
        "Validating".bold(),
        file.bold()
    );
    println!();

    let cfg = match config::load_config(file) {
        Ok(cfg) => cfg,
        Err(e) => {
            println!("  {} {}", "✗".red().bold(), e);
            println!();
            std::process::exit(1);
        }
    };
    println!("  {} YAML syntax is valid", "✓".green().bold());

    let issues = config::validate_config(&cfg);

    if issues.is_empty() {
        let total_cases: usize = cfg.tests.iter().map(|t| t.cases.len()).sum();
        let total_assertions: usize = cfg
            .tests
            .iter()
            .flat_map(|t| &t.cases)
            .map(|c| c.assertions.len())
            .sum();

        println!("  {} All checks passed", "✓".green().bold());
        println!();
        println!(
            "  {} {} test(s), {} case(s), {} assertion(s)",
            "→".bright_cyan(),
            cfg.tests.len(),
            total_cases,
            total_assertions
        );
        println!(
            "  {} Provider: {}, Model: {}",
            "→".bright_cyan(),
            cfg.defaults.provider.bold(),
            cfg.defaults.model.bold()
        );
    } else {
        println!(
            "  {} Found {} issue(s):",
            "✗".red().bold(),
            issues.len()
        );
        println!();
        for issue in &issues {
            println!("    {} {}", "•".red(), issue);
        }
        println!();
        std::process::exit(1);
    }

    println!();
    Ok(())
}

// ─── sentinel init ───────────────────────────────────────────────────────────

fn run_init() -> anyhow::Result<()> {
    use std::fs;
    use std::path::Path;

    println!();
    println!(
        "  {} {}",
        "⚡".bright_yellow(),
        "Prompt Sentinel — Project Init".bold()
    );
    println!();

    let tests_path = Path::new("tests.yaml");
    if tests_path.exists() {
        println!(
            "  {} tests.yaml already exists, skipping.",
            "⚠".yellow()
        );
    } else {
        let template = r#"version: "1.0"

defaults:
  provider: "openai"
  model: "gpt-4o-mini"
  temperature: 0.7

tests:
  - id: "hello-world"
    prompt: "Say hello to {{name}} in one short sentence."
    cases:
      - input:
          name: "Alice"
        assert:
          - type: "contains"
            value: "Alice"
          - type: "not-contains"
            value: "As an AI language model"
          - type: "latency_max"
            value: 10000
          - type: "min_length"
            value: 10
          - type: "max_length"
            value: 500
"#;
        fs::write(tests_path, template)?;
        println!("  {} Created tests.yaml", "✓".green().bold());
    }

    let env_example_path = Path::new(".env.example");
    if env_example_path.exists() {
        println!(
            "  {} .env.example already exists, skipping.",
            "⚠".yellow()
        );
    } else {
        let env_template = r#"# Prompt Sentinel — API Keys
# Copy this file to .env and fill in your keys.

# OpenAI (required if using provider: "openai")
OPENAI_API_KEY=sk-your-key-here

# Anthropic (required if using provider: "anthropic")
ANTHROPIC_API_KEY=sk-ant-your-key-here

# Custom webhook (required if using provider: "webhook")
# WEBHOOK_URL=http://localhost:8080/complete

# Sentinel Dashboard (optional — for `sentinel run --upload`)
# SENTINEL_TOKEN=your-dashboard-token
"#;
        fs::write(env_example_path, env_template)?;
        println!("  {} Created .env.example", "✓".green().bold());
    }

    let env_path = Path::new(".env");
    if env_path.exists() {
        println!(
            "  {} .env already exists, skipping.",
            "⚠".yellow()
        );
    } else {
        fs::copy(env_example_path, env_path)?;
        println!(
            "  {} Created .env (copy of .env.example)",
            "✓".green().bold()
        );
    }

    let gitignore_path = Path::new(".gitignore");
    let existing_gitignore = if gitignore_path.exists() {
        std::fs::read_to_string(gitignore_path)?
    } else {
        String::new()
    };

    let mut additions = Vec::new();
    if !existing_gitignore.lines().any(|l| l.trim() == ".env") {
        additions.push(".env");
    }

    if !additions.is_empty() {
        let mut content = existing_gitignore;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        for entry in &additions {
            content.push_str(entry);
            content.push('\n');
        }
        fs::write(gitignore_path, content)?;
        println!(
            "  {} Added {} to .gitignore",
            "✓".green().bold(),
            additions.join(", ")
        );
    }

    println!();
    println!("  {} Next steps:", "→".bright_cyan());
    println!("    1. Edit {} and add your API key", ".env".bold());
    println!("    2. Customize {} with your prompts", "tests.yaml".bold());
    println!(
        "    3. Run {} to check for errors",
        "sentinel validate".bold().cyan()
    );
    println!("    4. Run {}", "sentinel run".bold().green());
    println!();

    Ok(())
}

// ─── Upload ──────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ReportUpload<'a> {
    total: usize,
    passed: usize,
    failed: usize,
    results: &'a [runner::CaseResult],
}

async fn upload_results(
    results: &[runner::CaseResult],
    token: &str,
) -> anyhow::Result<()> {
    let api_url = std::env::var("SENTINEL_API_URL")
        .unwrap_or_else(|_| "https://app.promptsentinel.com/api/v1/reports".to_string());

    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();
    let payload = ReportUpload {
        total,
        passed,
        failed: total - passed,
        results,
    };

    println!("  {} Uploading results to dashboard...", "↑".bright_cyan());

    let client = reqwest::Client::new();
    let resp = client
        .post(&api_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await?;

    if resp.status().is_success() {
        println!(
            "  {} Results uploaded successfully!",
            "✓".green().bold()
        );
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Dashboard upload failed ({}): {}",
            status,
            body
        ));
    }

    Ok(())
}

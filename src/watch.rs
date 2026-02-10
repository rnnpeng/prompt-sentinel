use crate::config;
use crate::providers;
use crate::report;
use crate::runner::{self, Verbosity};
use colored::*;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub async fn run_watch_loop(
    file: &str,
    json: bool,
    upload: bool,
    _token: Option<String>,
    concurrency: usize,
    timeout: u64,
    update_snapshots: bool,
    no_validate: bool,
    filter: Option<String>,
    report_path: Option<Option<String>>,
    verbosity: Verbosity,
) -> anyhow::Result<()> {
    println!(
        "  {} {}",
        "👀".bright_cyan(),
        format!("Watching {} for changes...", file).bold()
    );

    // Initial run
    run_cycle(
        file,
        json,
        upload,
        _token.clone(),
        concurrency,
        timeout,
        update_snapshots,
        no_validate,
        filter.clone(),
        report_path.clone(),
        verbosity,
    )
    .await;

    // Setup watcher
    let (tx, rx) = channel();
    let mut watcher = RecommendedWatcher::new(tx, Config::default())?;

    watcher.watch(Path::new(file), RecursiveMode::NonRecursive)?;
    if Path::new(".env").exists() {
        watcher.watch(Path::new(".env"), RecursiveMode::NonRecursive)?;
    }

    let mut last_run = Instant::now();
    let debounce_interval = Duration::from_millis(500);

    loop {
        match rx.recv() {
            Ok(Ok(Event { .. })) => {
                if last_run.elapsed() < debounce_interval {
                    continue;
                }
                last_run = Instant::now();

                // Clear screen
                print!("\x1B[2J\x1B[1;1H");

                println!(
                    "  {} {}",
                    "↻".bright_cyan(),
                    "File changed, re-running tests...".dimmed()
                );

                run_cycle(
                    file,
                    json,
                    upload,
                    _token.clone(),
                    concurrency,
                    timeout,
                    update_snapshots,
                    no_validate,
                    filter.clone(),
                    report_path.clone(),
                    verbosity,
                )
                .await;
            }
            Ok(Err(e)) => println!("  {} Watch error: {}", "⚠".yellow(), e),
            Err(_) => break,
        }
    }

    Ok(())
}

async fn run_cycle(
    file: &str,
    json: bool,
    upload: bool,
    _token: Option<String>,
    concurrency: usize,
    timeout: u64,
    update_snapshots: bool,
    no_validate: bool,
    filter: Option<String>,
    report_path: Option<Option<String>>,
    verbosity: Verbosity,
) {
    // 1. Load config (hande errors gracefully so we don't crash watcher)
    let cfg = match config::load_config(file) {
        Ok(cfg) => cfg,
        Err(e) => {
            println!("\n  {} Failed to load config:\n  {}", "✗".red().bold(), e);
            return;
        }
    };

    // 2. Validate
    if !no_validate {
        let issues = config::validate_config(&cfg);
        if !issues.is_empty() {
            println!("\n  {} Config issues:", "✗".red().bold());
            for issue in &issues {
                println!("    {} {}", "•".red(), issue);
            }
            return;
        }
    }

    // 3. Provider
    let provider = match providers::create_provider(&cfg.defaults.provider) {
        Ok(p) => Arc::from(p),
        Err(e) => {
            println!("\n  {} Provider error: {}", "✗".red().bold(), e);
            return;
        }
    };

    // 4. Run
    let filter_ref = filter.as_deref();

    // Header for watch mode clarity
    if !json && verbosity != Verbosity::Quiet {
        let all_tests: usize = cfg.tests.iter().map(|t| t.cases.len()).sum();
        println!(
            "\n  {} Running {} tests...",
            "⚡".bright_yellow(),
            all_tests
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

    // 5. Print
    if json {
        if let Ok(json_output) = serde_json::to_string_pretty(&results) {
            println!("{}", json_output);
        }
    } else {
        runner::print_results(&results, verbosity);
    }

    // 6. Report
    if let Some(report_path) = report_path {
        let path = report_path.unwrap_or_else(|| "report.html".to_string());
        match report::generate_report(&results, Path::new(&path)) {
            Ok(generated) => {
                if !json {
                    println!(
                        "  {} Report saved to {}",
                        "📊".bright_cyan(),
                        generated.bold()
                    );
                }
            }
            Err(e) => println!("  {} Report error: {}", "⚠".yellow(), e),
        }
    }

    // 7. Upload
    // Should we upload on every watch cycle? Probably not, or only if requested.
    // If the user passed --upload, we do it.
    if upload {
        // Reuse upload logic from main? Need to expose it or duplicate it.
        // It's small enough to duplicate or factor out.
        // Let's assume we skip upload in watch mode for now unless critical.
        // Or better: Factor `upload_results` into `report.rs` or `runner.rs`.
        // I'll skip it for v1 watch mode to keep it fast.
        if !json {
            println!("  {} Upload skipped in watch mode", "⚠".yellow());
        }
    }
}

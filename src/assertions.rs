use crate::config::AssertionKind;
use std::path::Path;

/// Result of a single assertion check.
#[derive(Debug)]
pub struct AssertionResult {
    pub passed: bool,
    pub label: String,
    pub detail: String,
}

/// Evaluate an assertion against the LLM output and measured latency.
pub fn check_assertion(
    kind: &AssertionKind,
    output: &str,
    latency_ms: u64,
    snapshot_key: &str,
    snapshot_dir: &Path,
    update_snapshots: bool,
) -> AssertionResult {
    match kind {
        AssertionKind::Contains(expected) => {
            let lower_output = output.to_lowercase();
            let lower_expected = expected.to_lowercase();
            let passed = lower_output.contains(&lower_expected);
            AssertionResult {
                passed,
                label: format!("contains \"{}\"", expected),
                detail: if passed {
                    "found in output".to_string()
                } else {
                    "NOT found in output".to_string()
                },
            }
        }
        AssertionKind::NotContains(unexpected) => {
            let lower_output = output.to_lowercase();
            let lower_unexpected = unexpected.to_lowercase();
            let passed = !lower_output.contains(&lower_unexpected);
            AssertionResult {
                passed,
                label: format!("not-contains \"{}\"", unexpected),
                detail: if passed {
                    "correctly absent from output".to_string()
                } else {
                    "unexpectedly found in output".to_string()
                },
            }
        }
        AssertionKind::LatencyMax(max_ms) => {
            let passed = latency_ms <= *max_ms;
            AssertionResult {
                passed,
                label: format!("latency_max {}ms", max_ms),
                detail: format!("actual: {}ms", latency_ms),
            }
        }
        AssertionKind::Snapshot => {
            check_snapshot(output, snapshot_key, snapshot_dir, update_snapshots)
        }
        AssertionKind::Regex(pattern) => {
            let re = regex::Regex::new(pattern).expect("regex already validated at parse time");
            let passed = re.is_match(output);
            AssertionResult {
                passed,
                label: format!("regex /{}/", pattern),
                detail: if passed {
                    "pattern matched".to_string()
                } else {
                    "pattern NOT matched".to_string()
                },
            }
        }
        AssertionKind::JsonValid => {
            let passed = serde_json::from_str::<serde_json::Value>(output.trim()).is_ok();
            AssertionResult {
                passed,
                label: "json_valid".to_string(),
                detail: if passed {
                    "output is valid JSON".to_string()
                } else {
                    "output is NOT valid JSON".to_string()
                },
            }
        }
        AssertionKind::MinLength(min) => {
            let len = output.trim().len() as u64;
            let passed = len >= *min;
            AssertionResult {
                passed,
                label: format!("min_length {}", min),
                detail: format!("actual: {} chars", len),
            }
        }
        AssertionKind::MaxLength(max) => {
            let len = output.trim().len() as u64;
            let passed = len <= *max;
            AssertionResult {
                passed,
                label: format!("max_length {}", max),
                detail: format!("actual: {} chars", len),
            }
        }
    }
}

// ─── Snapshot logic ──────────────────────────────────────────────────────────

fn check_snapshot(
    output: &str,
    snapshot_key: &str,
    snapshot_dir: &Path,
    update: bool,
) -> AssertionResult {
    let snap_file = snapshot_dir.join(format!("{}.snap", snapshot_key));

    if update {
        if let Err(e) = std::fs::create_dir_all(snapshot_dir) {
            return AssertionResult {
                passed: false,
                label: "snapshot".to_string(),
                detail: format!("failed to create snapshot dir: {}", e),
            };
        }
        if let Err(e) = std::fs::write(&snap_file, output) {
            return AssertionResult {
                passed: false,
                label: "snapshot".to_string(),
                detail: format!("failed to write snapshot: {}", e),
            };
        }
        return AssertionResult {
            passed: true,
            label: "snapshot".to_string(),
            detail: "updated".to_string(),
        };
    }

    if !snap_file.exists() {
        if let Err(e) = std::fs::create_dir_all(snapshot_dir) {
            return AssertionResult {
                passed: false,
                label: "snapshot".to_string(),
                detail: format!("failed to create snapshot dir: {}", e),
            };
        }
        if let Err(e) = std::fs::write(&snap_file, output) {
            return AssertionResult {
                passed: false,
                label: "snapshot".to_string(),
                detail: format!("failed to write snapshot: {}", e),
            };
        }
        return AssertionResult {
            passed: true,
            label: "snapshot".to_string(),
            detail: "created (first run)".to_string(),
        };
    }

    let existing = match std::fs::read_to_string(&snap_file) {
        Ok(s) => s,
        Err(e) => {
            return AssertionResult {
                passed: false,
                label: "snapshot".to_string(),
                detail: format!("failed to read snapshot: {}", e),
            };
        }
    };

    let normalized_existing = existing.trim();
    let normalized_output = output.trim();

    if normalized_output == normalized_existing {
        AssertionResult {
            passed: true,
            label: "snapshot".to_string(),
            detail: "matches saved snapshot".to_string(),
        }
    } else {
        let diff = diff_summary(normalized_existing, normalized_output);
        AssertionResult {
            passed: false,
            label: "snapshot".to_string(),
            detail: format!(
                "differs from snapshot. {}. Run with --update-snapshots to accept.",
                diff
            ),
        }
    }
}

fn diff_summary(expected: &str, actual: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();

    for (i, (e, a)) in exp_lines.iter().zip(act_lines.iter()).enumerate() {
        if e != a {
            return format!(
                "First diff at line {}: expected '{}', got '{}'",
                i + 1,
                truncate(e, 40),
                truncate(a, 40)
            );
        }
    }

    if exp_lines.len() != act_lines.len() {
        format!(
            "Line count differs: snapshot has {}, output has {}",
            exp_lines.len(),
            act_lines.len()
        )
    } else {
        "Content differs".to_string()
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}…", &s[..max])
    } else {
        s.to_string()
    }
}

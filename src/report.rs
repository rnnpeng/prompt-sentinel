use crate::runner::CaseResult;
use std::path::Path;

/// Generate a self-contained HTML report file from test results.
pub fn generate_report(
    results: &[CaseResult],
    output_path: &Path,
) -> anyhow::Result<String> {
    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = total - passed;
    let pass_pct = if total > 0 {
        (passed as f64 / total as f64 * 100.0) as u32
    } else {
        0
    };
    let total_cost: f64 = results.iter().map(|r| r.cost_usd).sum();
    let total_tokens: u32 = results.iter().map(|r| r.tokens.total_tokens).sum();
    let avg_latency: u64 = if total > 0 {
        results.iter().map(|r| r.latency_ms).sum::<u64>() / total as u64
    } else {
        0
    };

    let mut rows = String::new();
    for r in results {
        let status_class = if r.passed { "pass" } else { "fail" };
        let status_text = if r.passed { "PASS" } else { "FAIL" };

        let mut assertion_html = String::new();
        for a in &r.assertions {
            let icon = if a.passed { "✓" } else { "✗" };
            let cls = if a.passed { "pass" } else { "fail" };
            assertion_html.push_str(&format!(
                "<div class=\"assertion {}\"><span class=\"icon\">{}</span> <strong>{}</strong> — {}</div>",
                cls, icon, html_escape(&a.label), html_escape(&a.detail)
            ));
        }

        if let Some(ref err) = r.error {
            assertion_html.push_str(&format!(
                "<div class=\"assertion fail\"><span class=\"icon\">✗</span> <strong>error</strong> — {}</div>",
                html_escape(err)
            ));
        }

        let cost_str = if r.cost_usd > 0.0 {
            format!("${:.6}", r.cost_usd)
        } else {
            "—".to_string()
        };

        rows.push_str(&format!(
            r#"<tr class="{}">
  <td><span class="badge {}">{}</span></td>
  <td class="test-id">{}</td>
  <td class="input">{}</td>
  <td class="num">{}</td>
  <td class="num">{}</td>
  <td class="num">{}</td>
  <td class="assertions">{}</td>
</tr>"#,
            status_class,
            status_class,
            status_text,
            html_escape(&r.test_id),
            html_escape(&r.input_label),
            r.latency_ms,
            r.tokens.total_tokens,
            cost_str,
            assertion_html,
        ));
    }

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Prompt Sentinel — Test Report</title>
<style>
  :root {{
    --bg: #0f0f13;
    --surface: #1a1a24;
    --surface2: #22222e;
    --border: #2d2d3d;
    --text: #e4e4ef;
    --text-dim: #8888a0;
    --pass: #22c55e;
    --pass-bg: rgba(34,197,94,0.08);
    --fail: #ef4444;
    --fail-bg: rgba(239,68,68,0.08);
    --accent: #6366f1;
    --accent2: #a78bfa;
    --yellow: #eab308;
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{
    font-family: 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    background: var(--bg);
    color: var(--text);
    line-height: 1.6;
    padding: 2rem;
  }}
  .container {{ max-width: 1100px; margin: 0 auto; }}
  header {{
    display: flex; align-items: center; gap: 1rem;
    margin-bottom: 2rem; padding-bottom: 1rem;
    border-bottom: 1px solid var(--border);
  }}
  header h1 {{ font-size: 1.4rem; font-weight: 700; }}
  header .logo {{ font-size: 1.6rem; }}
  header .subtitle {{ color: var(--text-dim); font-size: 0.85rem; margin-left: auto; }}
  .stats {{
    display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 1rem; margin-bottom: 2rem;
  }}
  .stat {{
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.2rem;
  }}
  .stat .value {{ font-size: 1.8rem; font-weight: 700; }}
  .stat .label {{ color: var(--text-dim); font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.05em; margin-top: 0.2rem; }}
  .stat.pass .value {{ color: var(--pass); }}
  .stat.fail .value {{ color: var(--fail); }}
  .stat.accent .value {{ color: var(--accent2); }}
  .stat.yellow .value {{ color: var(--yellow); }}
  .bar-track {{
    height: 6px; background: var(--fail);
    border-radius: 3px; overflow: hidden;
    margin-bottom: 2rem;
  }}
  .bar-fill {{
    height: 100%; background: var(--pass);
    border-radius: 3px;
    transition: width 0.5s ease;
  }}
  table {{
    width: 100%;
    border-collapse: collapse;
    font-size: 0.88rem;
  }}
  thead th {{
    text-align: left;
    padding: 0.8rem 0.6rem;
    border-bottom: 2px solid var(--border);
    color: var(--text-dim);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }}
  tbody tr {{
    border-bottom: 1px solid var(--border);
  }}
  tbody tr:hover {{ background: var(--surface); }}
  tbody td {{
    padding: 0.8rem 0.6rem;
    vertical-align: top;
  }}
  .badge {{
    display: inline-block;
    padding: 0.15rem 0.55rem;
    border-radius: 4px;
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.04em;
  }}
  .badge.pass {{ background: var(--pass-bg); color: var(--pass); }}
  .badge.fail {{ background: var(--fail-bg); color: var(--fail); }}
  .test-id {{ font-weight: 600; }}
  .input {{ color: var(--text-dim); font-size: 0.82rem; }}
  .num {{ text-align: right; font-variant-numeric: tabular-nums; }}
  .assertions {{ font-size: 0.82rem; }}
  .assertion {{ margin: 0.15rem 0; }}
  .assertion.pass .icon {{ color: var(--pass); }}
  .assertion.fail .icon {{ color: var(--fail); }}
  footer {{
    margin-top: 2rem; padding-top: 1rem;
    border-top: 1px solid var(--border);
    color: var(--text-dim); font-size: 0.75rem;
    text-align: center;
  }}
</style>
</head>
<body>
<div class="container">
  <header>
    <span class="logo">⚡</span>
    <h1>Prompt Sentinel — Test Report</h1>
    <span class="subtitle">Generated {timestamp}</span>
  </header>

  <div class="stats">
    <div class="stat pass"><div class="value">{passed}</div><div class="label">Passed</div></div>
    <div class="stat fail"><div class="value">{failed}</div><div class="label">Failed</div></div>
    <div class="stat accent"><div class="value">{avg_latency}ms</div><div class="label">Avg Latency</div></div>
    <div class="stat yellow"><div class="value">{total_tokens}</div><div class="label">Total Tokens</div></div>
    <div class="stat accent"><div class="value">${total_cost:.6}</div><div class="label">Total Cost</div></div>
  </div>

  <div class="bar-track"><div class="bar-fill" style="width:{pass_pct}%"></div></div>

  <table>
    <thead>
      <tr>
        <th>Status</th>
        <th>Test ID</th>
        <th>Input</th>
        <th>Latency</th>
        <th>Tokens</th>
        <th>Cost</th>
        <th>Assertions</th>
      </tr>
    </thead>
    <tbody>
      {rows}
    </tbody>
  </table>

  <footer>
    Prompt Sentinel v0.1.0 · {total} test(s) · {pass_pct}% pass rate
  </footer>
</div>
</body>
</html>"##,
        timestamp = chrono_now(),
        passed = passed,
        failed = failed,
        avg_latency = avg_latency,
        total_tokens = total_tokens,
        total_cost = total_cost,
        pass_pct = pass_pct,
        rows = rows,
        total = total,
    );

    std::fs::write(output_path, &html)?;

    Ok(output_path.display().to_string())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn chrono_now() -> String {
    // Simple ISO-ish timestamp without chrono dependency
    let now = std::time::SystemTime::now();
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Format as "YYYY-MM-DD HH:MM UTC" approximation
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;

    // Approximate year/month/day from epoch days
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let months: [i64; 12] = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 1;
    for days_in_month in months {
        if remaining < days_in_month {
            break;
        }
        remaining -= days_in_month;
        m += 1;
    }
    let d = remaining + 1;

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        y, m, d, hours, minutes
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

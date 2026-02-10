use serde::Deserialize;
use std::collections::HashMap;

/// Top-level configuration parsed from the YAML test file.
#[derive(Debug, Deserialize)]
pub struct Config {
    #[allow(dead_code)]
    pub version: String,
    #[serde(default)]
    pub defaults: Defaults,
    pub tests: Vec<TestDef>,
}

/// Default settings applied to all tests unless overridden.
#[derive(Debug, Deserialize)]
pub struct Defaults {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_model(),
            temperature: default_temperature(),
        }
    }
}

fn default_provider() -> String {
    "openai".to_string()
}
fn default_model() -> String {
    "gpt-4o-mini".to_string()
}
fn default_temperature() -> f64 {
    0.7
}

/// A single test definition containing an ID, prompt template, and test cases.
#[derive(Debug, Deserialize)]
pub struct TestDef {
    pub id: String,
    pub prompt: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Inline test cases
    #[serde(default)]
    pub cases: Vec<TestCase>,
    /// Load test cases from a CSV file (optional)
    pub cases_file: Option<String>,
    /// Default assertions to apply to all CSV rows
    #[serde(default)]
    pub assertions: Vec<Assertion>,
}

/// A single test case with input variables and assertions to check.
#[derive(Debug, Clone, Deserialize)]
pub struct TestCase {
    pub input: HashMap<String, String>,
    #[serde(rename = "assert")]
    pub assertions: Vec<Assertion>,
}

/// An assertion to evaluate against the LLM response.
#[derive(Debug, Clone, Deserialize)]
pub struct Assertion {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: serde_yaml::Value,
}

/// All recognized assertion type strings.
pub const KNOWN_ASSERTION_TYPES: &[&str] = &[
    "contains",
    "not-contains",
    "latency_max",
    "snapshot",
    "regex",
    "json_valid",
    "min_length",
    "max_length",
];

/// Known providers.
pub const KNOWN_PROVIDERS: &[&str] = &["openai", "anthropic", "webhook"];

/// Parsed assertion with strong types.
#[derive(Debug)]
pub enum AssertionKind {
    Contains(String),
    NotContains(String),
    LatencyMax(u64),
    Snapshot,
    Regex(String),
    JsonValid,
    MinLength(u64),
    MaxLength(u64),
}

impl AssertionKind {
    pub fn from_raw(kind: &str, value: &serde_yaml::Value) -> anyhow::Result<Self> {
        match kind {
            "contains" => {
                let s = value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("contains value must be a string"))?;
                Ok(AssertionKind::Contains(s.to_string()))
            }
            "not-contains" => {
                let s = value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("not-contains value must be a string"))?;
                Ok(AssertionKind::NotContains(s.to_string()))
            }
            "latency_max" => {
                let ms = value
                    .as_u64()
                    .or_else(|| value.as_f64().map(|f| f as u64))
                    .ok_or_else(|| anyhow::anyhow!("latency_max value must be a number"))?;
                Ok(AssertionKind::LatencyMax(ms))
            }
            "snapshot" => Ok(AssertionKind::Snapshot),
            "regex" => {
                let pattern = value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("regex value must be a string pattern"))?;
                // Validate the regex at parse time
                regex::Regex::new(pattern)
                    .map_err(|e| anyhow::anyhow!("invalid regex '{}': {}", pattern, e))?;
                Ok(AssertionKind::Regex(pattern.to_string()))
            }
            "json_valid" => Ok(AssertionKind::JsonValid),
            "min_length" => {
                let n = value
                    .as_u64()
                    .or_else(|| value.as_f64().map(|f| f as u64))
                    .ok_or_else(|| anyhow::anyhow!("min_length value must be a number"))?;
                Ok(AssertionKind::MinLength(n))
            }
            "max_length" => {
                let n = value
                    .as_u64()
                    .or_else(|| value.as_f64().map(|f| f as u64))
                    .ok_or_else(|| anyhow::anyhow!("max_length value must be a number"))?;
                Ok(AssertionKind::MaxLength(n))
            }
            other => Err(anyhow::anyhow!("unknown assertion type: {}", other)),
        }
    }
}

/// Render a prompt template by substituting `{{key}}` placeholders with values.
pub fn render_prompt(template: &str, vars: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{{{}}}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

// Helper to render assertions (e.g., contains: "{{expected}}")
fn render_assertions(assertions: &[Assertion], vars: &HashMap<String, String>) -> Vec<Assertion> {
    assertions
        .iter()
        .map(|a| {
            // Only string values in assertions can be templated
            let new_value = if let Some(s) = a.value.as_str() {
                serde_yaml::Value::String(render_prompt(s, vars))
            } else {
                a.value.clone()
            };
            Assertion {
                kind: a.kind.clone(),
                value: new_value,
            }
        })
        .collect()
}

/// Load and parse a Config from a YAML file path.
/// Also loads any referenced CSV files.
pub fn load_config(path: &str) -> anyhow::Result<Config> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read config file '{}': {}", path, e))?;
    let mut config: Config = serde_yaml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse config file '{}': {}", path, e))?;

    // Resolve CSV files
    let base_dir = std::path::Path::new(path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));

    for test in &mut config.tests {
        if let Some(csv_file) = &test.cases_file {
            let csv_path = base_dir.join(csv_file);
            let mut rdr = csv::Reader::from_path(&csv_path).map_err(|e| {
                anyhow::anyhow!("Failed to open CSV '{}': {}", csv_path.display(), e)
            })?;

            let headers = rdr.headers()?.clone();

            for result in rdr.records() {
                let record = result.map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to parse CSV record in '{}': {}",
                        csv_path.display(),
                        e
                    )
                })?;

                let mut input = HashMap::new();
                for (i, field) in record.iter().enumerate() {
                    if let Some(header) = headers.get(i) {
                        input.insert(header.to_string(), field.to_string());
                    }
                }

                // Apply test-level assertions (rendering templates if needed)
                let assertions = render_assertions(&test.assertions, &input);

                test.cases.push(TestCase { input, assertions });
            }
        }
    }

    Ok(config)
}

/// Validate a config for logical errors. Returns a list of warnings/errors.
pub fn validate_config(config: &Config) -> Vec<String> {
    let mut issues = Vec::new();

    if !KNOWN_PROVIDERS.contains(&config.defaults.provider.as_str()) {
        issues.push(format!(
            "Unknown default provider '{}'. Known: {}",
            config.defaults.provider,
            KNOWN_PROVIDERS.join(", ")
        ));
    }

    if config.defaults.temperature < 0.0 || config.defaults.temperature > 2.0 {
        issues.push(format!(
            "Temperature {} is out of range [0.0, 2.0]",
            config.defaults.temperature
        ));
    }

    if config.tests.is_empty() {
        issues.push("No tests defined".to_string());
    }

    let mut seen_ids = std::collections::HashSet::new();
    for test in &config.tests {
        if !seen_ids.insert(&test.id) {
            issues.push(format!("Duplicate test ID '{}'", test.id));
        }

        if test.prompt.is_empty() {
            issues.push(format!("Test '{}': prompt is empty", test.id));
        }

        if test.cases.is_empty() && test.cases_file.is_none() {
            issues.push(format!(
                "Test '{}': no test cases defined (inline or CSV)",
                test.id
            ));
        }

        // Validate assertions logic
        // We only validate inline cases here fully. CSV cases are loaded dynamically.
        // But we should validate the "template" assertions if present.
        for (i, assertion) in test.assertions.iter().enumerate() {
            if !KNOWN_ASSERTION_TYPES.contains(&assertion.kind.as_str()) {
                // Fuzzy match logic repeated...
                let suggestion = find_closest(&assertion.kind, KNOWN_ASSERTION_TYPES);
                let hint = suggestion
                    .map(|s| format!(". Did you mean '{}'?", s))
                    .unwrap_or_default();
                issues.push(format!(
                    "Test '{}', default assertion {}: unknown type '{}'{}",
                    test.id,
                    i + 1,
                    assertion.kind,
                    hint
                ));
            }
        }

        for (ci, case) in test.cases.iter().enumerate() {
            if case.assertions.is_empty() {
                issues.push(format!(
                    "Test '{}', case {}: no assertions defined",
                    test.id,
                    ci + 1
                ));
            }

            for assertion in &case.assertions {
                if !KNOWN_ASSERTION_TYPES.contains(&assertion.kind.as_str()) {
                    let suggestion = find_closest(&assertion.kind, KNOWN_ASSERTION_TYPES);
                    let hint = suggestion
                        .map(|s| format!(". Did you mean '{}'?", s))
                        .unwrap_or_default();
                    issues.push(format!(
                        "Test '{}', case {}: unknown assertion type '{}'{}",
                        test.id,
                        ci + 1,
                        assertion.kind,
                        hint
                    ));
                } else if let Err(e) = AssertionKind::from_raw(&assertion.kind, &assertion.value) {
                    // Only validate concrete values, skip template strings
                    let is_template = assertion.value.as_str().map_or(false, |s| s.contains("{{"));
                    if !is_template {
                        issues.push(format!("Test '{}', case {}: {}", test.id, ci + 1, e));
                    }
                }
            }

            let rendered = render_prompt(&test.prompt, &case.input);
            if rendered.contains("{{") && rendered.contains("}}") {
                issues.push(format!(
                    "Test '{}', case {}: unresolved template variables in prompt",
                    test.id,
                    ci + 1
                ));
            }
        }
    }

    issues
}

fn find_closest<'a>(input: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .filter(|c| levenshtein(input, c) <= 3)
        .min_by_key(|c| levenshtein(input, c))
        .copied()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut matrix = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in 0..=a.len() {
        matrix[i][0] = i;
    }
    for j in 0..=b.len() {
        matrix[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
        }
    }
    matrix[a.len()][b.len()]
}

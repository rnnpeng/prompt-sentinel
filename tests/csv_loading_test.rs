use prompt_sentinel::config::{load_config, validate_config};
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_csv_loading() {
    // 1. Create a dummy CSV file
    let mut csv_file = NamedTempFile::new().unwrap();
    writeln!(csv_file, "name,expected").unwrap();
    writeln!(csv_file, "Alice,Hello Alice").unwrap();
    writeln!(csv_file, "Bob,Hello Bob").unwrap();

    let csv_path = csv_file.path().to_str().unwrap();

    // 2. Create a config that references it
    // Note: We need the CSV path relative to the config file location for load_config logic
    // But since we are using tempfiles, we can just use absolute paths if the logic supports it.
    // config.rs says: base_dir.join(csv_file).
    // If csv_file is absolute, join uses it as is. Perfect.

    let yaml = format!(
        r#"
version: "1.0"
defaults:
  provider: "openai"
  model: "gpt-4o-mini"
tests:
  - id: "csv-test"
    prompt: "Say hello to {{name}}"
    cases_file: "{}"
    assertions:
      - type: "contains"
        value: "{{{{expected}}}}"
"#,
        csv_path
    );

    let mut config_file = NamedTempFile::new().unwrap();
    write!(config_file, "{}", yaml).unwrap();

    // 3. Load config
    let cfg = load_config(config_file.path().to_str().unwrap()).unwrap();

    // 4. Verify
    let test = &cfg.tests[0];
    assert_eq!(test.id, "csv-test");
    assert_eq!(test.cases.len(), 2);

    // Row 1: Alice
    let case1 = &test.cases[0];
    assert_eq!(case1.input.get("name").map(|s| s.as_str()), Some("Alice"));
    // Templated assertion should be rendered
    // Wait, render_assertions renders AT LOAD TIME based on input vars.
    // So "value" should be "Hello Alice"
    if let prompt_sentinel::config::AssertionKind::Contains(val) =
        prompt_sentinel::config::AssertionKind::from_raw(
            &case1.assertions[0].kind,
            &case1.assertions[0].value,
        )
        .unwrap()
    {
        assert_eq!(val, "Hello Alice");
    } else {
        panic!("Wrong assertion kind");
    }

    // Row 2: Bob
    let case2 = &test.cases[1];
    assert_eq!(case2.input.get("name").map(|s| s.as_str()), Some("Bob"));
    if let prompt_sentinel::config::AssertionKind::Contains(val) =
        prompt_sentinel::config::AssertionKind::from_raw(
            &case2.assertions[0].kind,
            &case2.assertions[0].value,
        )
        .unwrap()
    {
        assert_eq!(val, "Hello Bob");
    } else {
        panic!("Wrong assertion kind");
    }
}

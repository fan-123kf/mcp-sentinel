// Integration tests for mcp-sentinel
// These tests validate core functionality without requiring external backends

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[tokio::test]
async fn test_config_loads_from_example() {
    let example_path = std::path::Path::new(MANIFEST_DIR).join("sentinel.toml.example");
    assert!(example_path.exists(), "sentinel.toml.example should exist");

    let content = std::fs::read_to_string(&example_path).unwrap();
    let content = content.replace("${GITHUB_TOKEN}", "test_token");
    let result: Result<toml::Value, _> = toml::from_str(&content);
    assert!(
        result.is_ok(),
        "sentinel.toml.example should be valid TOML: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_minimal_config_loads() {
    let example_path = std::path::Path::new(MANIFEST_DIR).join("examples/minimal.toml");
    assert!(example_path.exists(), "examples/minimal.toml should exist");

    let content = std::fs::read_to_string(&example_path).unwrap();
    let result: Result<toml::Value, _> = toml::from_str(&content);
    assert!(
        result.is_ok(),
        "examples/minimal.toml should be valid TOML: {:?}",
        result.err()
    );
}

#[test]
fn test_temp_dir_is_writable() {
    // Verify the system temp directory is available (cross-platform)
    let tmp = std::env::temp_dir();
    assert!(tmp.exists(), "System temp directory should exist: {:?}", tmp);

    // Write a test file to confirm writability
    let test_file = tmp.join("mcp_sentinel_write_test.txt");
    std::fs::write(&test_file, b"ok").expect("Should be able to write to temp dir");
    std::fs::remove_file(&test_file).ok();
}

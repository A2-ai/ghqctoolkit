//! Tests for API routes.

pub mod helpers;

#[cfg(test)]
mod harness;

#[cfg(test)]
mod routes;

#[cfg(test)]
mod test_runner {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::harness::{TestCase, TestRunner};

    /// Discover all YAML test cases recursively
    fn discover_test_cases() -> Vec<PathBuf> {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let cases_dir = Path::new(manifest_dir).join("src/api/tests/cases");

        if !cases_dir.exists() {
            return Vec::new();
        }

        let mut test_files = Vec::new();
        collect_yaml_files(&cases_dir, &mut test_files);

        // Sort for deterministic order
        test_files.sort();
        test_files
    }

    /// Recursively collect all .yaml files
    fn collect_yaml_files(dir: &Path, files: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect_yaml_files(&path, files);
                } else if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
                    files.push(path);
                }
            }
        }
    }

    #[tokio::test]
    async fn run_all_test_cases() {
        let test_files = discover_test_cases();
        assert!(
            !test_files.is_empty(),
            "No test cases found in src/api/tests/cases/"
        );

        // Use CARGO_MANIFEST_DIR for fixture path
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = Path::new(manifest_dir).join("src/api/tests/fixtures");
        let mut runner = TestRunner::new(fixture_path);
        let mut all_passed = true;
        let mut results = Vec::new();

        for test_file in test_files {
            println!("\nRunning test: {}", test_file.display());

            // Parse YAML
            let yaml_content = fs::read_to_string(&test_file)
                .unwrap_or_else(|e| panic!("Failed to read test file {}: {}", test_file.display(), e));
            let test_case: TestCase = serde_yaml::from_str(&yaml_content)
                .unwrap_or_else(|e| panic!("Failed to parse test file {}: {}", test_file.display(), e));

            // Run test
            match runner.run_test(test_case).await {
                Ok(result) => {
                    if result.passed {
                        println!("  ✓ PASSED (status: {})", result.status_code);
                    } else {
                        println!("  ✗ FAILED");
                        if let Err(ref e) = result.validation {
                            println!("    {}", e);
                        }
                        all_passed = false;
                    }
                    results.push(result);
                }
                Err(e) => {
                    println!("  ✗ ERROR: {}", e);
                    all_passed = false;
                }
            }
        }

        // Print summary
        println!("\n========================================");
        println!("Test Summary");
        println!("========================================");
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = results.len() - passed;
        println!(
            "Total: {}  Passed: {}  Failed: {}",
            results.len(),
            passed,
            failed
        );

        if !all_passed {
            println!("\nFailed tests:");
            for result in results.iter().filter(|r| !r.passed) {
                println!("  - {}", result.name);
            }
        }

        assert!(all_passed, "Some tests failed");
    }
}

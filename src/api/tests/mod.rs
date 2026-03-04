//! Tests for API routes.

pub mod helpers;

#[cfg(test)]
mod harness;

#[cfg(test)]
mod test_runner {
    use std::fs;
    use std::path::{Path, PathBuf};

    use axum::body::Body;
    use axum::extract::Request;
    use http::StatusCode;
    use tower::ServiceExt;

    use crate::Configuration;
    use crate::api::cache::CacheKey;
    use crate::api::tests::harness::runner::TestRunner;
    use crate::api::tests::harness::types::TestCase;
    use crate::api::tests::helpers::{MockGitInfo, load_test_issue, load_test_milestone};
    use crate::api::{AppState, create_router};

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
            let yaml_content = fs::read_to_string(&test_file).unwrap_or_else(|e| {
                panic!("Failed to read test file {}: {}", test_file.display(), e)
            });
            let test_case: TestCase = serde_yaml::from_str(&yaml_content).unwrap_or_else(|e| {
                panic!("Failed to parse test file {}: {}", test_file.display(), e)
            });

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

    #[tokio::test]
    async fn test_batch_get_issue_status_cache_behavior() {
        // Setup: Create mock with test issues
        let test_issue_1 = load_test_issue("test_file_issue");
        let test_issue_2 = load_test_issue("config_file_issue");
        let milestone = load_test_milestone("v1.0");

        let mock = MockGitInfo::builder()
            .with_issue(1, test_issue_1.clone())
            .with_issue(2, test_issue_2.clone())
            .with_milestone(milestone)
            .with_commit("abc123")
            .with_branch("main")
            .build();

        let config = Configuration::default();
        let state = AppState::new(mock, config, None, None);
        let app = create_router(state.clone());

        // FIRST REQUEST: Cache should be empty, will populate
        let response1 = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/issues/status?issues=1,2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status1 = response1.status();
        if status1 != StatusCode::OK {
            let body = axum::body::to_bytes(response1.into_body(), usize::MAX)
                .await
                .unwrap();
            panic!(
                "Expected 200 OK, got {}. Body: {}",
                status1,
                String::from_utf8_lossy(&body)
            );
        }
        assert_eq!(status1, StatusCode::OK);

        // Consume first response body immediately
        let body1 = axum::body::to_bytes(response1.into_body(), usize::MAX)
            .await
            .unwrap();

        // Verify cache was populated (scoped to release lock automatically)
        let key1 = CacheKey {
            issue_updated_at: test_issue_1.updated_at,
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let key2 = CacheKey {
            issue_updated_at: test_issue_2.updated_at,
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        {
            let cache = state.status_cache.read().await;
            assert!(
                cache.get(1, &key1).is_some(),
                "Issue 1 should be cached after first request"
            );
            assert!(
                cache.get(2, &key2).is_some(),
                "Issue 2 should be cached after first request"
            );
        } // cache read lock released here

        // SECOND REQUEST: Should hit cache (no new fetches needed)
        let response2 = app
            .oneshot(
                Request::builder()
                    .uri("/api/issues/status?issues=1,2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response2.status(), StatusCode::OK);

        // Consume second response body
        let body2 = axum::body::to_bytes(response2.into_body(), usize::MAX)
            .await
            .unwrap();

        // Parse as generic JSON to verify structure
        let json1: serde_json::Value = serde_json::from_slice(&body1).unwrap();
        let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();

        assert!(
            json1.is_object(),
            "First request should return an envelope object"
        );
        assert!(
            json2.is_object(),
            "Second request should return an envelope object"
        );
        assert_eq!(
            json1["results"].as_array().unwrap().len(),
            2,
            "First request should return 2 issues in results"
        );
        assert_eq!(
            json2["results"].as_array().unwrap().len(),
            2,
            "Second request should return 2 issues in results"
        );
        assert_eq!(
            json1["errors"].as_array().unwrap().len(),
            0,
            "First request should have no errors"
        );
        assert_eq!(
            json2["errors"].as_array().unwrap().len(),
            0,
            "Second request should have no errors"
        );

        // Responses should be identical (proving cache hit)
        assert_eq!(json1, json2, "Responses should be identical (cache hit)");

        // Cache should still have valid entries (scoped to release lock automatically)
        {
            let cache = state.status_cache.read().await;
            assert!(
                cache.get(1, &key1).is_some(),
                "Issue 1 should still be cached after second request"
            );
            assert!(
                cache.get(2, &key2).is_some(),
                "Issue 2 should still be cached after second request"
            );
        } // cache read lock released here
    }
}

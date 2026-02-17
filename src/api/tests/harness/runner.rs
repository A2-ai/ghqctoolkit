use anyhow::{Context, Result};
use axum::body::Body;
use axum::http::{Method, Request};
use serde_json::Value;
use std::path::PathBuf;
use tower::ServiceExt;

use crate::Configuration;
use crate::api::tests::helpers::{MockGitInfo, WriteCall};
use crate::api::{server::create_router, state::AppState};

use super::{
    assertions::{ResponseAsserter, ValidationError},
    loader::FixtureLoader,
    mock_builder::MockBuilder,
    types::{ExpectedWriteCall, HttpMethod, HttpRequest, TestCase},
};

/// Test runner that executes test cases
pub struct TestRunner {
    loader: FixtureLoader,
}

/// Result of running a single test
#[derive(Debug)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub status_code: u16,
    pub validation: Result<(), ValidationError>,
}

impl TestRunner {
    /// Create a new test runner with the given fixture base path
    pub fn new(fixture_path: PathBuf) -> Self {
        Self {
            loader: FixtureLoader::new(fixture_path),
        }
    }

    /// Run a single test case
    pub async fn run_test(&mut self, test_case: TestCase) -> Result<TestResult> {
        // Load fixtures
        let fixtures = self
            .loader
            .load_fixtures(&test_case.fixtures)
            .context("Failed to load fixtures")?;

        // Build MockGitInfo
        let mock = MockBuilder::build(&test_case.git_state, &fixtures);

        // Create AppState and router (clone mock to keep a reference for assertions)
        let config = Configuration::default();
        let state = AppState::new(mock.clone(), config, None);
        let app = create_router(state);

        // Build HTTP request
        let request = self
            .build_request(&test_case.request)
            .context("Failed to build request")?;

        // Execute request
        let response = app
            .oneshot(request)
            .await
            .context("Failed to execute request")?;

        // Extract status and body
        let status = response.status();
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .context("Failed to read response body")?;

        // Parse body as JSON (if not empty and needed for validation)
        let body_json = if body_bytes.is_empty() {
            None
        } else if test_case.response.body.is_some() {
            // Only parse JSON if body validation is expected
            Some(
                serde_json::from_slice::<Value>(&body_bytes)
                    .context("Failed to parse response body as JSON")?,
            )
        } else {
            // Body validation not specified, skip JSON parsing
            None
        };

        // Validate response
        let asserter = ResponseAsserter::new(&test_case.response);
        let validation = asserter.validate(status, body_json.as_ref());

        // Validate write calls if specified
        let write_call_validation = if !test_case.assert_write_calls.is_empty() {
            validate_write_calls(&mock, &test_case.assert_write_calls)
        } else {
            Ok(())
        };

        // Combine validations
        let combined_validation = match (validation, write_call_validation) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(e), Ok(())) => Err(e),
            (Ok(()), Err(e)) => Err(e),
            (Err(mut e1), Err(e2)) => {
                e1.details.push(format!("Write call validation: {}", e2));
                Err(e1)
            }
        };

        Ok(TestResult {
            name: test_case.name,
            passed: combined_validation.is_ok(),
            status_code: status.as_u16(),
            validation: combined_validation,
        })
    }

    /// Build an HTTP request from test specification
    fn build_request(&self, http_request: &HttpRequest) -> Result<Request<Body>> {
        // Build URI with query parameters
        let uri = if http_request.query.is_empty() {
            http_request.path.clone()
        } else {
            // Sort keys for deterministic order and URL-encode values
            let mut keys: Vec<_> = http_request.query.keys().collect();
            keys.sort();
            let query_string: Vec<String> = keys
                .iter()
                .map(|k| {
                    let v = &http_request.query[*k];
                    format!("{}={}", urlencoding::encode(k), urlencoding::encode(v))
                })
                .collect();
            format!("{}?{}", http_request.path, query_string.join("&"))
        };

        // Convert HttpMethod to axum Method
        let method = match http_request.method {
            HttpMethod::Get => Method::GET,
            HttpMethod::Post => Method::POST,
            HttpMethod::Put => Method::PUT,
            HttpMethod::Delete => Method::DELETE,
            HttpMethod::Patch => Method::PATCH,
        };

        // Build request
        let mut builder = Request::builder().uri(&uri).method(method);

        // Add body if present
        let body = if let Some(body_value) = &http_request.body {
            // Set content-type header
            builder = builder.header("content-type", "application/json");
            // Serialize to JSON bytes
            let body_bytes =
                serde_json::to_vec(body_value).context("Failed to serialize request body")?;
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        builder.body(body).context("Failed to build request")
    }
}

/// Validate that expected write calls were made to the mock
fn validate_write_calls(
    mock: &MockGitInfo,
    expected: &[ExpectedWriteCall],
) -> Result<(), ValidationError> {
    let actual_calls = mock.write_calls();
    let mut errors = Vec::new();

    // Convert expected calls to WriteCall for comparison
    for expected_call in expected {
        let write_call = match expected_call {
            ExpectedWriteCall::CreateMilestone { name, description } => {
                WriteCall::CreateMilestone {
                    name: name.clone(),
                    description: description.clone(),
                }
            }
            ExpectedWriteCall::PostComment { comment_type } => WriteCall::PostComment {
                comment_type: comment_type.clone(),
            },
            ExpectedWriteCall::CloseIssue { issue_number } => WriteCall::CloseIssue {
                issue_number: *issue_number,
            },
            ExpectedWriteCall::OpenIssue { issue_number } => WriteCall::OpenIssue {
                issue_number: *issue_number,
            },
        };

        if !actual_calls.contains(&write_call) {
            errors.push(format!("Expected write call not found: {:?}", write_call));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationError {
            message: "Write call assertions failed".to_string(),
            details: errors,
        })
    }
}

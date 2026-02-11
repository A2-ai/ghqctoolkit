use crate::Configuration;
use crate::api::tests::helpers::{MockGitInfo, load_test_issue};
use crate::api::{server::create_router, state::AppState, types::Issue};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt; // for oneshot()

// Helper to load configuration with defaults
fn load_test_config() -> Configuration {
    Configuration::default()
}

#[tokio::test]
async fn test_health_check() {
    let mock = MockGitInfo::builder().build();
    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_get_issue_success() {
    // Setup mock with test issue
    let test_issue = load_test_issue("test_file_issue");
    let mock = MockGitInfo::builder()
        .with_issue(1, test_issue.clone())
        .with_commit("abc123")
        .with_branch("main")
        .build();

    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    // Make request
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/issues/1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    log::debug!("{response:#?}");

    // Assert response
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    if status != StatusCode::OK {
        let body_str = String::from_utf8_lossy(&body);
        eprintln!("Response status: {}", status);
        eprintln!("Response body: {}", body_str);
    }
    assert_eq!(status, StatusCode::OK);

    let issue: Issue = serde_json::from_slice(&body).unwrap();
    assert_eq!(issue.number, 1);
}

#[tokio::test]
async fn test_get_issue_not_found() {
    let mock = MockGitInfo::builder().build();
    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/issues/999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_get_blocked_issues_success() {
    let main_issue = load_test_issue("test_file_issue");
    let blocking_issue = load_test_issue("config_file_issue");

    let mock = MockGitInfo::builder()
        .with_issue(1, main_issue)
        .with_blocked_issues(1, vec![blocking_issue])
        .build();

    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/issues/1/blocked")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    if status != StatusCode::OK {
        eprintln!("Response status: {}", status);
        eprintln!("Response body: {}", String::from_utf8_lossy(&body));
    }

    assert_eq!(status, StatusCode::OK);

    let issues: Vec<crate::api::types::BlockedIssueStatus> = serde_json::from_slice(&body).unwrap();
    assert_eq!(issues.len(), 1);
}

#[tokio::test]
async fn test_get_blocked_issues_empty() {
    let main_issue = load_test_issue("test_file_issue");

    let mock = MockGitInfo::builder()
        .with_issue(1, main_issue)
        .with_blocked_issues(1, vec![])
        .build();

    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/issues/1/blocked")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let issues: Vec<crate::api::types::BlockedIssueStatus> = serde_json::from_slice(&body).unwrap();
    assert_eq!(issues.len(), 0);
}

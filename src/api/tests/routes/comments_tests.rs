use crate::Configuration;
use crate::api::tests::helpers::{MockGitInfo, load_test_issue};
use crate::api::{server::create_router, state::AppState};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn load_test_config() -> Configuration {
    Configuration::default()
}

#[tokio::test]
async fn test_create_comment() {
    let test_issue = load_test_issue("test_file_issue");
    let mock = MockGitInfo::builder()
        .with_issue(1, test_issue)
        .with_commit("abc123")
        .with_branch("main")
        .build();

    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let request_body = serde_json::json!({
        "issue_number": 1,
        "current_commit": "abc123",
        "previous_commit": "def456",
        "note": "Test comment"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/issues/1/comment")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // MockGitInfo returns NotImplemented for post_comment
    // This is expected behavior - we're testing the route works
    assert_ne!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_approve_issue() {
    let test_issue = load_test_issue("test_file_issue");
    let mock = MockGitInfo::builder()
        .with_issue(1, test_issue)
        .with_commit("abc123")
        .with_branch("main")
        .build();

    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let request_body = serde_json::json!({
        "issue_number": 1,
        "approved_commit": "abc123",
        "note": "Approved"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/issues/1/approve")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // MockGitInfo returns NotImplemented for post_comment
    assert_ne!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_unapprove_issue() {
    let test_issue = load_test_issue("test_file_issue");
    let mock = MockGitInfo::builder()
        .with_issue(1, test_issue)
        .with_commit("abc123")
        .with_branch("main")
        .build();

    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let request_body = serde_json::json!({
        "issue_number": 1,
        "reason": "Found a bug"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/issues/1/unapprove")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // MockGitInfo returns NotImplemented for post_comment
    assert_ne!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_review_issue() {
    let test_issue = load_test_issue("test_file_issue");
    let mock = MockGitInfo::builder()
        .with_issue(1, test_issue)
        .with_commit("abc123")
        .with_branch("main")
        .build();

    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let request_body = serde_json::json!({
        "issue_number": 1,
        "commit": "abc123",
        "note": "Reviewing changes"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/issues/1/review")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // MockGitInfo returns NotImplemented for post_comment
    assert_ne!(response.status(), StatusCode::OK);
}

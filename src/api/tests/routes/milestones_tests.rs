use crate::Configuration;
use crate::api::tests::helpers::MockGitInfo;
use crate::api::{server::create_router, state::AppState};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn load_test_config() -> Configuration {
    Configuration::default()
}

#[tokio::test]
async fn test_list_milestones() {
    let mock = MockGitInfo::builder().build();
    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/milestones")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let milestones: Vec<crate::api::types::Milestone> = serde_json::from_slice(&body).unwrap();
    // MockGitInfo returns empty list by default
    assert!(milestones.is_empty());
}

#[tokio::test]
async fn test_list_milestone_issues() {
    let mock = MockGitInfo::builder().build();
    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/milestones/1/issues")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let issues: Vec<crate::api::types::Issue> = serde_json::from_slice(&body).unwrap();
    // MockGitInfo returns empty list by default
    assert!(issues.is_empty());
}

#[tokio::test]
async fn test_create_milestone() {
    let mock = MockGitInfo::builder().build();
    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let request_body = serde_json::json!({
        "title": "Test Milestone",
        "description": "A test milestone"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/milestones")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // MockGitInfo returns NotImplemented error by default
    // This is expected behavior for the mock
    assert_ne!(response.status(), StatusCode::OK);
}

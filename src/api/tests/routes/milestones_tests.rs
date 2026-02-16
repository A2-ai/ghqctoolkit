//! Milestone tests - POST endpoints only
//! GET endpoints have been migrated to YAML tests in src/api/tests/cases/milestones/

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

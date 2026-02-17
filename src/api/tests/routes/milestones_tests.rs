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
    use crate::api::tests::helpers::WriteCall;
    use crate::api::types::Milestone;

    let mock = MockGitInfo::builder().build();
    let config = load_test_config();
    let state = AppState::new(mock.clone(), config, None);
    let app = create_router(state);

    let request_body = serde_json::json!({
        "name": "Test Milestone",
        "description": "A test milestone"
    });

    let request = Request::builder()
        .method("POST")
        .uri("/api/milestones")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Verify success response
    assert_eq!(response.status(), StatusCode::CREATED);

    // Verify response body structure
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let milestone: Milestone = serde_json::from_slice(&body).unwrap();
    assert_eq!(milestone.title, "Test Milestone");
    assert_eq!(milestone.description, Some("A test milestone".to_string()));
    assert_eq!(milestone.number, 1);
    assert_eq!(milestone.state, "open");

    // Verify write call was tracked
    let expected_call = WriteCall::CreateMilestone {
        name: "Test Milestone".to_string(),
        description: Some("A test milestone".to_string()),
    };
    assert!(mock.was_called(&expected_call), "create_milestone should have been called");
}

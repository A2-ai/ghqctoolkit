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
async fn test_list_assignees() {
    let mock = MockGitInfo::builder().build();
    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/assignees")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let assignees: Vec<String> = serde_json::from_slice(&body).unwrap();
    // MockGitInfo returns empty list by default
    assert_eq!(assignees.len(), 0);
}

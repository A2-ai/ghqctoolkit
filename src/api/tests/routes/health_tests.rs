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

    // Verify body contains expected JSON structure
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert!(body_str.contains("\"status\""));
}

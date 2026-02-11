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
async fn test_list_checklists() {
    let mock = MockGitInfo::builder().build();
    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/configuration/checklists")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let checklists: Vec<crate::api::types::Checklist> = serde_json::from_slice(&body).unwrap();
    // Default config has one checklist
    assert_eq!(checklists.len(), 1);
    assert_eq!(&checklists[0].name, "Custom");
}

#[tokio::test]
async fn test_get_configuration_status() {
    let mock = MockGitInfo::builder().build();
    let config = load_test_config();
    let state = AppState::new(mock, config, None);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/configuration/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let status: crate::api::types::ConfigurationStatusResponse = serde_json::from_slice(&body).unwrap();
    // Verify basic structure - should have checklists
    assert_eq!(status.checklists.len(), 1);
}

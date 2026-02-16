//! Issue status endpoint tests
//! Tests cache behavior for batch_get_issue_status endpoint

use crate::Configuration;
use crate::api::cache::CacheKey;
use crate::api::tests::helpers::{MockGitInfo, load_test_issue, load_test_milestone};
use crate::api::{server::create_router, state::AppState};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

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
    let state = AppState::new(mock, config, None);
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

    assert!(json1.is_array(), "First request should return an array");
    assert!(json2.is_array(), "Second request should return an array");
    assert_eq!(
        json1.as_array().unwrap().len(),
        2,
        "First request should return 2 issues"
    );
    assert_eq!(
        json2.as_array().unwrap().len(),
        2,
        "Second request should return 2 issues"
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

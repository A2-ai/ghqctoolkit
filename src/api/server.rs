//! Axum server setup and router assembly.

use crate::api::routes::{comments, configuration, health, issues, milestones, status};
use crate::api::state::AppState;
use axum::{
    Router,
    routing::{get, post},
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

/// Create the API router with all routes.
pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health
        .route("/api/health", get(health::health_check))
        // Milestones
        .route("/api/milestones", get(milestones::list_milestones))
        .route("/api/milestones", post(milestones::create_milestone))
        .route(
            "/api/milestones/{number}/issues",
            get(milestones::list_milestone_issues),
        )
        // Issues
        .route("/api/issues", post(issues::create_issue))
        .route("/api/issues/status", get(issues::batch_get_issue_status))
        .route("/api/issues/{number}", get(issues::get_issue))
        .route(
            "/api/issues/{number}/blocked",
            get(issues::get_blocked_issues),
        )
        // Comments & Actions
        .route(
            "/api/issues/{number}/comment",
            post(comments::create_comment),
        )
        .route(
            "/api/issues/{number}/approve",
            post(comments::approve_issue),
        )
        .route(
            "/api/issues/{number}/unapprove",
            post(comments::unapprove_issue),
        )
        .route("/api/issues/{number}/review", post(comments::review_issue))
        // Supporting Data
        .route("/api/assignees", get(status::list_assignees))
        // Configuration
        .route(
            "/api/configuration/checklists",
            get(configuration::list_checklists),
        )
        .route(
            "/api/configuration/status",
            get(configuration::get_configuration_status),
        )
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

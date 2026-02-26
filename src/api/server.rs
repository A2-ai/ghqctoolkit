//! Axum server setup and router assembly.

use crate::GitProvider;
use crate::api::routes::{
    archive, comments, configuration, files, health, issues, milestones, preview, record, status,
};
use crate::api::state::AppState;
use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

async fn log_request(req: Request, next: Next) -> Response {
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_else(|| req.uri().path());
    log::info!("{} {}", req.method(), path);
    next.run(req).await
}

/// Create the API router with all routes.
pub fn create_router<G: GitProvider + 'static>(state: AppState<G>) -> Router {
    // NOTE: Wildcard CORS is intentional for local development serving a GUI.
    // This should NOT be used in production or networked deployments.
    // For production, restrict origins to specific domains.
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
            get(milestones::list_milestone_issues).post(issues::create_issues),
        )
        // Issues
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
        // Files
        .route("/api/files/tree", get(files::list_tree))
        .route("/api/files/content", get(preview::get_file_content))
        // Previews
        .route("/api/preview/issue", post(preview::preview_issue))
        .route(
            "/api/preview/{number}/comment",
            post(preview::preview_comment),
        )
        .route(
            "/api/preview/{number}/review",
            post(preview::preview_review),
        )
        .route(
            "/api/preview/{number}/approve",
            post(preview::preview_approve),
        )
        .route(
            "/api/preview/{number}/unapprove",
            post(preview::preview_unapprove),
        )
        // Supporting Data
        .route("/api/assignees", get(status::list_assignees))
        .route("/api/repo", get(status::repo_info))
        // Record PDF generation
        .route(
            "/api/record/upload",
            post(record::upload_context_file).layer(DefaultBodyLimit::max(50 * 1024 * 1024)),
        )
        .route("/api/record/preview", post(record::preview_record))
        .route("/api/record/preview.pdf", get(record::serve_preview_pdf))
        .route("/api/record/generate", post(record::generate_record))
        // Archive
        .route("/api/archive/generate", post(archive::generate_archive))
        // Configuration
        .route(
            "/api/configuration",
            get(configuration::get_configuration).post(configuration::setup_configuration_repo),
        )
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(log_request))
        .with_state(state)
}

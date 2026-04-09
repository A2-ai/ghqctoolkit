//! Axum server setup and router assembly.

use crate::GitProvider;
use crate::api::routes::{
    archive, comments, commits, configuration, files, health, issues, milestones, preview, record,
    status,
};
use crate::api::state::AppState;
use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio::net::TcpListener;
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
        .route(
            "/api/milestones/{number}/renames",
            get(milestones::list_milestone_renames),
        )
        // Issues
        .route("/api/issues/status", get(issues::batch_get_issue_status))
        .route("/api/issues/{number}", get(issues::get_issue))
        .route(
            "/api/issues/{number}/blocked",
            get(issues::get_blocked_issues),
        )
        .route(
            "/api/issues/{number}/rename",
            post(issues::rename_issue),
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
        .route(
            "/api/files/collaborators",
            get(files::get_file_collaborators),
        )
        .route("/api/files/content", get(files::get_file_content))
        .route("/api/files/raw", get(files::get_file_raw))
        // Previews
        .route("/api/preview/issue", post(preview::preview_issue))
        .route(
            "/api/preview/previous-qc-diff",
            post(preview::preview_previous_qc_diff),
        )
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
        .route("/api/commits", get(commits::get_commits))
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

/// Bind the local HTTP server.
///
/// When `ipv4_only` is false, prefer IPv6 and fall back to IPv4 if IPv6 bind fails.
pub async fn bind_local_server(port: u16, ipv4_only: bool) -> std::io::Result<TcpListener> {
    if ipv4_only {
        return bind_ipv4(port).await;
    }

    match bind_dual_stack_ipv6(port).await {
        Ok(listener) => Ok(listener),
        Err(v6_err) => {
            log::warn!(
                "Failed to bind dual-stack IPv6 listener on port {port}: {v6_err}. Falling back to IPv4"
            );
            bind_ipv4(port).await
        }
    }
}

pub async fn bind_local_server_with_url(
    port: u16,
    ipv4_only: bool,
) -> std::io::Result<(TcpListener, String)> {
    let listener = bind_local_server(port, ipv4_only).await?;
    let url = local_server_url(&listener);
    Ok((listener, url))
}

pub fn local_server_url(listener: &TcpListener) -> String {
    match listener.local_addr() {
        Ok(addr) if addr.is_ipv6() => format!("http://[::1]:{}", addr.port()),
        Ok(addr) => format!("http://127.0.0.1:{}", addr.port()),
        Err(_) => "http://127.0.0.1".to_string(),
    }
}

async fn bind_dual_stack_ipv6(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv6Addr::UNSPECIFIED, port))).await
}

async fn bind_ipv4(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port))).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bind_local_server_with_url_uses_ipv4_when_requested() {
        let (_listener, url) = bind_local_server_with_url(0, true).await.unwrap();

        assert!(url.starts_with("http://127.0.0.1:"));
    }

    #[tokio::test]
    async fn bind_local_server_with_url_returns_matching_loopback_host() {
        let (listener, url) = bind_local_server_with_url(0, false).await.unwrap();

        let addr = listener.local_addr().unwrap();
        if addr.is_ipv6() {
            assert!(url.starts_with("http://[::1]:"));
        } else {
            assert!(url.starts_with("http://127.0.0.1:"));
        }
    }
}

//! Axum server setup and router assembly.

use crate::api::routes::{
    archive, comments, commits, configuration, files, health, issues, milestones, preview, record,
    status,
};
use crate::api::state::AppState;
use crate::{GitCli, GitProvider};
use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
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
pub fn create_router<G: GitProvider + 'static, C: GitCli + Send + Sync + 'static>(
    state: AppState<G>,
) -> Router {
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
        .route("/api/issues/{number}/rename", post(issues::rename_issue))
        .route("/api/issues/{number}/rounds", post(issues::create_round))
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
        // D47: the round comment's preview renders through `QCRound`'s real
        // `CommentBody`, so it cannot drift from what `POST /rounds` posts.
        .route("/api/preview/round", post(preview::preview_round))
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
            get(configuration::get_configuration)
                .post(configuration::setup_configuration_repo::<G, C>),
        )
        .route(
            "/api/configuration/update",
            post(configuration::update_configuration_repo::<G, C>),
        )
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(log_request))
        .with_state(state)
}

/// Bind the local HTTP server to an explicit address.
///
/// The caller decides the address (the CLI defaults to IPv4 loopback). There is no
/// probing and no fallback: if the requested address cannot be bound, that is an error
/// the caller should see rather than a silent switch to a different interface.
pub async fn bind_local_server(addr: SocketAddr) -> std::io::Result<TcpListener> {
    TcpListener::bind(addr).await
}

/// Bind as [`bind_local_server`] and also return the URL to reach the listener.
pub async fn bind_local_server_with_url(
    addr: SocketAddr,
) -> std::io::Result<(TcpListener, String)> {
    let listener = bind_local_server(addr).await?;
    let url = local_server_url(&listener);
    Ok((listener, url))
}

/// The URL for the address the listener actually bound.
///
/// Wildcard addresses are not reachable as-is, so they are displayed as the matching
/// loopback address.
pub fn local_server_url(listener: &TcpListener) -> String {
    match listener.local_addr() {
        Ok(addr) => format!("http://{}:{}", display_host(addr.ip()), addr.port()),
        Err(_) => "http://127.0.0.1".to_string(),
    }
}

/// Render `ip` as a URL host component.
///
/// Wildcards map to loopback, and IPv6 literals are bracketed. Built from the `IpAddr`
/// rather than the `SocketAddr` so an IPv6 scope id (`%eth0`) never leaks into the URL.
fn display_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) if v4 == Ipv4Addr::UNSPECIFIED => Ipv4Addr::LOCALHOST.to_string(),
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) if v6 == Ipv6Addr::UNSPECIFIED => format!("[{}]", Ipv6Addr::LOCALHOST),
        IpAddr::V6(v6) => format!("[{v6}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bind_local_server_with_url_reports_ipv4_loopback() {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let (_listener, url) = bind_local_server_with_url(addr).await.unwrap();

        assert!(url.starts_with("http://127.0.0.1:"), "got {url}");
    }

    #[tokio::test]
    async fn bind_local_server_with_url_reports_ipv6_loopback() {
        let addr = SocketAddr::from((Ipv6Addr::LOCALHOST, 0));
        let Ok((_listener, url)) = bind_local_server_with_url(addr).await else {
            // Some sandboxes have no IPv6 stack at all; nothing to assert there.
            return;
        };

        assert!(url.starts_with("http://[::1]:"), "got {url}");
    }

    #[test]
    fn display_host_maps_wildcards_to_loopback() {
        assert_eq!(display_host(IpAddr::V4(Ipv4Addr::UNSPECIFIED)), "127.0.0.1");
        assert_eq!(display_host(IpAddr::V6(Ipv6Addr::UNSPECIFIED)), "[::1]");
    }

    #[test]
    fn display_host_brackets_ipv6_literals() {
        assert_eq!(display_host(IpAddr::V4(Ipv4Addr::LOCALHOST)), "127.0.0.1");
        assert_eq!(display_host(IpAddr::V6(Ipv6Addr::LOCALHOST)), "[::1]");
    }
}

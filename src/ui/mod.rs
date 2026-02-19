use crate::GitProvider;
use crate::api::AppState;
use axum::{
    body::Body,
    http::{StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "ui/dist/client/"]
struct UiAssets;

/// Serve an embedded static file, falling back to index.html for SPA routing.
async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match UiAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(content.data))
                .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "").into_response())
        }
        None => match UiAssets::get("index.html") {
            Some(index) => Html(index.data).into_response(),
            None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
        },
    }
}

/// Start the embedded server (API + SPA) and open the browser.
pub async fn run<G: GitProvider + 'static>(port: u16, state: AppState<G>) -> anyhow::Result<()> {
    let app = crate::api::create_router(state).fallback(static_handler);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let url = format!("http://localhost:{port}");
    log::info!("ghqc UI running at {url}");

    // Open the browser (non-blocking, ignore errors)
    let _ = open::that(&url);

    axum::serve(listener, app).await?;
    Ok(())
}

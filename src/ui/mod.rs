use crate::GitProvider;
use crate::api::AppState;
use axum::{
    body::Body,
    extract::Request,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "ui/dist/client/"]
struct UiAssets;

/// Serve an embedded static file, falling back to index.html for SPA routing.
async fn static_handler(req: Request) -> Response {
    let uri_path = req.uri().path();
    let path = uri_path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match UiAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let bytes = content.data;
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CONTENT_LENGTH, bytes.len())
                .body(Body::from(bytes))
                .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "").into_response())
        }
        None => serve_index(uri_path),
    }
}

fn serve_index(path: &str) -> Response {
    match UiAssets::get("index.html") {
        Some(index) => {
            let depth = path
                .split('/')
                .filter(|s| !s.is_empty())
                .count()
                .saturating_sub(1);

            let base_href = if depth == 0 {
                "./".to_string()
            } else {
                "../".repeat(depth)
            };

            let html = String::from_utf8_lossy(&index.data);
            // Rewrite absolute asset paths to relative so they resolve through proxy prefixes.
            // Covers both HTML attributes and inline JS manifest strings.
            // let html = html.replace("\"/assets/", "\"./assets/");
            // let html = html.replace("href=\"/logo.", "href=\"./logo.");
            let html = html.replacen("<head>", &format!("<head><base href=\"{base_href}\">"), 1);
            let bytes = html.into_bytes();

            Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CONTENT_LENGTH, bytes.len().to_string())
                .body(Body::from(bytes))
                .unwrap_or((StatusCode::INTERNAL_SERVER_ERROR, "").into_response())
        }
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}

/// Start the embedded server (API + SPA) and open the browser.
pub async fn run<G: GitProvider + 'static>(
    port: u16,
    state: AppState<G>,
    no_open: bool,
) -> anyhow::Result<()> {
    let app = crate::api::create_router(state).fallback(static_handler);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let url = format!("http://localhost:{port}");
    log::info!("ghqc UI running at {url}");

    // Open the browser (non-blocking, ignore errors)
    if !no_open {
        let _ = open::that(&url);
    }

    axum::serve(listener, app).await?;
    Ok(())
}

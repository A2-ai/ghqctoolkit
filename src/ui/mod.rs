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
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if path == "index.html" {
        return serve_index();
    }

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
        None => serve_index(),
    }
}

fn serve_index() -> Response {
    match UiAssets::get("index.html") {
        Some(index) => {
            // Asset paths are already relative (./assets/...) — produced at build time by
            // transformAssetUrls in src/server.ts. No runtime rewriting needed.
            Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CONTENT_LENGTH, index.data.len())
                .body(Body::from(index.data))
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

    let addr = format!("::{port}");
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

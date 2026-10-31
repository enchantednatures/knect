//! Embedded WebUI static fallback.
//!
//! Task 16 replaces the placeholder asset with the Svelte bundle and adds CSP
//! headers. API routes are registered before this fallback, so `/rpc`,
//! `/events`, and `/healthz` never reach it.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/webui"]
struct Assets;

const INDEX: &str = "index.html";

/// Serves embedded assets with SPA fallback for extensionless paths.
pub async fn fallback(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let path = if requested.is_empty() {
        INDEX
    } else {
        requested
    };
    if let Some(file) = Assets::get(path) {
        return asset_response(path, file);
    }
    if path.contains('.') {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match Assets::get(INDEX) {
        Some(file) => asset_response(INDEX, file),
        None => (StatusCode::NOT_FOUND, "WebUI not built").into_response(),
    }
}

fn asset_response(path: &str, file: rust_embed::EmbeddedFile) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut response = Response::new(Body::from(file.data.into_owned()));
    if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn body_text(response: Response) -> String {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[tokio::test]
    async fn root_serves_placeholder_page() {
        let response = fallback(Uri::from_static("/")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/html")
        );
        assert!(body_text(response).await.contains("WebUI not built"));
    }

    #[tokio::test]
    async fn spa_routes_fall_back_but_missing_assets_do_not() {
        assert_eq!(
            fallback(Uri::from_static("/topology/default"))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            fallback(Uri::from_static("/missing.js")).await.status(),
            StatusCode::NOT_FOUND
        );
    }
}

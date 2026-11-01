//! Embedded WebUI served by `knectd`.
//!
//! The Svelte bundle is committed under `assets/webui` and compiled into the
//! daemon by `rust-embed`, so a release binary needs no Node runtime, sidecar,
//! or external web server. API routes are registered before this fallback, so
//! `/rpc`, `/events`, and `/healthz` never reach it.
//!
//! Routing contract:
//! - `/` and extensionless SPA routes serve `index.html`.
//! - Files that exist under the embedded folder are served with their MIME type.
//! - A missing `/assets/*` path is a 404, never the SPA document, so a broken
//!   bundle fails loudly instead of rendering a white page.
//! - A missing index (assets not built) is a 503 with build instructions.

use axum::body::Body;
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::Response;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/webui"]
struct Assets;

const INDEX: &str = "index.html";

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
     script-src 'self'; \
     style-src 'self' 'unsafe-inline'; \
     img-src 'self' data:; \
     font-src 'self'; \
     connect-src 'self'; \
     object-src 'none'; \
     base-uri 'none'; \
     frame-ancestors 'none'; \
     form-action 'self'";

const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
const CACHE_INDEX: &str = "no-cache";

/// Serves an embedded asset, the SPA document, or a precise 404/503.
pub async fn fallback(method: Method, uri: Uri) -> Response {
    let head = method == Method::HEAD;
    let raw = uri.path().trim_start_matches('/');
    let path = if raw.is_empty() { INDEX } else { raw };

    if is_unsafe_path(path) {
        return text_response(StatusCode::BAD_REQUEST, "invalid asset path", head);
    }

    if let Some(file) = Assets::get(path) {
        return asset_response(path, file, head);
    }

    if path.starts_with("assets/")
        || path
            .rsplit('/')
            .next()
            .is_some_and(|part| part.contains('.'))
    {
        return text_response(StatusCode::NOT_FOUND, "not found", head);
    }

    match Assets::get(INDEX) {
        Some(file) => asset_response(INDEX, file, head),
        None => text_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "WebUI assets are not embedded. Build them with: \
             npm --prefix webui ci && npm --prefix webui run build && npm --prefix webui run embed",
            head,
        ),
    }
}

fn is_unsafe_path(path: &str) -> bool {
    path.split('/')
        .any(|part| part == ".." || part == "." || part.contains('\\') || part.contains('\0'))
}

fn asset_response(path: &str, file: rust_embed::EmbeddedFile, head: bool) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let body = if head {
        Body::empty()
    } else {
        Body::from(file.data.into_owned())
    };
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;

    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    let cache = if path == INDEX {
        CACHE_INDEX
    } else if path.starts_with("assets/") {
        CACHE_IMMUTABLE
    } else {
        CACHE_INDEX
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn text_response(status: StatusCode, message: &str, head: bool) -> Response {
    let body = if head {
        Body::empty()
    } else {
        Body::from(message.to_owned())
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(CACHE_INDEX));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn body_bytes(response: Response) -> Vec<u8> {
        response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes()
            .to_vec()
    }

    async fn body_text(response: Response) -> String {
        String::from_utf8_lossy(&body_bytes(response).await).into_owned()
    }

    fn header(response: &Response, name: header::HeaderName) -> &str {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn root_serves_the_built_svelte_app() {
        let response = fallback(Method::GET, Uri::from_static("/")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header(&response, header::CONTENT_TYPE), "text/html");
        assert_eq!(header(&response, header::CACHE_CONTROL), CACHE_INDEX);
        assert!(
            header(&response, header::CONTENT_SECURITY_POLICY).contains("frame-ancestors 'none'")
        );
        assert!(header(&response, header::CONTENT_SECURITY_POLICY).contains("connect-src 'self'"));
        assert_eq!(header(&response, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        let html = body_text(response).await;
        assert!(html.contains("id=\"app\""));
        assert!(!html.contains("WebUI not built"));
    }

    #[tokio::test]
    async fn spa_routes_fall_back_to_the_index() {
        let response = fallback(Method::GET, Uri::from_static("/topology/default")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(body_text(response).await.contains("<title>knect"));

        let response = fallback(Method::GET, Uri::from_static("/deep/link")).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_assets_and_files_are_not_masked_by_the_spa() {
        assert_eq!(
            fallback(Method::GET, Uri::from_static("/assets/missing.js"))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            fallback(Method::GET, Uri::from_static("/missing.js"))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            fallback(Method::GET, Uri::from_static("/missing.css"))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn embedded_javascript_is_served_with_immutable_caching() {
        let path = Assets::iter()
            .find(|path| path.ends_with(".js"))
            .expect("the build embeds at least one JavaScript asset");

        let uri = format!("/{path}");
        let response = fallback(Method::GET, Uri::try_from(uri.as_str()).expect("uri")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(header(&response, header::CONTENT_TYPE).contains("javascript"));
        assert_eq!(header(&response, header::CACHE_CONTROL), CACHE_IMMUTABLE);
        assert!(!body_bytes(response).await.is_empty());
    }

    #[tokio::test]
    async fn head_requests_keep_headers_but_drop_the_body() {
        let response = fallback(Method::HEAD, Uri::from_static("/")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header(&response, header::CONTENT_TYPE), "text/html");
        assert!(body_bytes(response).await.is_empty());
    }

    #[tokio::test]
    async fn traversal_attempts_are_rejected() {
        let response = fallback(Method::GET, Uri::from_static("/../Cargo.toml")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn index_is_embedded() {
        assert!(Assets::get(INDEX).is_some());
        assert!(Assets::iter().any(|path| path.starts_with("assets/")));
    }
}

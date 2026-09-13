//! Serves the config UI out of the binary.
//!
//! Compiled only under the `ui` feature, which embeds `web/dist` at build
//! time. Without it an ordinary `cargo build` needs no JavaScript
//! toolchain, which keeps CI and a plain dev build fast; the container
//! image turns it on after running the bundler.
//!
//! The UI is a single-page app, so any path that isn't a real asset
//! returns `index.html` and lets the client router decide. API routes are
//! registered before this fallback and therefore win.

use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist"]
struct Assets;

/// Serve an embedded asset, falling back to `index.html` for anything
/// that looks like a browser navigation.
///
/// The fallback is deliberately *not* unconditional. A single-page app
/// needs a refresh on `/history` to return the shell rather than a 404 —
/// but an API client asking for a route that doesn't exist must still get
/// a 404, not a page of HTML with a 200 on it. Navigation requests are
/// the ones that say they accept HTML, so that is the line.
pub async fn serve_asset(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(asset) = Assets::get(path) {
        return respond(path, asset);
    }
    if !wants_html(&headers) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match Assets::get("index.html") {
        Some(index) => respond("index.html", index),
        // Only reachable if the bundle was built empty, which the build
        // would normally have failed on.
        None => (StatusCode::NOT_FOUND, "UI bundle is missing").into_response(),
    }
}

fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"))
}

fn respond(path: &str, asset: rust_embed::EmbeddedFile) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    // Vite fingerprints everything under assets/, so those are immutable;
    // index.html must not be cached or a deploy would keep serving the
    // old bundle's script tags.
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, cache),
        ],
        asset.data,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::DevicePublisher;
    use crate::server::router;
    use crate::state::AppState;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use niles_core::{DeviceRegistry, EventBus};
    use std::sync::Arc;
    use tower::ServiceExt;

    #[derive(Clone)]
    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    fn app() -> axum::Router {
        router(AppState::new(
            Arc::new(DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            EventBus::default(),
        ))
    }

    /// A browser navigation: says it accepts HTML.
    async fn browse(uri: &str) -> (StatusCode, String, String) {
        send(
            Request::builder()
                .uri(uri)
                .header(header::ACCEPT, "text/html,application/xhtml+xml")
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    /// An API client: no HTML in Accept.
    async fn get(uri: &str) -> (StatusCode, String, String) {
        send(Request::builder().uri(uri).body(Body::empty()).unwrap()).await
    }

    async fn send(request: Request<Body>) -> (StatusCode, String, String) {
        let response = app().oneshot(request).await.unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            content_type,
            String::from_utf8_lossy(&body).into_owned(),
        )
    }

    #[tokio::test]
    async fn root_serves_the_app_shell() {
        let (status, content_type, body) = browse("/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/html"), "{content_type}");
        assert!(body.contains("<div id=\"root\">"), "served the SPA shell");
    }

    #[tokio::test]
    async fn a_navigation_to_an_unknown_path_falls_back_to_the_app() {
        // A refresh on a client-side route must not 404.
        let (status, content_type, _) = browse("/some/client/route").await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/html"));
    }

    #[tokio::test]
    async fn an_api_client_asking_for_a_missing_route_still_gets_404() {
        // Otherwise every mistyped endpoint answers 200 with a page of
        // HTML, and a client has no way to tell it went wrong.
        let (status, _, _) = get("/no/such/endpoint").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn api_routes_still_win_over_the_ui_fallback() {
        // The fallback is registered last precisely so this holds.
        let (status, content_type, body) = get("/healthz").await;
        assert_eq!(status, StatusCode::OK);
        assert!(!content_type.starts_with("text/html"), "{content_type}");
        assert_eq!(body, "ok");
    }

    #[tokio::test]
    async fn fingerprinted_assets_are_cached_and_the_shell_is_not() {
        // index.html must never be cached, or a deploy keeps serving the
        // previous bundle's script tags.
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-cache"
        );
    }
}

use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;

use super::{state::AppState, websocket};

const INDEX_HTML: &str = include_str!("../../overlay/index.html");
const OVERLAY_CSS: &str = include_str!("../../overlay/overlay.css");
const OVERLAY_JS: &str = include_str!("../../overlay/overlay.js");

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/overlay.css", get(overlay_css))
        .route("/overlay.js", get(overlay_js))
        .route("/health", get(health))
        .route("/artwork", get(artwork))
        .route("/ws", get(websocket::upgrade))
        .with_state(state)
}

async fn index() -> axum::response::Html<&'static str> {
    axum::response::Html(INDEX_HTML)
}

async fn overlay_css() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        OVERLAY_CSS,
    )
}

async fn overlay_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/javascript; charset=utf-8"),
        )],
        OVERLAY_JS,
    )
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    available: bool,
    source: Option<String>,
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let current = state.media.borrow().clone();
    Json(HealthResponse {
        status: "ok",
        available: current.available,
        source: current.source,
    })
}

async fn artwork(State(state): State<AppState>) -> Response {
    let Some(snapshot) = state.artwork.snapshot().await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mut response = Response::new(Body::from(snapshot.bytes.as_ref().to_vec()));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&snapshot.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
}

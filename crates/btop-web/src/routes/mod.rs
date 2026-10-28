//! Router and static assets.

pub mod dashboard;
pub mod events;
pub mod settings;

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use rust_embed::RustEmbed;
use tower_http::trace::TraceLayer;

use crate::AppState;

/// The one-line body of every refusal, shared with the integration tests.
pub const READ_ONLY_REFUSAL: &str = "read-only mode: mutating requests are refused";

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(dashboard::index))
        .route("/events", get(events::events))
        .route("/config", post(settings::update))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/assets/{*path}", get(asset))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            readonly_guard,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// ACC-SAFE-12-READONLY is a server boundary, not a UI state: every method
/// that can change state is refused here, before routing reaches a handler.
async fn readonly_guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let method = request.method();
    if state.readonly
        && method != Method::GET
        && method != Method::HEAD
        && method != Method::OPTIONS
    {
        return (StatusCode::FORBIDDEN, READ_ONLY_REFUSAL).into_response();
    }
    next.run(request).await
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(state): State<AppState>) -> Response {
    if (*state.snapshot.borrow()).is_some() {
        (StatusCode::OK, "ready").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "starting").into_response()
    }
}

async fn asset(Path(path): Path<String>) -> Response {
    let Some(file) = Assets::get(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mimetype = file.metadata.mimetype();
    // mime_guess names no charset; the text assets are UTF-8.
    let content_type = if mimetype.starts_with("text/") {
        format!("{mimetype}; charset=utf-8")
    } else {
        mimetype.to_owned()
    };
    (
        [(header::CONTENT_TYPE, content_type)],
        file.data.into_owned(),
    )
        .into_response()
}

//! Router and static assets.

pub mod agents;
pub mod dashboard;
pub mod events;
pub mod settings;

use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
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

/// DEF-VIEW-98-ASSET-STALE: every asset URL the page loads carries the build
/// stamp, so a reload after a deploy fetches one matching set. The stamp
/// covers these files: the scripts and the stylesheet decide behaviour and
/// markup together, and the icon is named by URL from the same page.
pub const STAMPED_ASSETS: [&str; 6] = [
    "icon.svg",
    "app.css",
    "app.js",
    "theme.js",
    "htmx.min.js",
    "htmx-sse.min.js",
];

/// The build stamp: the crate version and every stamped file's embedded
/// sha256, hashed together. A changed asset or a version bump makes a new
/// stamp and a new URL; an unchanged build keeps its stamp, so the browser
/// cache still helps. `DefaultHasher` is deterministic within a build, which
/// is all the page and its asset responses need.
pub fn asset_stamp() -> String {
    let files: Vec<(&str, Option<[u8; 32]>)> = STAMPED_ASSETS
        .iter()
        .map(|name| {
            (
                *name,
                Assets::get(name).map(|file| file.metadata.sha256_hash()),
            )
        })
        .collect();
    stamp(env!("CARGO_PKG_VERSION"), &files)
}

fn stamp(version: &str, files: &[(&str, Option<[u8; 32]>)]) -> String {
    let mut hasher = DefaultHasher::new();
    version.hash(&mut hasher);
    for (name, hash) in files {
        name.hash(&mut hasher);
        hash.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(dashboard::index))
        .route("/events", get(events::events))
        .route("/agents", get(agents::agents))
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

/// ACC-NET-143-AGENT-MODES: an agent offers telemetry and liveness only - no
/// page, no assets, no config menu and no mutating route, so the read-only
/// guard has nothing to refuse.
///
/// ACC-NET-144-ACCESS-KEY: the access guard is layered on the whole router,
/// inside the trace layer and outside every handler, so no request reaches a
/// handler or yields a fragment without the key.
pub fn agent_router(state: AppState) -> Router {
    Router::new()
        .route("/events", get(events::events))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .layer(middleware::from_fn_with_state(state.clone(), access_guard))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// ACC-NET-144-ACCESS-KEY: every request to an agent listener carries the
/// agent's key as `Authorization: Bearer KEY`. A missing or wrong key is
/// answered with 401 and an empty body before the handler runs, so no
/// telemetry byte is collected or sent. A state without a key fails closed.
async fn access_guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match (presented, state.access_key.as_deref()) {
        (Some(presented), Some(key)) if ct_eq(presented.as_bytes(), key.as_bytes()) => {
            next.run(request).await
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// ACC-NET-144-ACCESS-KEY: compare two byte strings without stopping at the
/// first difference. `fold` has no early exit and `|` combines every byte, so
/// only the length is observable; the length itself is public, 43 base64url
/// characters for every key, so the check before the scan leaks nothing about
/// the secret. The implementation is local rather than `subtle` because the
/// test observes that the scan visits every byte, which a library primitive
/// does not expose.
pub(crate) fn ct_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && ct_scan(left.iter().copied().zip(right.iter().copied())) == 0
}

fn ct_scan(pairs: impl Iterator<Item = (u8, u8)>) -> u8 {
    pairs.fold(0, |difference, (left, right)| difference | (left ^ right))
}

/// The `agent` value of a query string, without percent-decoding: a name
/// matches `[A-Za-z0-9._-]+`, so it never needs it. An empty value selects the
/// default agent.
pub fn agent_name(query: Option<&str>) -> Option<&str> {
    query?
        .split('&')
        .find_map(|pair| pair.strip_prefix("agent="))
        .filter(|name| !name.is_empty())
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

/// ACC-NET-143-AGENT-MODES and ACC-PERF-136-SLEEP: readiness is the serving
/// listener, not a sample. The sampler parks while no viewer is connected, so
/// an idle server would never report one; a page request wakes it within one
/// interval, and a client with its agents down still serves a truthful page.
async fn readyz() -> &'static str {
    "ready"
}

/// DEF-VIEW-59-ASSET-CACHE: rust-embed hashes each file at build time, so the
/// ETag changes exactly when the bytes do. `no-cache` stores the asset but
/// makes the browser revalidate it on the next load instead of reusing it.
/// DEF-VIEW-98-ASSET-STALE: with the stamp in the query, a new build asks for
/// a URL this cache has never seen and gets the new bytes; the same build
/// revalidates to 304, so the stamp costs one conditional request.
async fn asset(Path(path): Path<String>, headers: HeaderMap) -> Response {
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
    let mut etag = String::with_capacity(66);
    etag.push('"');
    for byte in file.metadata.sha256_hash() {
        write!(etag, "{byte:02x}").expect("writing to a String");
    }
    etag.push('"');
    let fresh = headers.get(header::IF_NONE_MATCH).is_some_and(|value| {
        value.to_str().is_ok_and(|tags| {
            tags.split(',')
                .any(|tag| tag.trim().trim_start_matches("W/") == etag || tag.trim() == "*")
        })
    });
    if fresh {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CACHE_CONTROL, "no-cache".to_owned()),
            ],
        )
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, "no-cache".to_owned()),
        ],
        file.data.into_owned(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ACC-NET-144-ACCESS-KEY: the comparison is constant time because it
    /// never stops at the first differing byte; the count proves the scan
    /// visited every pair. A length mismatch is refused.
    #[test]
    fn the_key_comparison_visits_every_byte() {
        assert!(ct_eq(b"abcdef", b"abcdef"));
        assert!(!ct_eq(b"abcdef", b"abcdeg"));
        assert!(!ct_eq(b"xbcdef", b"abcdef"));
        assert!(!ct_eq(b"abcdef", b"abcde"));
        assert!(!ct_eq(b"", b"a"));

        let mut visited = 0;
        let difference = ct_scan(
            [(1u8, 9u8), (2, 2), (3, 3)]
                .into_iter()
                .inspect(|_| visited += 1),
        );
        assert_eq!(difference, 8);
        assert_eq!(visited, 3, "the scan stopped before every byte was read");
    }

    /// A missing file would leave its URL with a stamp that ignores it, so
    /// every stamped name must exist in the embedded folder.
    #[test]
    fn every_stamped_asset_is_embedded() {
        for name in STAMPED_ASSETS {
            assert!(Assets::get(name).is_some(), "{name} is not embedded");
        }
    }

    /// DEF-VIEW-98-ASSET-STALE: a changed asset or a changed version makes a
    /// new stamp; the same build keeps its stamp, so the cache still helps.
    #[test]
    fn a_changed_build_changes_the_stamp() {
        let files = [("app.css", Some([1u8; 32])), ("app.js", Some([2u8; 32]))];
        let base = stamp("1.0.0", &files);
        assert_eq!(base, stamp("1.0.0", &files));
        assert_ne!(
            base,
            stamp(
                "1.0.0",
                &[("app.css", Some([9u8; 32])), ("app.js", Some([2u8; 32]))]
            )
        );
        assert_ne!(base, stamp("1.0.1", &files));
        assert_ne!(
            base,
            stamp("1.0.0", &[("app.css", None), ("app.js", Some([2u8; 32]))])
        );
    }
}

//! ACC-NET-143-AGENT-MODES: the top-bar agent strip, one chip per agent the
//! client shows, each with its sample age.
//!
//! The page renders the strip and an htmx poll replaces it every second, so an
//! age ticks and a stopped agent turns stale or unavailable while the page
//! stays open. Nothing here samples: the client's upstream streams supply
//! every value the strip shows.

use askama::Template;
use axum::extract::{RawQuery, State};
use axum::http::header;
use axum::response::{Html, IntoResponse};

use crate::AppState;
use crate::client::{Chip, Client};
use crate::error::AppError;
use crate::routes::agent_name;

#[derive(Template)]
#[template(path = "partials/agents.html")]
struct AgentsTemplate<'a> {
    chips: &'a [Chip],
    /// The poll's own URL, carrying the selected agent so the refreshed strip
    /// marks the same one.
    strip_url: String,
}

/// The strip for every configured agent, the selected one from `?agent=NAME`
/// and the first configured agent by default.
pub async fn agents(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
) -> Result<impl IntoResponse, AppError> {
    let client = state.client.as_deref().ok_or(AppError::NotFound)?;
    let link = client
        .select(agent_name(query.as_deref()))
        .ok_or(AppError::NotFound)?;
    // A cached strip would freeze every age; each poll must reach the client.
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Html(render(client, &link.name)?),
    ))
}

/// The strip fragment for `client`, `selected` marked.
pub fn render(client: &Client, selected: &str) -> Result<String, askama::Error> {
    let chips = client.chips(selected);
    AgentsTemplate {
        chips: &chips,
        strip_url: format!("agents?agent={selected}"),
    }
    .render()
}

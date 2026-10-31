//! Full-page dashboard render.

use std::time::Duration;

use askama::Template;
use axum::extract::{RawQuery, State};
use axum::response::Html;

use crate::AppState;
use crate::client::Chip;
use crate::collector::Snapshot;
use crate::config::{self, CONFIG_PAGES, SERVER_OPTIONS};
use crate::error::AppError;
use crate::routes::agent_name;
use crate::routes::settings::CfgRow;
use crate::theme::ThemeChoice;

/// ACC-VIEW-40-GADGETS: every panel's `data-panel` id and its name in the
/// config popover, in default page order (ACC-LAYOUT-157). `assets/theme.js`
/// carries the same ids with their bands and stored order.
pub const PANELS: [(&str, &str); 10] = [
    ("cpu-stat", "CPU history"),
    ("mem-stat", "Memory history"),
    ("proc-stat", "Process count"),
    ("cores", "Cores"),
    ("cpu-gauge", "CPU gauge"),
    ("mem-gauge", "Memory gauge"),
    ("gpu", "GPU"),
    ("net", "Network"),
    ("storage", "Storage"),
    ("proc", "Processes"),
];

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    title: &'a str,
    snapshot: &'a Snapshot,
    css: &'a str,
    themes: &'a [ThemeChoice],
    initial_theme: &'a str,
    initial_filter: &'a str,
    initial_proc_view: &'a str,
    readonly: bool,
    version: &'a str,
    /// DEF-VIEW-98-ASSET-STALE: the stamp on every asset URL.
    asset_stamp: &'a str,
    host: Option<&'a str>,
    container: Option<&'a str>,
    update_ms: u64,
    /// The time the charts span at this interval (ACC-GRAPH-98-ANIM-CLOCK).
    window_ms: u64,
    panels: &'a [(&'a str, &'a str)],
    /// ACC-CONF-61-CONFIG-PAGES: each config page's id and its server rows.
    pages: Vec<(&'static str, Vec<CfgRow>)>,
    /// The server group's note line.
    note: String,
    /// ACC-NET-143-AGENT-MODES: `events`, or `events?agent=NAME` in client
    /// mode, the stream this page subscribes to.
    sse_url: String,
    /// One chip per agent in client mode; empty in the server modes.
    chips: Vec<Chip>,
    /// ACC-NET-143-AGENT-MODES: the strip's own poll URL.
    strip_url: String,
}

/// The page for the latest snapshot, or an empty one before the first sample
/// (and always in client mode, where the panels fill from the relayed
/// fragments), with the live configuration and the config file the menu
/// writes. `selected` names the client's agent whose stream the page
/// subscribes to; `None` in the server modes.
pub fn render(
    state: &AppState,
    file: Option<&std::path::Path>,
    selected: Option<&str>,
) -> Result<String, askama::Error> {
    // One page load copies the snapshot out of the shared frame; the frames
    // are published behind an `Arc` for the SSE path, not for this one.
    let snapshot = (*state.snapshot.borrow())
        .as_ref()
        .map(|frame| frame.snapshot.clone())
        .unwrap_or_default();
    let config = state.config.borrow().clone();
    let (sse_url, chips, strip_url) = match (state.client.as_deref(), selected) {
        (Some(client), Some(name)) => (
            format!("events?agent={name}"),
            client.chips(name),
            format!("agents?agent={name}"),
        ),
        _ => ("events".to_string(), Vec::new(), String::new()),
    };
    let note = match (state.readonly, file) {
        (true, _) => "read-only: restart without --readonly to change".to_string(),
        (false, Some(path)) => format!(
            "changes apply to every viewer and are saved to {} (btop-web.conf; $XDG_CONFIG_HOME/btop-web/ when set, else ~/.config/btop-web/; btop.conf stays the baseline)",
            path.display()
        ),
        (false, None) => {
            "changes apply to every viewer until restart; no btop-web.conf path: set HOME or XDG_CONFIG_HOME"
                .to_string()
        }
    };
    IndexTemplate {
        title: &state.title,
        snapshot: &snapshot,
        css: &state.theme_css,
        themes: &state.themes,
        initial_theme: config.theme_id(),
        initial_filter: &state.initial_filter,
        initial_proc_view: config::initial_proc_view(state.proc_view_flag, config.proc_tree),
        readonly: state.readonly,
        version: env!("CARGO_PKG_VERSION"),
        asset_stamp: &state.asset_stamp,
        host: state.host.as_deref(),
        container: state.container.as_deref(),
        update_ms: config.update_ms,
        window_ms: crate::chart::window(config.window_ms(), config.update_ms),
        panels: &PANELS,
        pages: CONFIG_PAGES
            .iter()
            .map(|page| {
                // ACC-NET-143-AGENT-MODES: a client applies none of the
                // server options - the agents own the interval, the filters
                // and the file - so its menu carries no server group.
                let rows = if state.client.is_some() {
                    Vec::new()
                } else {
                    SERVER_OPTIONS
                        .iter()
                        .filter(|option| option.page == *page)
                        .map(|option| CfgRow::new(option, &config, String::new()))
                        .collect()
                };
                (*page, rows)
            })
            .collect(),
        note,
        sse_url,
        chips,
        strip_url,
    }
    .render()
}

/// ACC-PERF-136-SLEEP: the page request wakes a parked sampler and waits for
/// its first sample, at most one interval, so the first frame carries current
/// values. ACC-PERF-137-WAKE-BURST: a sample still inside the interval is
/// served at once, and the GPU sweep never holds the page.
pub async fn index(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
) -> Result<Html<String>, AppError> {
    // ACC-NET-143-AGENT-MODES: in client mode the request names the agent to
    // show; an unknown name is a 404, so the parameter never becomes anything
    // but a configured label.
    let selected = match state.client.as_deref() {
        Some(client) => Some(
            client
                .select(agent_name(query.as_deref()))
                .ok_or(AppError::NotFound)?
                .name
                .clone(),
        ),
        None => None,
    };
    if state.client.is_none() {
        // ACC-GRAPH-164-NET-SCALE: a page load is the only thing that lowers
        // the network axis; the reset lands before the sampler's next tick, so
        // the first sample after the reload sets the ceiling to the decade above
        // the measured peak.
        let proc_root = state.config.borrow().proc_root.clone();
        crate::collector::net::reset_scale(&proc_root);
        let interval = Duration::from_millis(state.config.borrow().update_ms.max(1));
        let mut receiver = state.snapshot.clone();
        // The state's receiver never marks its value seen, so a clone must, or
        // `changed` would report an old sample as new.
        receiver.borrow_and_update();
        if state.wake.stale(interval) {
            state.wake.request();
            let _ = tokio::time::timeout(interval, receiver.changed()).await;
        }
    }
    let file = state.config_file.lock().await.clone();
    Ok(Html(render(&state, file.as_deref(), selected.as_deref())?))
}

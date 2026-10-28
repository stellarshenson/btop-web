//! Full-page dashboard render.

use askama::Template;
use axum::extract::State;
use axum::response::Html;

use crate::AppState;
use crate::collector::Snapshot;
use crate::config::{self, CONFIG_PAGES, SERVER_OPTIONS};
use crate::error::AppError;
use crate::routes::settings::CfgRow;
use crate::theme::ThemeChoice;

/// ACC-VIEW-40-GADGETS: every panel's `data-panel` id and its name in the
/// config popover, in page order. `assets/theme.js` carries the same ids with
/// their bands.
pub const PANELS: [(&str, &str); 10] = [
    ("cpu-stat", "CPU history"),
    ("mem-stat", "Memory history"),
    ("proc-stat", "Process count"),
    ("net", "Network"),
    ("cpu-gauge", "CPU gauge"),
    ("mem-gauge", "Memory gauge"),
    ("gpu", "GPU"),
    ("cores", "Cores"),
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
    host: Option<&'a str>,
    container: Option<&'a str>,
    update_ms: u64,
    panels: &'a [(&'a str, &'a str)],
    /// ACC-CONF-61-CONFIG-PAGES: each config page's id and its server rows.
    pages: Vec<(&'static str, Vec<CfgRow>)>,
    /// The server group's note line.
    note: String,
}

/// The page for the latest snapshot, or an empty one before the first sample,
/// with the live configuration and the config file the menu writes.
pub fn render(state: &AppState, file: Option<&std::path::Path>) -> Result<String, askama::Error> {
    let snapshot = (*state.snapshot.borrow()).clone().unwrap_or_default();
    let config = state.config.borrow().clone();
    let note = match (state.readonly, file) {
        (true, _) => "read-only: start with --no-readonly to change".to_string(),
        (false, Some(path)) => format!(
            "changes apply to every viewer and are saved to {}",
            path.display()
        ),
        (false, None) => {
            "changes apply to every viewer until restart; no config file was loaded (-c PATH)"
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
        host: state.host.as_deref(),
        container: state.container.as_deref(),
        update_ms: config.update_ms,
        panels: &PANELS,
        pages: CONFIG_PAGES
            .iter()
            .map(|page| {
                let rows = SERVER_OPTIONS
                    .iter()
                    .filter(|option| option.page == *page)
                    .map(|option| CfgRow::new(option, &config, String::new()))
                    .collect();
                (*page, rows)
            })
            .collect(),
        note,
    }
    .render()
}

pub async fn index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let file = state.config_file.lock().await.clone();
    Ok(Html(render(&state, file.as_deref())?))
}

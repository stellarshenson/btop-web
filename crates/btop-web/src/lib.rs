//! btop-web: btop's system telemetry served to a web browser.
//!
//! The library target exposes the application state and router so main,
//! integration tests and the e2e test share one construction path.

pub mod chart;
pub mod collector;
pub mod config;
pub mod error;
pub mod routes;
pub mod theme;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, watch};

use crate::collector::Snapshot;
use crate::config::Config;
use crate::theme::ThemeChoice;

#[derive(Clone)]
pub struct AppState {
    /// Latest sample from the collector; `None` until the first one lands.
    pub snapshot: watch::Receiver<Option<Snapshot>>,
    /// ACC-SAFE-12-READONLY: the guard refuses every mutating method.
    pub readonly: bool,
    /// CSS custom properties for every embedded theme and both modes.
    pub theme_css: Arc<str>,
    pub themes: Arc<[ThemeChoice]>,
    /// ACC-CONF-61-CONFIG-PAGES: the configuration the sampler runs on. A
    /// page renders from it, so a new page load shows the live values; the
    /// config menu replaces it with the guard off.
    pub config: watch::Sender<Config>,
    /// The `-c` file the config menu rewrites, `None` when none was loaded.
    /// The lock serialises writes, so two requests never lose a line.
    pub config_file: Arc<Mutex<Option<PathBuf>>>,
    /// ACC-CONF-63-TITLE-FLAG: the page `<title>`, `--title` or btop-web.
    pub title: Arc<str>,
    /// The configured process filter, rendered into the filter input.
    pub initial_filter: Arc<str>,
    /// ACC-CONF-60-VIEW-FLAG: `--proc-view`, which outranks `proc_tree` in
    /// `config::initial_proc_view`.
    pub proc_view_flag: Option<&'static str>,
    /// btop `Tools::hostname()`: `gethostname`, read once; `None` when the
    /// kernel does not answer.
    pub host: Option<Arc<str>>,
    /// ACC-SCOPE-64-CONTAINER: the CONTAINER chip's title; `None` on a host.
    pub container: Option<Arc<str>>,
}

impl AppState {
    pub fn new(snapshot: watch::Receiver<Option<Snapshot>>, readonly: bool) -> Self {
        let pairs = theme::embedded_pairs();
        Self {
            snapshot,
            readonly,
            theme_css: theme::render_css(&pairs).into(),
            themes: pairs.iter().map(|pair| pair.choice()).collect(),
            config: watch::Sender::new(Config::defaults()),
            config_file: Arc::new(Mutex::new(None)),
            title: "btop-web".into(),
            initial_filter: "".into(),
            proc_view_flag: None,
            host: nix::unistd::gethostname()
                .ok()
                .and_then(|name| name.into_string().ok())
                .map(Into::into),
            container: None,
        }
    }
}

pub fn app(state: AppState) -> axum::Router {
    routes::router(state)
}

//! btop-web: btop's system telemetry served to a web browser.
//!
//! The library target exposes the application state and router so main,
//! integration tests and the e2e test share one construction path.

pub mod access;
pub mod chart;
pub mod client;
pub mod collector;
pub mod config;
pub mod error;
pub mod routes;
pub mod secret;
pub mod theme;
pub mod tls;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, watch};

use crate::collector::{Frame, Wake};
use crate::config::Config;
use crate::theme::ThemeChoice;

#[derive(Clone)]
pub struct AppState {
    /// Latest frame from the collector; `None` until the first one lands.
    /// Shared behind an `Arc`, so a handler clones a pointer.
    pub snapshot: watch::Receiver<Option<Arc<Frame>>>,
    /// ACC-PERF-136-SLEEP: the gate that wakes the parked sampler; `main`
    /// replaces the detached one [`AppState::new`] builds with the sampler's.
    pub wake: Arc<Wake>,
    /// ACC-SAFE-12-READONLY: the guard refuses every mutating method.
    pub readonly: bool,
    /// CSS custom properties for every embedded theme and both modes.
    pub theme_css: Arc<str>,
    pub themes: Arc<[ThemeChoice]>,
    /// DEF-VIEW-98-ASSET-STALE: the build stamp every asset URL carries, so a
    /// reload after a deploy fetches a matching set.
    pub asset_stamp: Arc<str>,
    /// ACC-CONF-61-CONFIG-PAGES: the configuration the sampler runs on. A
    /// page renders from it, so a new page load shows the live values; the
    /// config menu replaces it with the guard off.
    pub config: watch::Sender<Config>,
    /// ACC-CONF-148-OWN-FILE: btop-web.conf, the file the config menu writes,
    /// resolved at startup from XDG_CONFIG_HOME or HOME; `None` when neither
    /// resolves, so a change applies until restart. The lock serialises
    /// writes, so two requests never lose a line.
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
    /// ACC-NET-143-AGENT-MODES: the agents this process relays in client
    /// mode; `None` in standalone and agent mode, where the sampler is the
    /// frame source.
    pub client: Option<Arc<crate::client::Client>>,
    /// ACC-NET-144-ACCESS-KEY: the key an agent listener demands; `None` in
    /// standalone and client mode, where no listener demands one.
    pub access_key: Option<Arc<str>>,
}

impl AppState {
    pub fn new(snapshot: watch::Receiver<Option<Arc<Frame>>>, readonly: bool) -> Self {
        let pairs = theme::embedded_pairs();
        Self {
            snapshot,
            wake: Wake::detached(),
            readonly,
            theme_css: theme::render_css(&pairs).into(),
            themes: pairs.iter().map(|pair| pair.choice()).collect(),
            asset_stamp: routes::asset_stamp().into(),
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
            client: None,
            access_key: None,
        }
    }
}

pub fn app(state: AppState) -> axum::Router {
    routes::router(state)
}

//! Binary entry: arguments, tracing, configuration, collector and server.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use btop_web::AppState;
use btop_web::collector;
use btop_web::config::{Config, PROC_VIEWS, THEMES, default_config_text, theme_index};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, watch};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "\
Usage: btop-web [OPTIONS]

Options:
  -c, --config <PATH>   Read configuration from PATH after the embedded defaults
      --default-config  Print the embedded default configuration and exit
  -d, --debug           Log at debug level
  -f, --filter <TEXT>   Initial process filter, shown in the filter input
  -u, --update <MS>     Sampling interval in milliseconds, 100 to 86400000
                        (default 250)
      --theme <N>       Theme number: 0 Default, 1 Everforest Medium,
                        2 Flexoki, 3 Gruvbox, 4 Solarized
      --proc-view <VIEW>
                        Starting process list for a viewer with no stored
                        choice: top, all or tree (default tree when btop's
                        proc_tree is True, otherwise top)
      --title <TEXT>    Page title in the browser tab (default btop-web)
      --readonly        Refuse every mutating request (default)
      --no-readonly     Turn the read-only guard off
  -h, --help            Print this help and exit
  -v, --version         Print the version and exit

btop's terminal-only flags -b, -l, -p, -t and --utf-force are refused.
";

#[tokio::main]
async fn main() -> Result<()> {
    let mut config_path: Option<PathBuf> = None;
    let mut readonly = true;
    let mut debug = false;
    let mut update_ms: Option<u64> = None;
    let mut theme: Option<usize> = None;
    let mut filter: Option<String> = None;
    let mut proc_view: Option<&'static str> = None;
    let mut title: Option<String> = None;
    let mut args = std::env::args().skip(1);
    // Own loop: btop's argumentParser is the same token loop; a crate would be a new dependency.
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--default-config" => {
                print!("{}", default_config_text());
                return Ok(());
            }
            "-c" | "--config" => {
                let path = args.next().context("-c/--config requires a path")?;
                config_path = Some(PathBuf::from(path));
            }
            "-d" | "--debug" => debug = true,
            "-f" | "--filter" => {
                filter = Some(args.next().context("-f/--filter requires a string")?);
            }
            "-u" | "--update" => {
                let value = args.next().context("-u/--update requires milliseconds")?;
                let millis: u64 = value.parse().map_err(|_| {
                    anyhow::anyhow!("-u/--update expects milliseconds, got `{value}`")
                })?;
                if !(100..=86_400_000).contains(&millis) {
                    anyhow::bail!("-u/--update must be in 100..=86400000, got {millis}");
                }
                update_ms = Some(millis);
            }
            "--theme" => {
                let value = args.next().context("--theme requires a number")?;
                let index = value
                    .parse::<i64>()
                    .ok()
                    .and_then(theme_index)
                    .with_context(|| {
                        format!("theme must be in 0..={}, got {value}", THEMES.len() - 1)
                    })?;
                theme = Some(index);
            }
            "--proc-view" => {
                let value = args.next();
                let Some(view) = PROC_VIEWS
                    .into_iter()
                    .find(|view| value.as_deref() == Some(*view))
                else {
                    eprintln!("btop-web: --proc-view requires top, all or tree");
                    std::process::exit(2);
                };
                proc_view = Some(view);
            }
            "--title" => match args.next() {
                Some(text) if !text.trim().is_empty() => title = Some(text),
                _ => {
                    eprintln!("btop-web: --title requires a non-empty text");
                    std::process::exit(2);
                }
            },
            "--readonly" => readonly = true,
            "--no-readonly" => readonly = false,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "-v" | "-V" | "--version" => {
                println!("btop-web {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-b" | "--boxes" | "-l" | "--low-color" | "-p" | "--preset" | "-t" | "--tty_on"
            | "--utf-force" => {
                eprintln!("btop-web: {arg} is terminal-only and not applicable");
                std::process::exit(2);
            }
            other => anyhow::bail!("unknown argument `{other}`\n{USAGE}"),
        }
    }

    let env_filter = if debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let mut config = Config::load(config_path.as_deref()).context("invalid configuration")?;
    config.readonly = readonly;
    if let Some(millis) = update_ms {
        config.update_ms = millis;
    }
    if let Some(index) = theme {
        config.theme = index;
    }
    if let Some(filter) = filter {
        config.proc_filter = filter;
    }
    config.container = collector::container::detect(&config.proc_root);
    match &config.container {
        Some(container) => tracing::info!(
            scope = %"container",
            runtime = %container.runtime,
            signal = ?container.signal,
            cgroup = %container
                .cgroup
                .as_deref()
                .map_or_else(|| "unavailable".to_string(), |path| path.display().to_string()),
            network = if container.own_netns {
                "own namespace"
            } else {
                "host namespace, withheld"
            },
            "ACC-SCOPE-64: panels show only this container"
        ),
        None => tracing::info!(scope = %"host", "btop collection, no container found"),
    }
    tracing::debug!(
        theme = config.theme_id(),
        filter = %config.proc_filter,
        "effective options"
    );

    let (config_sender, config_receiver) = watch::channel(config.clone());
    let (snapshot, sampler) = collector::spawn(config_receiver);
    let mut state = AppState::new(snapshot, config.readonly);
    state.config = config_sender;
    state.config_file = Arc::new(Mutex::new(config_path));
    state.initial_filter = config.proc_filter.clone().into();
    state.proc_view_flag = proc_view;
    if let Some(title) = title {
        state.title = title.into();
    }
    state.container = config
        .container
        .as_ref()
        .map(|container| container.title().into());
    let app = btop_web::app(state);

    let listener = TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("cannot bind {}", config.listen))?;
    tracing::info!(
        listen = %config.listen,
        update_ms = config.update_ms,
        theme = config.theme_id(),
        proc_root = %config.proc_root.display(),
        readonly = config.readonly,
        "btop-web listening"
    );

    // Graceful shutdown waits for open connections; an SSE stream ends only
    // when the sampler's sender drops, so the sampler stops first.
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            sampler.abort();
        })
        .await
        .context("server failed")
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM handler");
    #[cfg(unix)]
    let terminate = async {
        terminate.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

//! Binary entry: arguments, tracing, configuration, collector and server.

use std::os::fd::RawFd;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use btop_web::AppState;
use btop_web::client;
use btop_web::collector;
use btop_web::config::{
    Config, Mode, PROC_VIEWS, THEMES, default_config_text, parse_connect, theme_index,
    web_config_path,
};
use btop_web::secret::Source as SecretSource;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, watch};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "\
Usage: btop-web [OPTIONS]

Options:
      --agent           Serve telemetry only: /events, /healthz and /readyz,
                        no page; a client connects to this process. The access
                        key is printed once on the start that creates it and
                        kept at $XDG_CONFIG_HOME/btop-web/access-key
      --client          Sample nothing; serve the dashboard from the agents
                        named by --connect (mode = client in btop-web.conf).
                        Every agent needs a `connect_key = NAME=PATH` line in
                        the configuration file; the key is never a flag
      --connect <NAME=URL>
                        An agent the client shows, as NAME=http://HOST:PORT;
                        repeatable, NAME matches [A-Za-z0-9._-] and is unique;
                        the first agent is selected, the strip switches
      --tls-cert <PATH>  The agent listener's certificate in PEM, with
                        --tls-key; without both, the agent generates a pair on
                        its first start at $XDG_CONFIG_HOME/btop-web/agent.crt
                        and agent.key and reuses it afterwards
      --tls-cert-fd <N>  Read that certificate once from inherited descriptor N
      --tls-key <PATH>   The agent listener's private key in PEM, with
                        --tls-cert; a given pair is never written to disk. An
                        encrypted PKCS#8 key needs a passphrase source
      --tls-key-fd <N>   Read that private key once from inherited descriptor N
      --tls-key-passphrase-env <VAR>
                        Read the encrypted key's passphrase from the
                        environment variable VAR
      --tls-key-passphrase-file <PATH>
                        Read the encrypted key's passphrase from a file
      --tls-key-passphrase-fd <N>
                        Read the encrypted key's passphrase once from
                        inherited descriptor N
      --access-key-file <PATH>
                        The agent's access key, read from a file instead of
                        $XDG_CONFIG_HOME/btop-web/access-key; a given key is
                        never created and never printed
      --access-key-fd <N>
                        Read the agent's access key once from inherited
                        descriptor N
      --known-agents <PATH>
                        The client's pin file, instead of
                        $XDG_CONFIG_HOME/btop-web/known_agents; a named file
                        must exist
  -c, --config <PATH>   Read configuration from PATH after the embedded defaults
      --default-config  Print the embedded default configuration and exit
  -d, --debug           Log at debug level
  -f, --filter <TEXT>   Initial process filter, shown in the filter input
  -u, --update <MS>     Sampling interval in milliseconds, 100 to 86400000
                        (default 250)
      --theme <N>       Theme number: 0 Default, 1 Everforest Medium,
                        2 Flexoki, 3 Gruvbox, 4 Solarized, 5 Synthwave,
                        6 Terracotta, 7 Ultraviolet, 8 Cyberpunk, 9 Steel
      --proc-view <VIEW>
                        Starting process list for a viewer with no stored
                        choice: top, all or tree (default tree when btop's
                        proc_tree is True, otherwise top)
      --title <TEXT>    Page title in the browser tab (default btop-web)
      --readonly        Refuse every mutating request (default)
      --no-readonly     Turn the read-only guard off; config menu edits are
                        saved to btop-web.conf at
                        $XDG_CONFIG_HOME/btop-web/btop-web.conf, or at
                        ~/.config/btop-web/btop-web.conf when XDG_CONFIG_HOME
                        is unset; btop.conf is never written
  -h, --help            Print this help and exit
  -v, --version         Print the version and exit

Secret values are never flags: a certificate, key, passphrase or access key is
named by a path, a descriptor number or an environment variable name.

btop's terminal-only flags -b, -l, -p, -t and --utf-force are refused.
";

fn main() -> Result<()> {
    // Two workers cut the runtime's thread stacks: 69 threads -> 7 with a
    // viewer, while the sampler keeps its own thread and the 250 ms tick holds
    // with ten viewers.
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> Result<()> {
    let mut config_path: Option<PathBuf> = None;
    let mut readonly = true;
    let mut debug = false;
    let mut update_ms: Option<u64> = None;
    let mut theme: Option<usize> = None;
    let mut filter: Option<String> = None;
    let mut proc_view: Option<&'static str> = None;
    let mut title: Option<String> = None;
    let mut mode: Option<Mode> = None;
    let mut connects = Vec::new();
    let mut tls_cert: Option<SecretSource> = None;
    let mut tls_key: Option<SecretSource> = None;
    let mut tls_key_passphrase_env: Option<String> = None;
    let mut tls_key_passphrase_file: Option<SecretSource> = None;
    let mut access_key: Option<SecretSource> = None;
    let mut known_agents: Option<PathBuf> = None;
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
            // ACC-NET-143-AGENT-MODES: standalone is the absence of a mode
            // flag; the two other roles are explicit.
            "--agent" | "--client" => {
                let wanted = if arg == "--agent" {
                    Mode::Agent
                } else {
                    Mode::Client
                };
                if mode.is_some_and(|mode| mode != wanted) {
                    anyhow::bail!("--agent and --client are exclusive");
                }
                mode = Some(wanted);
            }
            "--connect" => {
                let value = args
                    .next()
                    .context("--connect requires NAME=http://HOST:PORT")?;
                connects.push(parse_connect(&value).map_err(|error| anyhow::anyhow!("{error}"))?);
            }
            // ACC-NET-145-TLS-PINNING: a given pair replaces generation;
            // both flags must appear together, checked by config validate.
            // ACC-NET-146-SECRET-INPUT: a path or a descriptor number names
            // the secret; its value never appears on the command line.
            "--tls-cert" => {
                let value = args.next().context("--tls-cert requires a path")?;
                set_flag_source(
                    &mut tls_cert,
                    "--tls-cert",
                    SecretSource::Path(PathBuf::from(value)),
                )?;
            }
            "--tls-cert-fd" => {
                let value = args.next().context("--tls-cert-fd requires a descriptor")?;
                set_flag_source(
                    &mut tls_cert,
                    "--tls-cert-fd",
                    SecretSource::Fd(descriptor("--tls-cert-fd", &value)?),
                )?;
            }
            "--tls-key" => {
                let value = args.next().context("--tls-key requires a path")?;
                set_flag_source(
                    &mut tls_key,
                    "--tls-key",
                    SecretSource::Path(PathBuf::from(value)),
                )?;
            }
            "--tls-key-fd" => {
                let value = args.next().context("--tls-key-fd requires a descriptor")?;
                set_flag_source(
                    &mut tls_key,
                    "--tls-key-fd",
                    SecretSource::Fd(descriptor("--tls-key-fd", &value)?),
                )?;
            }
            "--tls-key-passphrase-env" => {
                let name = args
                    .next()
                    .context("--tls-key-passphrase-env requires a variable name")?;
                if name.is_empty() {
                    anyhow::bail!("--tls-key-passphrase-env names no variable");
                }
                if tls_key_passphrase_env.is_some() || tls_key_passphrase_file.is_some() {
                    anyhow::bail!("the passphrase already has a source; give one form");
                }
                tls_key_passphrase_env = Some(name);
            }
            "--tls-key-passphrase-file" => {
                if tls_key_passphrase_env.is_some() {
                    anyhow::bail!("the passphrase already has a source; give one form");
                }
                let value = args
                    .next()
                    .context("--tls-key-passphrase-file requires a path")?;
                set_flag_source(
                    &mut tls_key_passphrase_file,
                    "--tls-key-passphrase-file",
                    SecretSource::Path(PathBuf::from(value)),
                )?;
            }
            "--tls-key-passphrase-fd" => {
                if tls_key_passphrase_env.is_some() {
                    anyhow::bail!("the passphrase already has a source; give one form");
                }
                let value = args
                    .next()
                    .context("--tls-key-passphrase-fd requires a descriptor")?;
                set_flag_source(
                    &mut tls_key_passphrase_file,
                    "--tls-key-passphrase-fd",
                    SecretSource::Fd(descriptor("--tls-key-passphrase-fd", &value)?),
                )?;
            }
            "--access-key-file" => {
                let value = args.next().context("--access-key-file requires a path")?;
                set_flag_source(
                    &mut access_key,
                    "--access-key-file",
                    SecretSource::Path(PathBuf::from(value)),
                )?;
            }
            "--access-key-fd" => {
                let value = args
                    .next()
                    .context("--access-key-fd requires a descriptor")?;
                set_flag_source(
                    &mut access_key,
                    "--access-key-fd",
                    SecretSource::Fd(descriptor("--access-key-fd", &value)?),
                )?;
            }
            // ACC-NET-146-SECRET-INPUT: a secret is never a flag value. The
            // value is never echoed, so a mistyped `=` form cannot put it in
            // a log or a terminal scrollback.
            "--access-key" | "--tls-key-passphrase" => {
                eprintln!(
                    "btop-web: {arg} takes no secret; use --access-key-file or --access-key-fd for the access key, and --tls-key-passphrase-env, --tls-key-passphrase-file or --tls-key-passphrase-fd for the passphrase"
                );
                std::process::exit(2);
            }
            other
                if other.starts_with("--access-key=")
                    || other.starts_with("--tls-key-passphrase=") =>
            {
                eprintln!(
                    "btop-web: a secret is never a flag value; use the file, descriptor or environment forms in the usage text"
                );
                std::process::exit(2);
            }
            "--known-agents" => {
                known_agents = Some(PathBuf::from(
                    args.next().context("--known-agents requires a path")?,
                ));
            }
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

    // ACC-CONF-148-OWN-FILE: btop.conf is the baseline, btop-web.conf carries
    // the web edits over it, and the environment outranks both.
    let web_config = web_config_path();
    let mut config = Config::load_with_web(config_path.as_deref(), web_config.as_deref())
        .context("invalid configuration")?;
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
    // ACC-NET-143-AGENT-MODES: a flag outranks the file's `mode`; `--connect`
    // replaces the file's list.
    if let Some(mode) = mode {
        config.mode = mode;
    }
    if !connects.is_empty() {
        config.connect = connects;
    }
    if tls_cert.is_some() {
        config.tls_cert = tls_cert;
    }
    if tls_key.is_some() {
        config.tls_key = tls_key;
    }
    // ACC-NET-146-SECRET-INPUT: a flag outranks the files for every secret
    // source; the value is a path, an fd number or a variable name.
    if let Some(name) = tls_key_passphrase_env {
        config.tls_key_passphrase_env = Some(name);
        config.tls_key_passphrase_file = None;
    }
    if tls_key_passphrase_file.is_some() {
        config.tls_key_passphrase_file = tls_key_passphrase_file;
        config.tls_key_passphrase_env = None;
    }
    if access_key.is_some() {
        config.access_key = access_key;
    }
    if known_agents.is_some() {
        config.known_agents = known_agents;
    }
    config.validate().context("invalid configuration")?;
    if config.mode != Mode::Client {
        config.container = collector::container::detect(&config.proc_root);
    }
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
        None if config.mode == Mode::Client => {
            tracing::info!(
                agents = config.connect.len(),
                "client: panels come from the agents"
            )
        }
        None => tracing::info!(scope = %"host", "btop collection, no container found"),
    }
    tracing::debug!(
        theme = config.theme_id(),
        filter = %config.proc_filter,
        "effective options"
    );

    let (config_sender, config_receiver) = watch::channel(config.clone());
    // ACC-NET-143-AGENT-MODES: a client samples nothing, so it runs no
    // sampler; the empty channel keeps the handlers that borrow the snapshot
    // valid, and the sender lives until the server stops.
    let (snapshot_keep, snapshot_receiver) = watch::channel(None);
    let sampler = if config.mode == Mode::Client {
        None
    } else {
        Some(collector::spawn(config_receiver))
    };
    let mut state = match &sampler {
        Some(sampler) => AppState::new(sampler.snapshots.clone(), config.readonly),
        None => AppState::new(snapshot_receiver, config.readonly),
    };
    if let Some(sampler) = &sampler {
        state.wake = Arc::clone(&sampler.wake);
    }
    state.config = config_sender;
    state.config_file = Arc::new(Mutex::new(web_config));
    state.initial_filter = config.proc_filter.clone().into();
    state.proc_view_flag = proc_view;
    if let Some(title) = title {
        state.title = title.into();
    }
    state.container = config
        .container
        .as_ref()
        .map(|container| container.tip().into());
    if config.mode == Mode::Client {
        // The panels show an agent's host; this host's name is not the
        // source and is not shown.
        state.host = None;
        // ACC-NET-145-TLS-PINNING: pins live beside the access keys; a
        // client without a writable location cannot remember an agent.
        // ACC-NET-146-SECRET-INPUT: a pin file named by --known-agents must
        // exist; a missing default file is a first connect with no pins.
        let pins = match &config.known_agents {
            Some(path) => {
                anyhow::ensure!(
                    path.exists(),
                    "pin file {} does not exist; name an existing file or drop --known-agents",
                    path.display()
                );
                path.clone()
            }
            None => btop_web::tls::pins_path().context(
                "client mode needs XDG_CONFIG_HOME or HOME to store pinned fingerprints",
            )?,
        };
        state.client = Some(Arc::new(
            client::Client::start(&config.connect, &config.connect_keys, &pins)
                .context("client access keys and pinned fingerprints")?,
        ));
    }
    if config.mode == Mode::Agent {
        // ACC-NET-144-ACCESS-KEY: the key lives in the config directory,
        // created on the first start and printed exactly once.
        // ACC-NET-146-SECRET-INPUT: --access-key names a file or descriptor
        // instead, read once and never printed; a missing or empty source
        // stops the start with the secret named.
        let key = match &config.access_key {
            Some(source) => {
                let bytes = btop_web::secret::read(source, "access key")?;
                let key = btop_web::access::key_from_bytes(&bytes)
                    .with_context(|| format!("access key from {}", source.describe()))?;
                tracing::info!(source = %source.describe(), "access key loaded");
                key
            }
            None => {
                let path = btop_web::access::key_path()
                    .context("agent mode needs XDG_CONFIG_HOME or HOME to store its access key")?;
                let (key, created) = btop_web::access::load_or_create(&path)
                    .with_context(|| format!("access key file {}", path.display()))?;
                if created {
                    // ACC-NET-144-ACCESS-KEY: the key is printed to the
                    // controlling terminal only; the log carries its path.
                    if btop_web::access::announce_created(&path, &key) {
                        tracing::info!(
                            path = %path.display(),
                            "access key created; shown once on the terminal"
                        );
                    } else {
                        tracing::info!(
                            path = %path.display(),
                            "access key created; the key is in this file only"
                        );
                    }
                } else {
                    tracing::info!(path = %path.display(), "access key loaded");
                }
                key
            }
        };
        state.access_key = Some(key.into());
    }
    let _snapshot_keep = snapshot_keep;
    let app = if config.mode == Mode::Agent {
        btop_web::routes::agent_router(state)
    } else {
        btop_web::app(state)
    };

    // ACC-NET-145-TLS-PINNING: the agent's listener speaks TLS 1.3 only. The
    // pair is generated on the first start, reused afterwards, and replaced
    // by --tls-cert/--tls-key when both are given. Standalone and client mode
    // keep the plain HTTP loopback listener exactly as before.
    let tls = if config.mode == Mode::Agent {
        let identity = btop_web::tls::identity(
            config.tls_cert.as_ref(),
            config.tls_key.as_ref(),
            config.tls_key_passphrase_env.as_deref(),
            config.tls_key_passphrase_file.as_ref(),
        )
        .context("agent TLS certificate")?;
        tracing::info!(
            cert = %identity.cert_source,
            key = %identity.key_source,
            fingerprint = %identity.fingerprint,
            created = identity.created,
            "agent TLS certificate"
        );
        Some(
            btop_web::tls::server_config(identity.certs, identity.key)
                .context("agent TLS configuration")?,
        )
    } else {
        None
    };

    let listener = TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("cannot bind {}", config.listen))?;
    tracing::info!(
        mode = config.mode.as_str(),
        listen = %config.listen,
        update_ms = config.update_ms,
        theme = config.theme_id(),
        proc_root = %config.proc_root.display(),
        readonly = config.readonly,
        "btop-web listening"
    );

    // Graceful shutdown waits for open connections; an SSE stream ends only
    // when the sampler's sender drops, so the sampler stops first.
    let task = sampler.map(|sampler| sampler.task);
    let shutdown = async move {
        shutdown_signal().await;
        if let Some(task) = task {
            task.abort();
        }
    };
    match tls {
        Some(tls) => btop_web::tls::serve(listener, app, tls, shutdown)
            .await
            .context("server failed"),
        None => axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
            .context("server failed"),
    }
}

/// ACC-NET-146-SECRET-INPUT: one form per secret on the command line; the
/// path and the descriptor forms of the same secret are exclusive.
fn set_flag_source(
    slot: &mut Option<SecretSource>,
    flag: &str,
    source: SecretSource,
) -> Result<()> {
    anyhow::ensure!(
        slot.is_none(),
        "{flag}: the same secret already has a source on the command line; give one form"
    );
    *slot = Some(source);
    Ok(())
}

/// ACC-NET-146-SECRET-INPUT: a descriptor number is a non-negative integer;
/// the refusal names the flag and never carries a secret value.
fn descriptor(flag: &str, value: &str) -> Result<RawFd> {
    let fd: RawFd = value
        .parse()
        .map_err(|_| anyhow::anyhow!("{flag} expects a descriptor number, got `{value}`"))?;
    anyhow::ensure!(
        fd >= 0,
        "{flag} expects a non-negative descriptor number, got {fd}"
    );
    Ok(fd)
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

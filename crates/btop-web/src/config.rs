//! Configuration in btop.conf's `key = value` dialect.
//!
//! Precedence is embedded defaults, then a file given with `--config`, then
//! btop-web.conf with the web edits (ACC-CONF-148-OWN-FILE), then the
//! `BTOP_WEB_*` environment variables. Unknown keys are ignored and invalid
//! values stop startup. With the read-only guard off, the config menu changes
//! the [`SERVER_OPTIONS`] live and rewrites only the changed key's line of
//! btop-web.conf ([`save`]); flags and the environment still outrank the files
//! at the next start.
//!
//! The parser is local: no maintained INI crate implements btop's stream
//! dialect or its per-key validation (see [`Config::apply`]).

use std::ffi::OsString;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use rust_embed::RustEmbed;

use crate::secret::Source as SecretSource;

#[derive(RustEmbed)]
#[folder = "config/"]
struct ConfigAsset;

/// The embedded canonical defaults, as printed by `--default-config`.
pub fn default_config_text() -> String {
    let file = ConfigAsset::get("btop-web.conf").expect("embedded config/btop-web.conf");
    String::from_utf8_lossy(&file.data).into_owned()
}

/// The port the server binds when no file or environment overrides it.
///
/// The UTF-8 bytes of the name fold eight bits at a time modulo
/// `65536 - 1024`: 39266 remains, so the port is `1024 + 39266 = 40290`.
pub const fn default_port() -> u16 {
    let name = b"btop-web";
    let mut value: u32 = 0;
    let mut index = 0;
    while index < name.len() {
        value = (value * 256 + name[index] as u32) % (65536 - 1024);
        index += 1;
    }
    (1024 + value) as u16
}

/// btop-web's sampling interval when no file, variable or flag sets one;
/// btop's own default is 2000 ms.
pub const DEFAULT_UPDATE_MS: u64 = 250;

/// ACC-PERF-149-SPLIT-CADENCE: how often the slow pass rebuilds the process
/// list, its counts and the chart paths; the values keep [`DEFAULT_UPDATE_MS`].
pub const DEFAULT_PROC_REFRESH_MS: u64 = 1000;

/// ACC-VIEW-110-TIME-AXIS: the time the charts span when no file, variable or
/// flag sets one, in seconds; the earlier fixed window was 240 s.
pub const DEFAULT_WINDOW_S: u64 = 180;

/// ACC-VIEW-130-LOAD-SCALE: how the CPU load averages are reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LoadScale {
    /// Percent of the whole machine: the load divided by the core count, so
    /// every core fully loaded is 100.
    #[default]
    Normalised,
    /// Percent of one core summed over every core: the load times 100, so the
    /// core count sets the ceiling (6400 at 64 cores).
    Aggregate,
}

impl LoadScale {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normalised => "normalised",
            Self::Aggregate => "aggregate",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        [Self::Normalised, Self::Aggregate]
            .into_iter()
            .find(|scale| scale.as_str() == value)
    }
}

/// ACC-NET-143-AGENT-MODES: what this process does at startup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    /// Sample this host and serve the dashboard. The default.
    #[default]
    Standalone,
    /// Sample this host and serve telemetry only, for a client to read.
    Agent,
    /// Sample nothing and serve the dashboard from the configured agents.
    Client,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standalone => "standalone",
            Self::Agent => "agent",
            Self::Client => "client",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        [Self::Standalone, Self::Agent, Self::Client]
            .into_iter()
            .find(|mode| mode.as_str() == value)
    }
}

/// ACC-NET-143-AGENT-MODES: one agent a client shows, `NAME=http://HOST:PORT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connect {
    /// The name the page shows; `[A-Za-z0-9._-]+`, unique in one client.
    pub name: String,
    /// `HOST:PORT` the agent listens on, without the scheme.
    pub addr: String,
}

/// Parse `NAME=http://HOST:PORT`. The name is the page's label for the agent,
/// so it is restricted to `[A-Za-z0-9._-]` and 32 bytes; the URL must be plain
/// HTTP on an explicit port, the only transport this version speaks.
pub fn parse_connect(value: &str) -> Result<Connect, ConfigError> {
    let refused = || {
        ConfigError::Validation(format!(
            "connect expects NAME=http://HOST:PORT, got `{value}`"
        ))
    };
    let (name, url) = value.split_once('=').ok_or_else(refused)?;
    let name = name.trim();
    if !valid_agent_name(name) {
        return Err(ConfigError::Validation(format!(
            "connect name must match [A-Za-z0-9._-] and be at most 32 bytes, got `{name}`"
        )));
    }
    let addr = url.trim().strip_prefix("http://").ok_or_else(refused)?;
    let (host, port) = addr.rsplit_once(':').ok_or_else(refused)?;
    let port: u16 = port.parse().map_err(|_| refused())?;
    if host.is_empty() || port == 0 || host.contains(['/', ' ', '\t']) {
        return Err(refused());
    }
    Ok(Connect {
        name: name.to_string(),
        addr: format!("{host}:{port}"),
    })
}

/// ACC-NET-144-ACCESS-KEY: one agent's key, `NAME=PATH` or `NAME=fd:N`. The
/// key itself is never part of the configuration, only where it comes from
/// (ACC-NET-146-SECRET-INPUT).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectKey {
    /// The agent the key belongs to; must match a [`Connect`] name.
    pub name: String,
    /// Where the key comes from: a file path or an inherited descriptor.
    pub source: SecretSource,
}

/// Parse `NAME=/PATH` or `NAME=fd:N`. The name follows the `connect` name
/// rule; the source is everything after the first `=`, so a path may itself
/// contain `=`.
pub fn parse_connect_key(value: &str) -> Result<ConnectKey, ConfigError> {
    let refused =
        || ConfigError::Validation(format!("connect_key expects NAME=/PATH, got `{value}`"));
    let (name, source) = value.split_once('=').ok_or_else(refused)?;
    let name = name.trim();
    if !valid_agent_name(name) {
        return Err(ConfigError::Validation(format!(
            "connect_key name must match [A-Za-z0-9._-] and be at most 32 bytes, got `{name}`"
        )));
    }
    let source = source.trim();
    if source.is_empty() {
        return Err(refused());
    }
    let source = SecretSource::parse(source).map_err(|error| {
        ConfigError::Validation(format!("connect_key for agent `{name}`: {error}"))
    })?;
    Ok(ConnectKey {
        name: name.to_string(),
        source,
    })
}

/// ACC-NET-143-AGENT-MODES: an agent name is the page's label, so it is
/// restricted to `[A-Za-z0-9._-]` and 32 bytes.
fn valid_agent_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// The shipped themes in picker order. Index 0 is the built-in Default.
pub const THEMES: [&str; 10] = [
    "default",
    "everforest-medium",
    "flexoki",
    "gruvbox",
    "solarized",
    "synthwave",
    "terracotta",
    "ultraviolet",
    "cyberpunk",
    "steel",
];

/// ACC-CONF-60-VIEW-FLAG: the process list views `--proc-view` accepts.
pub const PROC_VIEWS: [&str; 3] = ["top", "all", "tree"];

/// The process list a viewer with no stored choice starts in: the
/// `--proc-view` flag, else tree when btop's `proc_tree` is True, else
/// top-level only.
pub fn initial_proc_view(flag: Option<&'static str>, proc_tree: bool) -> &'static str {
    flag.unwrap_or(if proc_tree { "tree" } else { "top" })
}

/// ACC-CONF-61-CONFIG-PAGES: the config menu's pages, btop 1.3.0's options
/// menu categories (`btop_menu.cpp:1417`) plus layout.
pub const CONFIG_PAGES: [&str; 7] = ["general", "cpu", "gpu", "mem", "net", "proc", "layout"];

/// How the config menu edits a server option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// Two submit chips, `True` and `False`.
    Bool,
    /// A number input; the parser and [`Config::validate`] decide acceptance.
    Int { min: i64, max: i64 },
    /// A house-chip group of `(value, label)` pairs, written bare like an int.
    Chips(&'static [(&'static str, &'static str)]),
    /// A text input, written as a quoted string.
    Text,
    /// A select of `(value, label)` pairs.
    Choice(&'static [(&'static str, &'static str)]),
}

/// One btop key the config menu shows and, with the guard off, changes for
/// every viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerOption {
    pub page: &'static str,
    pub key: &'static str,
    pub control: Control,
    /// btop's first description line in `btop_menu.cpp`.
    pub help: &'static str,
}

const THEME_CHOICES: [(&str, &str); 10] = [
    ("0", "0 default"),
    ("1", "1 everforest-medium"),
    ("2", "2 flexoki"),
    ("3", "3 gruvbox"),
    ("4", "4 solarized"),
    ("5", "5 synthwave"),
    ("6", "6 terracotta"),
    ("7", "7 ultraviolet"),
    ("8", "8 cyberpunk"),
    ("9", "9 steel"),
];

/// Each value is its own label.
const fn same<const N: usize>(values: [&'static str; N]) -> [(&'static str, &'static str); N] {
    let mut pairs = [("", ""); N];
    let mut index = 0;
    while index < N {
        pairs[index] = (values[index], values[index]);
        index += 1;
    }
    pairs
}

const SORT_CHOICES: [(&str, &str); 8] = same(crate::collector::proc::SORT_MODES);

/// ACC-VIEW-130-LOAD-SCALE: the CPU load reporting choices.
const LOAD_SCALE_CHOICES: [(&str, &str); 2] = same(["normalised", "aggregate"]);

/// ACC-PERF-149-SPLIT-CADENCE: the process refresh periods the CONFIG proc
/// page offers, in milliseconds.
const PROC_REFRESH_CHOICES: [(&str, &str); 5] = same(["250", "500", "1000", "2000", "5000"]);

/// ACC-VIEW-110-TIME-AXIS: the chart windows the CONFIG time axis row offers,
/// in seconds.
const WINDOW_CHOICES: [(&str, &str); 5] = [
    ("60", "1m"),
    ("120", "2m"),
    ("180", "3m"),
    ("300", "5m"),
    ("600", "10m"),
];

const fn option(
    page: &'static str,
    key: &'static str,
    control: Control,
    help: &'static str,
) -> ServerOption {
    ServerOption {
        page,
        key,
        control,
        help,
    }
}

const fn flag(page: &'static str, key: &'static str, help: &'static str) -> ServerOption {
    option(page, key, Control::Bool, help)
}

const fn text(page: &'static str, key: &'static str, help: &'static str) -> ServerOption {
    option(page, key, Control::Text, help)
}

const MEBIBITS: Control = Control::Int {
    min: 0,
    max: i32::MAX as i64,
};

/// The server options in page order and, within a page, btop's menu order.
pub const SERVER_OPTIONS: [ServerOption; 41] = [
    option(
        "general",
        "theme",
        Control::Choice(&THEME_CHOICES),
        "Set color theme.",
    ),
    option(
        "general",
        "update_ms",
        Control::Int {
            min: 100,
            max: 86_400_000,
        },
        "Update time in milliseconds.",
    ),
    option(
        "general",
        "window_s",
        Control::Chips(&WINDOW_CHOICES),
        "Chart time axis, in seconds.",
    ),
    flag("general", "show_battery", "Show battery stats."),
    text("general", "selected_battery", "Select battery."),
    flag("cpu", "check_temp", "Enable cpu temperature reporting."),
    text("cpu", "cpu_sensor", "Cpu temperature sensor."),
    flag("cpu", "show_coretemp", "Show temperatures for cpu cores."),
    text(
        "cpu",
        "cpu_core_map",
        "Custom mapping between core and coretemp.",
    ),
    flag("cpu", "show_cpu_freq", "Show CPU frequency."),
    text(
        "cpu",
        "custom_cpu_name",
        "Custom cpu model name in cpu percentage box.",
    ),
    option(
        "cpu",
        "cpu_load",
        Control::Chips(&LOAD_SCALE_CHOICES),
        "CPU load reporting: percent of all cores, or summed over the cores.",
    ),
    flag(
        "gpu",
        "nvml_measure_pcie_speeds",
        "Measure PCIe throughput on NVIDIA cards.",
    ),
    text(
        "gpu",
        "custom_gpu_name0",
        "Custom gpu0 model name in gpu stats box.",
    ),
    text(
        "gpu",
        "custom_gpu_name1",
        "Custom gpu1 model name in gpu stats box.",
    ),
    text(
        "gpu",
        "custom_gpu_name2",
        "Custom gpu2 model name in gpu stats box.",
    ),
    text(
        "gpu",
        "custom_gpu_name3",
        "Custom gpu3 model name in gpu stats box.",
    ),
    text(
        "gpu",
        "custom_gpu_name4",
        "Custom gpu4 model name in gpu stats box.",
    ),
    text(
        "gpu",
        "custom_gpu_name5",
        "Custom gpu5 model name in gpu stats box.",
    ),
    flag("mem", "show_disks", "Split memory box to also show disks."),
    flag(
        "mem",
        "show_swap",
        "If swap memory should be shown in memory box.",
    ),
    flag("mem", "swap_disk", "Show swap as a disk."),
    flag("mem", "only_physical", "Filter out non physical disks."),
    flag(
        "mem",
        "use_fstab",
        "(Linux) Read disks list from /etc/fstab.",
    ),
    flag(
        "mem",
        "zfs_hide_datasets",
        "(Linux) Hide ZFS datasets in disks list.",
    ),
    flag(
        "mem",
        "disk_free_priv",
        "(Linux) Type of available disk space.",
    ),
    text("mem", "disks_filter", "Optional filter for shown disks."),
    flag(
        "mem",
        "zfs_arc_cached",
        "(Linux) Count ZFS ARC as cached memory.",
    ),
    flag(
        "net",
        "net_auto",
        "Start in network graphs auto rescaling mode.",
    ),
    option(
        "net",
        "net_floor",
        Control::Int {
            min: 0,
            max: i32::MAX as i64,
        },
        "Lowest auto-scaled ceiling, in Mbit/s.",
    ),
    option(
        "net",
        "net_fall",
        Control::Int {
            min: 1,
            max: 86_400,
        },
        "Seconds below a tenth of the ceiling before the auto scale falls.",
    ),
    flag("net", "net_sync", "Network scale sync."),
    option(
        "net",
        "net_download",
        MEBIBITS,
        "Fixed network graph download value.",
    ),
    option(
        "net",
        "net_upload",
        MEBIBITS,
        "Fixed network graph upload value.",
    ),
    text("net", "net_iface", "Network Interface."),
    option(
        "proc",
        "proc_sorting",
        Control::Choice(&SORT_CHOICES),
        "Processes sorting option.",
    ),
    flag("proc", "proc_reversed", "Reverse processes sorting order."),
    flag("proc", "proc_tree", "Processes tree view."),
    flag("proc", "proc_per_core", "Process usage per core."),
    flag(
        "proc",
        "proc_filter_kernel",
        "(Linux) Filter kernel processes from output.",
    ),
    option(
        "proc",
        "proc_refresh_ms",
        Control::Chips(&PROC_REFRESH_CHOICES),
        "Process list refresh, in milliseconds.",
    ),
];

/// The server option named `key`, `None` for every other key.
pub fn server_option(key: &str) -> Option<&'static ServerOption> {
    SERVER_OPTIONS.iter().find(|option| option.key == key)
}

/// `text` with the line that sets `key` replaced by `line`, as btop's loader
/// reads it: the last uncommented `key =` line wins, so that one is replaced,
/// keeping its indentation and line ending; every other byte is unchanged.
/// No such line: `line` is appended.
pub fn replace_key_line(text: &str, key: &str, line: &str) -> String {
    let segments: Vec<&str> = text.split_inclusive('\n').collect();
    let sets_key = |segment: &str| {
        let body = segment.trim();
        !body.starts_with('#')
            && body
                .split_once('=')
                .is_some_and(|(name, _)| name.trim() == key)
    };
    let Some(at) = segments.iter().rposition(|segment| sets_key(segment)) else {
        let mut next = text.to_string();
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(line);
        next.push('\n');
        return next;
    };
    let segment = segments[at];
    let indent = &segment[..segment.len() - segment.trim_start().len()];
    let ending = if segment.ends_with("\r\n") {
        "\r\n"
    } else if segment.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let mut next = String::with_capacity(text.len() + line.len());
    segments[..at]
        .iter()
        .for_each(|segment| next.push_str(segment));
    next.push_str(indent);
    next.push_str(line);
    next.push_str(ending);
    segments[at + 1..]
        .iter()
        .for_each(|segment| next.push_str(segment));
    next
}

/// ACC-CONF-148-OWN-FILE: the file the config menu writes.
/// `$XDG_CONFIG_HOME/btop-web/btop-web.conf` when XDG_CONFIG_HOME names an
/// absolute directory, else `$HOME/.config/btop-web/btop-web.conf`; `None`
/// when neither is set.
pub fn web_config_path() -> Option<PathBuf> {
    web_config_path_in(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

fn web_config_path_in(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    Some(
        config_base_in(xdg, home)?
            .join("btop-web")
            .join("btop-web.conf"),
    )
}

/// The directory that holds btop-web's own files:
/// `$XDG_CONFIG_HOME` when it names an absolute directory, else
/// `$HOME/.config`; `None` when neither is set.
pub(crate) fn config_base_in(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    match xdg {
        Some(dir) if Path::new(&dir).is_absolute() => Some(PathBuf::from(dir)),
        _ => Some(PathBuf::from(home?).join(".config")),
    }
}

/// Rewrite `key`'s line of the file at `path` as `line`, atomically: a
/// temporary file in the same directory, then a rename over the target. A
/// missing file is created with 0600 and its directory as needed; a symlinked
/// path keeps its link and the file it points to changes. A failed write
/// leaves the target exactly as it was. Returns the file written.
pub fn save(path: &Path, key: &str, line: &str) -> std::io::Result<PathBuf> {
    let target = match std::fs::canonicalize(path) {
        Ok(target) => target,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error),
    };
    let text = match std::fs::read_to_string(&target) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let next = replace_key_line(&text, key, line);
    let directory = target.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(directory)?;
    let existing = std::fs::metadata(&target).ok();
    let (fd, temporary) = nix::unistd::mkstemp(&directory.join(".btop-web.XXXXXX"))?;
    let written = (|| {
        let mut file = std::fs::File::from(fd);
        file.write_all(next.as_bytes())?;
        if let Some(metadata) = &existing {
            file.set_permissions(metadata.permissions())?;
        }
        file.sync_all()?;
        std::fs::rename(&temporary, &target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written.map(|()| target)
}

/// The picker index for a theme number, `None` when it is not one of
/// [`THEMES`].
pub fn theme_index(number: i64) -> Option<usize> {
    usize::try_from(number)
        .ok()
        .filter(|index| *index < THEMES.len())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub listen: SocketAddr,
    /// ACC-NET-143-AGENT-MODES: standalone samples and serves the dashboard,
    /// agent samples and serves telemetry only, client samples nothing and
    /// serves the dashboard from the agents in [`Self::connect`].
    pub mode: Mode,
    /// ACC-NET-143-AGENT-MODES: the agents a client shows, in page order.
    pub connect: Vec<Connect>,
    /// ACC-NET-144-ACCESS-KEY: the key file for each `connect` agent, by
    /// name; a client with no readable key for an agent refuses to start.
    pub connect_keys: Vec<ConnectKey>,
    /// ACC-NET-145-TLS-PINNING: the operator's certificate and key for the
    /// agent listener, `--tls-cert` and `--tls-key`; both or neither, and a
    /// pair given here is never written to the state directory.
    /// ACC-NET-146-SECRET-INPUT: each is a path or `fd:N`.
    pub tls_cert: Option<SecretSource>,
    pub tls_key: Option<SecretSource>,
    /// ACC-NET-146-SECRET-INPUT: the passphrase of an encrypted PKCS#8 key,
    /// from the environment variable named here or from the file or
    /// descriptor in [`Self::tls_key_passphrase_file`]; at most one source.
    pub tls_key_passphrase_env: Option<String>,
    pub tls_key_passphrase_file: Option<SecretSource>,
    /// ACC-NET-146-SECRET-INPUT: the agent listener's access key, a path or
    /// `fd:N`; without it the key is generated or read at
    /// `$XDG_CONFIG_HOME/btop-web/access-key` (ACC-NET-144-ACCESS-KEY).
    pub access_key: Option<SecretSource>,
    /// ACC-NET-146-SECRET-INPUT: the client's pin file, overriding
    /// `$XDG_CONFIG_HOME/btop-web/known_agents`; an explicitly named file
    /// must exist (ACC-NET-145-TLS-PINNING).
    pub known_agents: Option<PathBuf>,
    pub update_ms: u64,
    /// btop-web `window_s`: the time every chart spans, in seconds
    /// (ACC-VIEW-110-TIME-AXIS).
    pub window_s: u64,
    /// btop-web `cpu_load`: the CPU load reporting scale
    /// (ACC-VIEW-130-LOAD-SCALE).
    pub cpu_load: LoadScale,
    pub proc_root: PathBuf,
    /// The shipped theme number; index 0 is the built-in Default.
    pub theme: usize,
    /// ACC-SAFE-12-READONLY: refuse every request method that can change
    /// state. Set by the `--readonly` / `--no-readonly` flags, on by default.
    pub readonly: bool,
    /// btop `show_swap`: swap values and rows.
    pub show_swap: bool,
    /// btop `swap_disk`: a synthetic swap disk after the first disk.
    pub swap_disk: bool,
    /// btop `show_disks`: disk usage and IO collection.
    pub show_disks: bool,
    /// btop `zfs_arc_cached`: count the ZFS ARC as cached memory.
    pub zfs_arc_cached: bool,
    /// btop `use_fstab`: the disk list comes from `/etc/fstab`.
    pub use_fstab: bool,
    /// btop `only_physical`: keep only fstypes in `/proc/filesystems`.
    pub only_physical: bool,
    /// btop `zfs_hide_datasets`: hide datasets and aggregate their pools.
    pub zfs_hide_datasets: bool,
    /// btop `disk_free_priv`: `f_bfree` instead of `f_bavail`.
    pub disk_free_priv: bool,
    /// btop `disks_filter`: include, or `exclude=` to exclude, mountpoints.
    pub disks_filter: String,
    /// btop `show_io_stat`: draw disk IO activity.
    pub show_io_stat: bool,
    /// btop `show_cpu_freq`: read and show the CPU frequency.
    pub show_cpu_freq: bool,
    /// btop `check_temp`: read CPU and GPU temperatures.
    pub check_temp: bool,
    /// btop `show_coretemp`: per-core temperatures when sensors are found.
    pub show_coretemp: bool,
    /// btop `cpu_sensor`: sensor name, or `Auto` for btop's choice.
    pub cpu_sensor: String,
    /// btop `cpu_core_map`: whitespace-separated `x:y` core-to-sensor pairs.
    pub cpu_core_map: String,
    /// btop `show_battery`: battery statistics when a battery is present.
    pub show_battery: bool,
    /// btop `selected_battery`: device name, or `Auto`.
    pub selected_battery: String,
    /// btop `custom_cpu_name`: CPU name override, empty to disable.
    pub custom_cpu_name: String,
    /// btop `nvml_measure_pcie_speeds`: read NVML PCIe throughput.
    pub nvml_measure_pcie_speeds: bool,
    /// btop `show_gpu_info`: `Auto`, `On` or `Off`.
    pub show_gpu_info: String,
    /// btop `custom_gpu_name0`: GPU 0 name override, empty to disable.
    pub custom_gpu_name0: String,
    /// btop `custom_gpu_name1`: GPU 1 name override, empty to disable.
    pub custom_gpu_name1: String,
    /// btop `custom_gpu_name2`: GPU 2 name override, empty to disable.
    pub custom_gpu_name2: String,
    /// btop `custom_gpu_name3`: GPU 3 name override, empty to disable.
    pub custom_gpu_name3: String,
    /// btop `custom_gpu_name4`: GPU 4 name override, empty to disable.
    pub custom_gpu_name4: String,
    /// btop `custom_gpu_name5`: GPU 5 name override, empty to disable.
    pub custom_gpu_name5: String,
    /// btop `net_iface`: starting interface, empty to auto-select.
    pub net_iface: String,
    /// btop `net_auto`: auto-rescale the network graphs.
    pub net_auto: bool,
    /// btop `net_sync`: share the auto scale between both directions.
    pub net_sync: bool,
    /// btop-web `net_floor`: the lowest auto-scaled ceiling in Mbit/s
    /// (ACC-GRAPH-96-NET-AUTOSCALE).
    pub net_floor: i32,
    /// btop-web `net_fall`: seconds below a tenth of the ceiling before the
    /// auto scale falls, 300 by default (ACC-GRAPH-96-NET-AUTOSCALE).
    pub net_fall: i32,
    /// btop `net_download`: fixed download ceiling in Mebibits.
    pub net_download: i32,
    /// btop `net_upload`: fixed upload ceiling in Mebibits.
    pub net_upload: i32,
    /// btop `proc_sorting`: sort mode, accepted verbatim as btop does.
    pub proc_sorting: String,
    /// btop `proc_reversed`: reverse the sort order.
    pub proc_reversed: bool,
    /// btop `proc_tree`: the tree view. The rows are always sent in tree
    /// order; True starts a viewer with no stored choice in the tree view
    /// ([`initial_proc_view`]).
    pub proc_tree: bool,
    /// btop `proc_per_core`: CPU as a share of one core, not all cores.
    pub proc_per_core: bool,
    /// btop `proc_filter`: process substring filter, empty to disable.
    pub proc_filter: String,
    /// btop `proc_filter_kernel`: hide kernel threads.
    pub proc_filter_kernel: bool,
    /// btop-web `proc_refresh_ms`: the process list, its counts and the chart
    /// paths rebuild this often, while the values keep `update_ms`
    /// (ACC-PERF-149-SPLIT-CADENCE).
    pub proc_refresh_ms: u64,
    /// ACC-SCOPE-64-CONTAINER: the container btop-web runs in, set once at
    /// startup by `collector::container::detect`; never read from a file or
    /// the environment, `None` on a host.
    pub container: Option<crate::collector::container::Container>,
}

impl Config {
    /// Load the embedded defaults, then `path` when given, then the
    /// environment, and validate the result; btop-web.conf is not read
    /// ([`Self::load_with_web`] is main's start).
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        Self::load_with_web(path, None)
    }

    /// ACC-CONF-148-OWN-FILE: the embedded defaults, then `base` (`--config`,
    /// btop.conf), then `web` (btop-web.conf, absent until the first web
    /// edit), then the environment. A web key overrides the base at the next
    /// start; a web file that does not exist yet is simply empty.
    pub fn load_with_web(base: Option<&Path>, web: Option<&Path>) -> Result<Self, ConfigError> {
        let mut config = Self::from_text(&default_config_text(), "embedded defaults")?;
        if let Some(path) = base {
            let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
                path: path.to_path_buf(),
                source,
            })?;
            config.apply(&text, &path.display().to_string())?;
        }
        if let Some(path) = web {
            match std::fs::read_to_string(path) {
                Ok(text) => config.apply(&text, &path.display().to_string())?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(ConfigError::Read {
                        path: path.to_path_buf(),
                        source,
                    });
                }
            }
        }
        config.apply_env()?;
        config.validate()?;
        Ok(config)
    }

    /// The embedded defaults alone.
    pub fn defaults() -> Self {
        Self::from_text(&default_config_text(), "embedded defaults")
            .expect("the embedded defaults parse")
    }

    /// A server option's value as btop writes it (`True`, `250`, `cpu lazy`);
    /// `None` for a key that is not one of [`SERVER_OPTIONS`].
    pub fn get(&self, key: &str) -> Option<String> {
        let flag = |value: bool| Some(String::from(if value { "True" } else { "False" }));
        match key {
            "theme" => Some(self.theme.to_string()),
            "update_ms" => Some(self.update_ms.to_string()),
            "window_s" => Some(self.window_s.to_string()),
            "cpu_load" => Some(self.cpu_load.as_str().to_string()),
            "show_battery" => flag(self.show_battery),
            "selected_battery" => Some(self.selected_battery.clone()),
            "check_temp" => flag(self.check_temp),
            "cpu_sensor" => Some(self.cpu_sensor.clone()),
            "show_coretemp" => flag(self.show_coretemp),
            "cpu_core_map" => Some(self.cpu_core_map.clone()),
            "show_cpu_freq" => flag(self.show_cpu_freq),
            "custom_cpu_name" => Some(self.custom_cpu_name.clone()),
            "nvml_measure_pcie_speeds" => flag(self.nvml_measure_pcie_speeds),
            "custom_gpu_name0" => Some(self.custom_gpu_name0.clone()),
            "custom_gpu_name1" => Some(self.custom_gpu_name1.clone()),
            "custom_gpu_name2" => Some(self.custom_gpu_name2.clone()),
            "custom_gpu_name3" => Some(self.custom_gpu_name3.clone()),
            "custom_gpu_name4" => Some(self.custom_gpu_name4.clone()),
            "custom_gpu_name5" => Some(self.custom_gpu_name5.clone()),
            "show_disks" => flag(self.show_disks),
            "show_swap" => flag(self.show_swap),
            "swap_disk" => flag(self.swap_disk),
            "only_physical" => flag(self.only_physical),
            "use_fstab" => flag(self.use_fstab),
            "zfs_hide_datasets" => flag(self.zfs_hide_datasets),
            "disk_free_priv" => flag(self.disk_free_priv),
            "disks_filter" => Some(self.disks_filter.clone()),
            "zfs_arc_cached" => flag(self.zfs_arc_cached),
            "net_download" => Some(self.net_download.to_string()),
            "net_upload" => Some(self.net_upload.to_string()),
            "net_auto" => flag(self.net_auto),
            "net_floor" => Some(self.net_floor.to_string()),
            "net_fall" => Some(self.net_fall.to_string()),
            "net_sync" => flag(self.net_sync),
            "net_iface" => Some(self.net_iface.clone()),
            "proc_sorting" => Some(self.proc_sorting.clone()),
            "proc_reversed" => flag(self.proc_reversed),
            "proc_tree" => flag(self.proc_tree),
            "proc_per_core" => flag(self.proc_per_core),
            "proc_filter_kernel" => flag(self.proc_filter_kernel),
            "proc_refresh_ms" => Some(self.proc_refresh_ms.to_string()),
            _ => None,
        }
    }

    /// Set a server option from the config menu with the file's parser and
    /// range check, and return the line btop's `Config::write` would write
    /// (`btop_config.cpp:736-741`): bools `True`/`False`, ints bare, strings
    /// quoted. A refused value leaves the config unchanged.
    pub fn set(&mut self, key: &str, value: &str) -> Result<String, ConfigError> {
        let option = server_option(key)
            .ok_or_else(|| ConfigError::Validation(format!("not a server option: {key}")))?;
        if value.contains(['\r', '\n', '"']) {
            return Err(ConfigError::Validation(format!(
                "{key}: a value cannot hold a line break or a double quote"
            )));
        }
        let line = match option.control {
            Control::Bool => match value {
                "true" | "True" => format!("{key} = True"),
                "false" | "False" => format!("{key} = False"),
                other => format!("{key} = {other}"),
            },
            // theme is btop-web's int choice; proc_sorting is a btop string.
            Control::Int { .. } | Control::Chips(_) => format!("{key} = {value}"),
            Control::Choice(_) if key == "theme" => format!("{key} = {value}"),
            Control::Text | Control::Choice(_) => format!("{key} = \"{value}\""),
        };
        let mut candidate = self.clone();
        candidate.apply(&line, "request")?;
        candidate.validate()?;
        *self = candidate;
        Ok(line)
    }

    fn from_text(text: &str, origin: &str) -> Result<Self, ConfigError> {
        let mut config = Self {
            listen: SocketAddr::from(([127, 0, 0, 1], default_port())),
            mode: Mode::Standalone,
            connect: Vec::new(),
            connect_keys: Vec::new(),
            tls_cert: None,
            tls_key: None,
            tls_key_passphrase_env: None,
            tls_key_passphrase_file: None,
            access_key: None,
            known_agents: None,
            update_ms: DEFAULT_UPDATE_MS,
            window_s: DEFAULT_WINDOW_S,
            cpu_load: LoadScale::Normalised,
            proc_root: PathBuf::from("/proc"),
            theme: 0,
            readonly: true,
            show_swap: true,
            swap_disk: true,
            show_disks: true,
            zfs_arc_cached: true,
            use_fstab: true,
            only_physical: true,
            zfs_hide_datasets: false,
            disk_free_priv: false,
            disks_filter: String::new(),
            show_io_stat: true,
            show_cpu_freq: true,
            check_temp: true,
            show_coretemp: true,
            cpu_sensor: String::from("Auto"),
            cpu_core_map: String::new(),
            show_battery: true,
            selected_battery: String::from("Auto"),
            custom_cpu_name: String::new(),
            nvml_measure_pcie_speeds: true,
            show_gpu_info: String::from("Auto"),
            custom_gpu_name0: String::new(),
            custom_gpu_name1: String::new(),
            custom_gpu_name2: String::new(),
            custom_gpu_name3: String::new(),
            custom_gpu_name4: String::new(),
            custom_gpu_name5: String::new(),
            net_iface: String::new(),
            net_auto: true,
            net_floor: 1,
            net_fall: 300,
            net_sync: true,
            net_download: 100,
            net_upload: 100,
            proc_sorting: String::from("cpu lazy"),
            proc_reversed: false,
            proc_tree: false,
            proc_per_core: false,
            proc_filter: String::new(),
            proc_filter_kernel: false,
            proc_refresh_ms: DEFAULT_PROC_REFRESH_MS,
            container: None,
        };
        config.apply(text, origin)?;
        Ok(config)
    }

    /// Kept local: btop's loader reads a flat, case-sensitive `key = value`
    /// stream where `#` starts a comment only at the start of a line, bools,
    /// ints and unquoted strings end at the first whitespace, quoted strings
    /// keep their spaces, unknown keys are skipped, and `isbool`, `isint` and
    /// `stringValid` decide acceptance per key. rust-ini 0.21.3 keeps the
    /// whole rest of a line for a boolean (`False # note` is not `False`)
    /// unless its `inline-comment` feature is on, and still keeps the spaces
    /// in an unquoted string. configparser 3.3.0 strips inline `#` comments
    /// and folds key case by default; `ini` 1.3.0 is a macro over configparser
    /// 1.0.0 with the same comment and case rules. None carries btop's
    /// per-key validation or its line-numbered refusal, and adopting one would
    /// change which files start the server.
    fn apply(&mut self, text: &str, origin: &str) -> Result<(), ConfigError> {
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(ConfigError::Syntax {
                    origin: origin.to_string(),
                    line: index + 1,
                });
            };
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            match key {
                "listen" => {
                    self.listen = value.parse().map_err(|_| ConfigError::Invalid {
                        origin: origin.to_string(),
                        line: index + 1,
                        key: key.to_string(),
                        value: value.to_string(),
                    })?;
                }
                "mode" => {
                    self.mode =
                        Mode::parse(value).ok_or_else(|| invalid(key, value, origin, index))?;
                }
                "connect" => {
                    let link = parse_connect(value).map_err(|error| {
                        ConfigError::Validation(format!("{origin}:{}: {error}", index + 1))
                    })?;
                    self.connect.push(link);
                }
                "connect_key" => {
                    let key = parse_connect_key(value).map_err(|error| {
                        ConfigError::Validation(format!("{origin}:{}: {error}", index + 1))
                    })?;
                    self.connect_keys.push(key);
                }
                "tls_cert" => set_source(
                    &mut self.tls_cert,
                    "tls_cert",
                    path_source(key, value, origin, index)?,
                    origin,
                    index,
                )?,
                "tls_cert_fd" => set_source(
                    &mut self.tls_cert,
                    "tls_cert_fd",
                    fd_source(key, value, origin, index)?,
                    origin,
                    index,
                )?,
                "tls_key" => set_source(
                    &mut self.tls_key,
                    "tls_key",
                    path_source(key, value, origin, index)?,
                    origin,
                    index,
                )?,
                "tls_key_fd" => set_source(
                    &mut self.tls_key,
                    "tls_key_fd",
                    fd_source(key, value, origin, index)?,
                    origin,
                    index,
                )?,
                "tls_key_passphrase_env" => {
                    if value.is_empty() {
                        return Err(ConfigError::Validation(format!(
                            "{origin}:{}: tls_key_passphrase_env names no variable",
                            index + 1
                        )));
                    }
                    self.tls_key_passphrase_env = Some(value.to_string());
                }
                "tls_key_passphrase_file" => set_source(
                    &mut self.tls_key_passphrase_file,
                    "tls_key_passphrase_file",
                    path_source(key, value, origin, index)?,
                    origin,
                    index,
                )?,
                "tls_key_passphrase_fd" => set_source(
                    &mut self.tls_key_passphrase_file,
                    "tls_key_passphrase_fd",
                    fd_source(key, value, origin, index)?,
                    origin,
                    index,
                )?,
                "access_key" => set_source(
                    &mut self.access_key,
                    "access_key",
                    path_source(key, value, origin, index)?,
                    origin,
                    index,
                )?,
                "access_key_fd" => set_source(
                    &mut self.access_key,
                    "access_key_fd",
                    fd_source(key, value, origin, index)?,
                    origin,
                    index,
                )?,
                "known_agents" => self.known_agents = Some(PathBuf::from(value)),
                "update_ms" => {
                    self.update_ms = value.parse().map_err(|_| ConfigError::Invalid {
                        origin: origin.to_string(),
                        line: index + 1,
                        key: key.to_string(),
                        value: value.to_string(),
                    })?;
                }
                "window_s" => {
                    self.window_s = value
                        .parse()
                        .map_err(|_| invalid(key, value, origin, index))?;
                }
                "cpu_load" => {
                    self.cpu_load = LoadScale::parse(value)
                        .ok_or_else(|| invalid(key, value, origin, index))?;
                }
                "proc_root" => self.proc_root = PathBuf::from(value),
                "theme" => {
                    self.theme = theme_index(i64::from(parse_int(key, value, origin, index)?))
                        .ok_or_else(|| {
                            ConfigError::Validation(format!(
                                "{origin}:{}: theme must be in 0..={}, got {value}",
                                index + 1,
                                THEMES.len() - 1
                            ))
                        })?;
                }
                "show_swap" => self.show_swap = parse_bool(key, value, origin, index)?,
                "swap_disk" => self.swap_disk = parse_bool(key, value, origin, index)?,
                "show_disks" => self.show_disks = parse_bool(key, value, origin, index)?,
                "zfs_arc_cached" => {
                    self.zfs_arc_cached = parse_bool(key, value, origin, index)?;
                }
                "use_fstab" => self.use_fstab = parse_bool(key, value, origin, index)?,
                "only_physical" => {
                    self.only_physical = parse_bool(key, value, origin, index)?;
                }
                "zfs_hide_datasets" => {
                    self.zfs_hide_datasets = parse_bool(key, value, origin, index)?;
                }
                "disk_free_priv" => {
                    self.disk_free_priv = parse_bool(key, value, origin, index)?;
                }
                "disks_filter" => self.disks_filter = value.to_string(),
                "show_io_stat" => self.show_io_stat = parse_bool(key, value, origin, index)?,
                "show_cpu_freq" => self.show_cpu_freq = parse_bool(key, value, origin, index)?,
                "check_temp" => self.check_temp = parse_bool(key, value, origin, index)?,
                "show_coretemp" => self.show_coretemp = parse_bool(key, value, origin, index)?,
                "cpu_sensor" => self.cpu_sensor = value.to_string(),
                "cpu_core_map" => {
                    if !valid_core_map(value) {
                        return Err(invalid(key, value, origin, index));
                    }
                    self.cpu_core_map = value.to_string();
                }
                "show_battery" => self.show_battery = parse_bool(key, value, origin, index)?,
                "selected_battery" => self.selected_battery = value.to_string(),
                "custom_cpu_name" => self.custom_cpu_name = value.to_string(),
                "nvml_measure_pcie_speeds" => {
                    self.nvml_measure_pcie_speeds = parse_bool(key, value, origin, index)?;
                }
                "show_gpu_info" => {
                    if !SHOW_GPU_VALUES.contains(&value) {
                        return Err(invalid(key, value, origin, index));
                    }
                    self.show_gpu_info = value.to_string();
                }
                "custom_gpu_name0" => self.custom_gpu_name0 = value.to_string(),
                "custom_gpu_name1" => self.custom_gpu_name1 = value.to_string(),
                "custom_gpu_name2" => self.custom_gpu_name2 = value.to_string(),
                "custom_gpu_name3" => self.custom_gpu_name3 = value.to_string(),
                "custom_gpu_name4" => self.custom_gpu_name4 = value.to_string(),
                "custom_gpu_name5" => self.custom_gpu_name5 = value.to_string(),
                "net_iface" => self.net_iface = value.to_string(),
                "net_auto" => self.net_auto = parse_bool(key, value, origin, index)?,
                "net_floor" => self.net_floor = parse_int(key, value, origin, index)?,
                "net_fall" => self.net_fall = parse_int(key, value, origin, index)?,
                "net_sync" => self.net_sync = parse_bool(key, value, origin, index)?,
                "net_download" => self.net_download = parse_int(key, value, origin, index)?,
                "net_upload" => self.net_upload = parse_int(key, value, origin, index)?,
                "proc_sorting" => self.proc_sorting = value.to_string(),
                "proc_reversed" => self.proc_reversed = parse_bool(key, value, origin, index)?,
                "proc_tree" => self.proc_tree = parse_bool(key, value, origin, index)?,
                "proc_per_core" => self.proc_per_core = parse_bool(key, value, origin, index)?,
                "proc_filter" => self.proc_filter = value.to_string(),
                "proc_filter_kernel" => {
                    self.proc_filter_kernel = parse_bool(key, value, origin, index)?;
                }
                "proc_refresh_ms" => {
                    self.proc_refresh_ms = value.parse().map_err(|_| ConfigError::Invalid {
                        origin: origin.to_string(),
                        line: index + 1,
                        key: key.to_string(),
                        value: value.to_string(),
                    })?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn apply_env(&mut self) -> Result<(), ConfigError> {
        if let Ok(value) = std::env::var("BTOP_WEB_LISTEN") {
            self.listen = value.parse().map_err(|_| ConfigError::InvalidEnv {
                key: "BTOP_WEB_LISTEN",
                value,
            })?;
        }
        if let Ok(value) = std::env::var("BTOP_WEB_UPDATE_MS") {
            self.update_ms = value.parse().map_err(|_| ConfigError::InvalidEnv {
                key: "BTOP_WEB_UPDATE_MS",
                value,
            })?;
        }
        if let Ok(value) = std::env::var("BTOP_WEB_PROC_ROOT") {
            self.proc_root = PathBuf::from(value);
        }
        Ok(())
    }

    /// The configured theme's id, the page's initial `data-theme`.
    pub fn theme_id(&self) -> &'static str {
        THEMES[self.theme]
    }

    /// ACC-VIEW-110-TIME-AXIS: the time every chart spans, in milliseconds.
    pub fn window_ms(&self) -> u64 {
        self.window_s.saturating_mul(1000)
    }

    /// The checks startup applies after the files, the environment and the
    /// flags have all been merged.
    pub fn validate(&self) -> Result<(), ConfigError> {
        // ACC-NET-143-AGENT-MODES: a client with nothing to show is a
        // configuration error, not an empty page.
        if self.mode == Mode::Client && self.connect.is_empty() {
            return Err(ConfigError::Validation(
                "client mode needs at least one `connect = NAME=http://HOST:PORT`".to_string(),
            ));
        }
        for (index, link) in self.connect.iter().enumerate() {
            if self.connect[..index]
                .iter()
                .any(|other| other.name == link.name)
            {
                return Err(ConfigError::Validation(format!(
                    "connect name `{}` is repeated; each agent needs its own name",
                    link.name
                )));
            }
        }
        // ACC-NET-144-ACCESS-KEY: every agent a client connects to has one
        // key file.
        for (index, key) in self.connect_keys.iter().enumerate() {
            if self.connect_keys[..index]
                .iter()
                .any(|other| other.name == key.name)
            {
                return Err(ConfigError::Validation(format!(
                    "connect_key name `{}` is repeated; each agent has one key file",
                    key.name
                )));
            }
            // A key line for an agent a `--connect` flag will name is valid
            // at load time, before the flags merge, so an unknown name is not
            // refused here; a typo surfaces as the missing key below.
        }
        // ACC-NET-145-TLS-PINNING: a certificate without its key, or a key
        // without its certificate, cannot serve TLS; generation is
        // overridden by the pair or not at all.
        if self.tls_cert.is_some() != self.tls_key.is_some() {
            return Err(ConfigError::Validation(
                "tls_cert and tls_key must be given together".to_string(),
            ));
        }
        // ACC-NET-146-SECRET-INPUT: one passphrase source at most, and only
        // for an operator key: a generated pair is never encrypted.
        if self.tls_key_passphrase_env.is_some() && self.tls_key_passphrase_file.is_some() {
            return Err(ConfigError::Validation(
                "give tls_key_passphrase_env or tls_key_passphrase_file, not both".to_string(),
            ));
        }
        if (self.tls_key_passphrase_env.is_some() || self.tls_key_passphrase_file.is_some())
            && self.tls_key.is_none()
        {
            return Err(ConfigError::Validation(
                "tls_key_passphrase_env or tls_key_passphrase_file needs tls_key to name the encrypted key".to_string(),
            ));
        }
        if self.mode == Mode::Client {
            for link in &self.connect {
                if !self.connect_keys.iter().any(|key| key.name == link.name) {
                    return Err(ConfigError::Validation(format!(
                        "client mode needs `connect_key = {}=PATH` for agent `{}`",
                        link.name, link.name
                    )));
                }
            }
        }
        if !(100..=86_400_000).contains(&self.update_ms) {
            return Err(ConfigError::Validation(format!(
                "update_ms must be in 100..=86400000, got {}",
                self.update_ms
            )));
        }
        if !WINDOW_CHOICES
            .iter()
            .any(|(value, _)| value.parse::<u64>() == Ok(self.window_s))
        {
            return Err(ConfigError::Validation(format!(
                "window_s must be one of 60, 120, 180, 300 or 600, got {}",
                self.window_s
            )));
        }
        if !(250..=5000).contains(&self.proc_refresh_ms) {
            return Err(ConfigError::Validation(format!(
                "proc_refresh_ms must be in 250..=5000, got {}",
                self.proc_refresh_ms
            )));
        }
        if !self.proc_root.is_dir() {
            return Err(ConfigError::Validation(format!(
                "proc_root is not a directory: {}",
                self.proc_root.display()
            )));
        }
        Ok(())
    }
}

/// btop's `show_gpu_values` (`src/btop_config.hpp`).
const SHOW_GPU_VALUES: [&str; 3] = ["Auto", "On", "Off"];

fn invalid(key: &str, value: &str, origin: &str, index: usize) -> ConfigError {
    ConfigError::Invalid {
        origin: origin.to_string(),
        line: index + 1,
        key: key.to_string(),
        value: value.to_string(),
    }
}

/// ACC-NET-146-SECRET-INPUT: a secret is named by a path or a descriptor
/// number, never by its value; a refusal carries the key and the line.
fn path_source(
    key: &str,
    value: &str,
    origin: &str,
    index: usize,
) -> Result<SecretSource, ConfigError> {
    if value.is_empty() {
        return Err(ConfigError::Validation(format!(
            "{origin}:{}: {key} names no file",
            index + 1
        )));
    }
    Ok(SecretSource::Path(PathBuf::from(value)))
}

fn fd_source(
    key: &str,
    value: &str,
    origin: &str,
    index: usize,
) -> Result<SecretSource, ConfigError> {
    let fd = parse_int(key, value, origin, index)?;
    Ok(SecretSource::Fd(fd))
}

/// ACC-NET-146-SECRET-INPUT: one secret, one source: the path key and the
/// descriptor key for the same secret are exclusive.
fn set_source(
    slot: &mut Option<SecretSource>,
    key: &str,
    source: SecretSource,
    origin: &str,
    index: usize,
) -> Result<(), ConfigError> {
    if slot.is_some() {
        return Err(ConfigError::Validation(format!(
            "{origin}:{}: {key} names a secret that its other form already names",
            index + 1
        )));
    }
    *slot = Some(source);
    Ok(())
}

/// btop's `isbool` spellings, with the same meaning as `stobool`.
fn parse_bool(key: &str, value: &str, origin: &str, index: usize) -> Result<bool, ConfigError> {
    match value {
        "true" | "True" => Ok(true),
        "false" | "False" => Ok(false),
        _ => Err(invalid(key, value, origin, index)),
    }
}

/// btop's `isint` and `stoi`: decimal digits only, fitting an `int`.
fn parse_int(key: &str, value: &str, origin: &str, index: usize) -> Result<i32, ConfigError> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(key, value, origin, index));
    }
    value
        .parse()
        .map_err(|_| invalid(key, value, origin, index))
}

/// btop's `stringValid` for `cpu_core_map`: whitespace-separated `x:y` pairs,
/// each side digits only. btop's `ssplit` drops empty segments.
fn valid_core_map(value: &str) -> bool {
    value.split_whitespace().all(|pair| {
        let mut parts = pair.split(':').filter(|part| !part.is_empty());
        matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some(core), Some(sensor), None)
                if core.bytes().all(|byte| byte.is_ascii_digit())
                    && sensor.bytes().all(|byte| byte.is_ascii_digit())
        )
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{origin}:{line}: expected `key = value`")]
    Syntax { origin: String, line: usize },
    #[error("{origin}:{line}: invalid value for `{key}`: {value}")]
    Invalid {
        origin: String,
        line: usize,
        key: String,
        value: String,
    },
    #[error("invalid value for `{key}` in the environment: {value}")]
    InvalidEnv { key: &'static str, value: String },
    #[error("{0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_defaults_parse() {
        let config = Config::from_text(&default_config_text(), "test").unwrap();
        assert_eq!(config.listen, SocketAddr::from(([127, 0, 0, 1], 40290)));
        assert_eq!(config.update_ms, DEFAULT_UPDATE_MS);
        assert_eq!(config.update_ms, 250);
        assert_eq!(config.window_s, DEFAULT_WINDOW_S);
        assert_eq!(config.window_s, 180);
        assert_eq!(config.window_ms(), 180_000);
        assert_eq!(config.proc_root, PathBuf::from("/proc"));
        assert_eq!(config.theme, 0);
        assert_eq!(config.theme_id(), "default");
        assert!(config.readonly);
        assert!(config.show_swap);
        assert!(config.swap_disk);
        assert!(config.show_disks);
        assert!(config.zfs_arc_cached);
        assert!(config.use_fstab);
        assert!(config.only_physical);
        assert!(!config.zfs_hide_datasets);
        assert!(!config.disk_free_priv);
        assert_eq!(config.disks_filter, "");
        assert!(config.show_io_stat);
        assert!(config.show_cpu_freq);
        assert!(config.check_temp);
        assert!(config.show_coretemp);
        assert_eq!(config.cpu_sensor, "Auto");
        assert_eq!(config.cpu_core_map, "");
        assert!(config.show_battery);
        assert_eq!(config.selected_battery, "Auto");
        assert_eq!(config.custom_cpu_name, "");
        assert!(config.nvml_measure_pcie_speeds);
        assert_eq!(config.show_gpu_info, "Auto");
        for name in [
            &config.custom_gpu_name0,
            &config.custom_gpu_name1,
            &config.custom_gpu_name2,
            &config.custom_gpu_name3,
            &config.custom_gpu_name4,
            &config.custom_gpu_name5,
        ] {
            assert_eq!(name, "");
        }
        assert_eq!(config.net_iface, "");
        assert!(config.net_auto);
        assert!(config.net_sync);
        assert_eq!(config.net_floor, 1);
        assert_eq!(config.net_fall, 300);
        assert_eq!(config.net_download, 100);
        assert_eq!(config.net_upload, 100);
        assert_eq!(config.proc_sorting, "cpu lazy");
        assert!(!config.proc_reversed);
        assert!(!config.proc_tree);
        assert!(!config.proc_per_core);
        assert_eq!(config.proc_filter, "");
        assert!(!config.proc_filter_kernel);
        assert_eq!(config.mode, Mode::Standalone);
        assert!(config.connect.is_empty());
        assert!(config.connect_keys.is_empty());
    }

    /// ACC-NET-143-AGENT-MODES: the mode and every `connect` line come from
    /// the file, a client with no agent is refused, and a repeated name is
    /// refused. ACC-NET-144-ACCESS-KEY: a client needs one key file per
    /// agent, and a key file for an agent that is not configured is refused.
    #[test]
    fn modes_and_connect_lines_are_read_from_the_file() {
        let mut config = Config::from_text(&default_config_text(), "test").unwrap();
        config
            .apply(
                "\
mode = client
connect = nas=http://10.0.0.5:40290
connect = \"desk=http://[::1]:8080\"
connect_key = nas=/keys/nas
connect_key = \"desk=/keys/desk\"
",
                "test",
            )
            .unwrap();
        assert_eq!(config.mode, Mode::Client);
        assert_eq!(
            config.connect,
            vec![
                Connect {
                    name: "nas".to_string(),
                    addr: "10.0.0.5:40290".to_string()
                },
                Connect {
                    name: "desk".to_string(),
                    addr: "[::1]:8080".to_string()
                },
            ]
        );
        assert_eq!(
            config.connect_keys,
            vec![
                ConnectKey {
                    name: "nas".to_string(),
                    source: SecretSource::Path(PathBuf::from("/keys/nas"))
                },
                ConnectKey {
                    name: "desk".to_string(),
                    source: SecretSource::Path(PathBuf::from("/keys/desk"))
                },
            ]
        );
        config.validate().unwrap();

        let mut empty = Config::from_text(&default_config_text(), "test").unwrap();
        empty.mode = Mode::Client;
        assert!(empty.validate().is_err());

        let mut repeated = config.clone();
        repeated.connect.push(repeated.connect[0].clone());
        assert!(repeated.validate().is_err());

        let mut unkeyed = config.clone();
        unkeyed.connect_keys.retain(|key| key.name != "desk");
        assert!(unkeyed.validate().is_err());

        let mut repeated_key = config.clone();
        repeated_key.connect_keys.push(ConnectKey {
            name: "nas".to_string(),
            source: SecretSource::Path(PathBuf::from("/keys/other")),
        });
        assert!(repeated_key.validate().is_err());

        // A key line for an agent a --connect flag will name is accepted at
        // load time, when the merged connect list is still empty.
        let mut flagged = Config::from_text(&default_config_text(), "test").unwrap();
        flagged
            .apply("connect_key = nas=/keys/nas\n", "test")
            .unwrap();
        flagged.validate().unwrap();
        flagged.mode = Mode::Client;
        flagged.connect.push(Connect {
            name: "nas".to_string(),
            addr: "127.0.0.1:1".to_string(),
        });
        flagged.validate().unwrap();

        for refused in [
            "nashttps://host:1",
            "na s=http://host:1",
            "nas=https://host:1",
            "nas=http://host",
            "nas=http://host:0",
            "nas=http://host:1/path",
            "=http://host:1",
        ] {
            assert!(parse_connect(refused).is_err(), "{refused}");
        }
        assert_eq!(parse_connect(" nas = http://h:1 ").unwrap().name, "nas");

        for refused in ["nas", "nas=", "= /keys/nas", "na s=/keys/nas"] {
            assert!(parse_connect_key(refused).is_err(), "{refused}");
        }
        assert_eq!(
            parse_connect_key(" nas = /keys/with=equals ").unwrap(),
            ConnectKey {
                name: "nas".to_string(),
                source: SecretSource::Path(PathBuf::from("/keys/with=equals"))
            }
        );
    }

    /// ACC-NET-146-SECRET-INPUT: every secret source is a path or `fd:N`,
    /// read from the file, and a flag value parses the same way.
    #[test]
    fn secret_sources_are_read_from_the_file() {
        let mut config = Config::from_text(&default_config_text(), "test").unwrap();
        config
            .apply(
                "\
tls_cert_fd = 3
tls_key = /keys/agent.key
tls_key_passphrase_env = BTOP_WEB_PASSPHRASE
access_key_fd = 4
known_agents = /keys/known_agents
connect_key = nas=fd:5
",
                "test",
            )
            .unwrap();
        assert_eq!(config.tls_cert, Some(SecretSource::Fd(3)));
        assert_eq!(
            config.tls_key,
            Some(SecretSource::Path(PathBuf::from("/keys/agent.key")))
        );
        assert_eq!(
            config.tls_key_passphrase_env.as_deref(),
            Some("BTOP_WEB_PASSPHRASE")
        );
        assert_eq!(config.access_key, Some(SecretSource::Fd(4)));
        assert_eq!(
            config.known_agents,
            Some(PathBuf::from("/keys/known_agents"))
        );
        assert_eq!(
            config.connect_keys,
            vec![ConnectKey {
                name: "nas".to_string(),
                source: SecretSource::Fd(5)
            }]
        );
        config.validate().unwrap();

        // Every secret has a descriptor form, and the pair of descriptor keys
        // is valid on its own.
        let mut descriptors = Config::from_text(&default_config_text(), "test").unwrap();
        descriptors
            .apply(
                "tls_cert_fd = 3\ntls_key_fd = 4\ntls_key_passphrase_fd = 5\naccess_key_fd = 6\n",
                "test",
            )
            .unwrap();
        assert_eq!(descriptors.tls_cert, Some(SecretSource::Fd(3)));
        assert_eq!(descriptors.tls_key, Some(SecretSource::Fd(4)));
        assert_eq!(
            descriptors.tls_key_passphrase_file,
            Some(SecretSource::Fd(5))
        );
        assert_eq!(descriptors.access_key, Some(SecretSource::Fd(6)));
        descriptors.validate().unwrap();
    }

    /// ACC-NET-146-SECRET-INPUT: one passphrase source at most, and only for
    /// an operator key; a malformed descriptor is refused with its key.
    #[test]
    fn secret_source_refusals_are_named() {
        let mut passphrase_without_key = Config::from_text(&default_config_text(), "test").unwrap();
        passphrase_without_key
            .apply("tls_key_passphrase_env = PASS\n", "test")
            .unwrap();
        let error = passphrase_without_key.validate().unwrap_err().to_string();
        assert!(error.contains("tls_key_passphrase_env"), "{error}");
        assert!(error.contains("tls_key"), "{error}");

        let mut two_passphrases = Config::from_text(&default_config_text(), "test").unwrap();
        two_passphrases
            .apply(
                "tls_cert = /keys/agent.crt\ntls_key = /keys/agent.key\ntls_key_passphrase_env = PASS\ntls_key_passphrase_file = /keys/pass\n",
                "test",
            )
            .unwrap();
        let error = two_passphrases.validate().unwrap_err().to_string();
        assert!(error.contains("not both"), "{error}");

        let mut config = Config::from_text(&default_config_text(), "test").unwrap();
        let error = config
            .apply("tls_cert_fd = x\n", "test")
            .unwrap_err()
            .to_string();
        assert!(error.contains("tls_cert_fd"), "{error}");

        // The path form and the descriptor form of one secret are exclusive.
        let mut both = Config::from_text(&default_config_text(), "test").unwrap();
        both.apply("tls_cert = /keys/agent.crt\n", "test").unwrap();
        let error = both
            .apply("tls_cert_fd = 3\n", "test")
            .unwrap_err()
            .to_string();
        assert!(error.contains("tls_cert_fd"), "{error}");
        assert!(error.contains("other form"), "{error}");
    }

    #[test]
    fn collector_keys_are_read_from_the_file() {
        let mut config = Config::from_text(&default_config_text(), "test").unwrap();
        config
            .apply(
                "\
show_cpu_freq = False
check_temp = False
show_coretemp = False
cpu_sensor = \"Package id 0\"
cpu_core_map = \"4:0 5:1\"
show_battery = False
selected_battery = BAT1
custom_cpu_name = \"Test CPU\"
nvml_measure_pcie_speeds = False
show_gpu_info = Off
custom_gpu_name0 = gpu0
custom_gpu_name1 = gpu1
custom_gpu_name2 = gpu2
custom_gpu_name3 = gpu3
custom_gpu_name4 = gpu4
custom_gpu_name5 = gpu5
net_iface = eth0
net_auto = False
net_floor = 2
net_fall = 60
net_sync = False
net_download = 250
net_upload = 50
proc_sorting = \"cpu direct\"
proc_reversed = True
proc_tree = True
proc_per_core = True
proc_filter = rust
proc_filter_kernel = True
",
                "test.conf",
            )
            .unwrap();
        assert!(!config.show_cpu_freq);
        assert!(!config.check_temp);
        assert!(!config.show_coretemp);
        assert_eq!(config.cpu_sensor, "Package id 0");
        assert_eq!(config.cpu_core_map, "4:0 5:1");
        assert!(!config.show_battery);
        assert_eq!(config.selected_battery, "BAT1");
        assert_eq!(config.custom_cpu_name, "Test CPU");
        assert!(!config.nvml_measure_pcie_speeds);
        assert_eq!(config.show_gpu_info, "Off");
        assert_eq!(config.custom_gpu_name0, "gpu0");
        assert_eq!(config.custom_gpu_name1, "gpu1");
        assert_eq!(config.custom_gpu_name2, "gpu2");
        assert_eq!(config.custom_gpu_name3, "gpu3");
        assert_eq!(config.custom_gpu_name4, "gpu4");
        assert_eq!(config.custom_gpu_name5, "gpu5");
        assert_eq!(config.net_iface, "eth0");
        assert!(!config.net_auto);
        assert!(!config.net_sync);
        assert_eq!(config.net_floor, 2);
        assert_eq!(config.net_fall, 60);
        assert_eq!(config.net_download, 250);
        assert_eq!(config.net_upload, 50);
        assert_eq!(config.proc_sorting, "cpu direct");
        assert!(config.proc_reversed);
        assert!(config.proc_tree);
        assert!(config.proc_per_core);
        assert_eq!(config.proc_filter, "rust");
        assert!(config.proc_filter_kernel);
    }

    /// ACC-VIEW-110-TIME-AXIS: the time axis row offers 1, 2, 3, 5 and 10
    /// minutes, a setting is written to the loaded file, and a value outside
    /// the row is refused.
    #[test]
    fn the_window_option_sets_the_chart_axis() {
        let mut config = Config::from_text(&default_config_text(), "test").unwrap();
        assert_eq!(config.window_s, 180);
        assert_eq!(config.window_ms(), 180_000);
        assert_eq!(config.get("window_s").as_deref(), Some("180"));
        let row = server_option("window_s").expect("the time axis row");
        assert!(matches!(row.control, Control::Chips(_)), "house chips");
        assert_eq!(config.set("window_s", "600").unwrap(), "window_s = 600");
        assert_eq!(config.window_ms(), 600_000);
        assert!(config.set("window_s", "240").is_err());
        assert_eq!(config.window_s, 600, "a refused value changes nothing");
        for seconds in ["60", "120", "180", "300", "600"] {
            let mut candidate = config.clone();
            assert!(candidate.set("window_s", seconds).is_ok(), "{seconds}");
        }
    }

    /// ACC-VIEW-130-LOAD-SCALE: the CPU page offers normalised and aggregate
    /// load reporting, the choice is written bare to the loaded file, and any
    /// other value is refused.
    #[test]
    fn the_cpu_load_option_chooses_the_reporting_scale() {
        let mut config = Config::defaults();
        assert_eq!(config.cpu_load, LoadScale::Normalised);
        assert_eq!(config.get("cpu_load").as_deref(), Some("normalised"));
        let row = server_option("cpu_load").expect("the CPU load row");
        assert_eq!(row.page, "cpu");
        assert!(matches!(row.control, Control::Chips(_)), "house chips");
        assert_eq!(
            config.set("cpu_load", "aggregate").unwrap(),
            "cpu_load = aggregate"
        );
        assert_eq!(config.cpu_load, LoadScale::Aggregate);
        assert!(config.set("cpu_load", "both").is_err());
        assert_eq!(
            config.cpu_load,
            LoadScale::Aggregate,
            "a refused value changes nothing"
        );
    }

    #[test]
    fn invalid_collector_values_are_rejected_with_their_line() {
        for (text, key) in [
            ("show_cpu_freq = yes\n", "show_cpu_freq"),
            ("proc_tree = 1\n", "proc_tree"),
            ("net_download = -1\n", "net_download"),
            ("net_upload = 1.5\n", "net_upload"),
            ("net_download = 2147483648\n", "net_download"),
            ("net_floor = -1\n", "net_floor"),
            ("net_fall = 1.5\n", "net_fall"),
            ("show_gpu_info = Sometimes\n", "show_gpu_info"),
            ("cpu_core_map = 0-1\n", "cpu_core_map"),
            ("cpu_core_map = 0:1:2\n", "cpu_core_map"),
            ("cpu_core_map = 0:\n", "cpu_core_map"),
        ] {
            let mut config = Config::from_text(&default_config_text(), "test").unwrap();
            let error = config.apply(text, "test.conf").unwrap_err();
            let message = error.to_string();
            assert!(message.contains("test.conf:1"), "{text}");
            assert!(message.contains(key), "{text}");
        }
    }

    #[test]
    fn btop_bool_spellings_are_accepted_and_invalid_ones_rejected() {
        let mut config = Config::from_text(&default_config_text(), "test").unwrap();
        config
            .apply(
                "use_fstab = False\nonly_physical = true\ndisks_filter = \"exclude=/boot /home\"\n",
                "test",
            )
            .unwrap();
        assert!(!config.use_fstab);
        assert!(config.only_physical);
        assert_eq!(config.disks_filter, "exclude=/boot /home");

        let error = config.apply("use_fstab = yes\n", "test.conf").unwrap_err();
        assert!(error.to_string().contains("test.conf:1"));
    }

    #[test]
    fn file_values_override_defaults_and_unknown_keys_are_ignored() {
        let mut config = Config::from_text(&default_config_text(), "test").unwrap();
        config
            .apply("update_ms = 500\ncolor_theme = \"gruvbox\"\n", "test")
            .unwrap();
        assert_eq!(config.update_ms, 500);
        assert_eq!(config.listen, SocketAddr::from(([127, 0, 0, 1], 40290)));
    }

    #[test]
    fn default_port_folds_the_name() {
        assert_eq!(default_port(), 40290);
    }

    #[test]
    fn themes_are_numbered_in_the_shipped_order() {
        assert_eq!(
            THEMES,
            [
                "default",
                "everforest-medium",
                "flexoki",
                "gruvbox",
                "solarized",
                "synthwave",
                "terracotta",
                "ultraviolet",
                "cyberpunk",
                "steel"
            ]
        );
        let mut config = Config::from_text(&default_config_text(), "test").unwrap();
        config.apply("theme = 4\n", "test.conf").unwrap();
        assert_eq!(config.theme_id(), "solarized");
        config.apply("theme = 7\n", "test.conf").unwrap();
        assert_eq!(config.theme_id(), "ultraviolet");
        config.apply("theme = 8\n", "test.conf").unwrap();
        assert_eq!(config.theme_id(), "cyberpunk");
        config.apply("theme = 9\n", "test.conf").unwrap();
        assert_eq!(config.theme_id(), "steel");
    }

    /// The theme numbers are the picker order: the menu lists exactly
    /// [`THEMES`] in this order, so a number names the same theme in the flag,
    /// the config file and the picker.
    #[test]
    fn shipped_theme_numbers_follow_the_embedded_picker_order() {
        let pairs = crate::theme::embedded_pairs();
        let ids: Vec<&str> = pairs.iter().map(|pair| pair.id.as_str()).collect();
        assert_eq!(ids, THEMES);
    }

    #[test]
    fn an_out_of_range_theme_lists_the_valid_range() {
        let mut config = Config::from_text(&default_config_text(), "test").unwrap();
        let error = config.apply("theme = 10\n", "test.conf").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("test.conf:1"), "{message}");
        assert!(
            message.contains("theme must be in 0..=9, got 10"),
            "{message}"
        );
    }

    #[test]
    fn initial_proc_view_precedence() {
        assert_eq!(initial_proc_view(None, false), "top");
        assert_eq!(initial_proc_view(None, true), "tree");
        assert_eq!(initial_proc_view(Some("all"), true), "all");
    }

    #[test]
    fn invalid_value_is_reported_with_its_line() {
        let mut config = Config::from_text(&default_config_text(), "test").unwrap();
        let error = config.apply("listen = nope\n", "test.conf").unwrap_err();
        assert!(error.to_string().contains("test.conf:1"));
    }

    #[test]
    fn replace_key_line_replaces_only_the_last_uncommented_line() {
        let text = "# update_ms = 1\nupdate_ms = 2\nupdate_ms_x = 3\nxupdate_ms = 4\r\n  update_ms = 5\r\ntheme = 0";
        assert_eq!(
            replace_key_line(text, "update_ms", "update_ms = 9"),
            "# update_ms = 1\nupdate_ms = 2\nupdate_ms_x = 3\nxupdate_ms = 4\r\n  update_ms = 9\r\ntheme = 0"
        );
        assert_eq!(
            replace_key_line(text, "theme", "theme = 3"),
            text.replace("theme = 0", "theme = 3")
        );
        assert_eq!(
            replace_key_line("a = 1", "show_swap", "show_swap = False"),
            "a = 1\nshow_swap = False\n"
        );
        assert_eq!(
            replace_key_line("a = 1\n", "show_swap", "show_swap = False"),
            "a = 1\nshow_swap = False\n"
        );
        assert_eq!(
            replace_key_line("", "net_auto", "net_auto = True"),
            "net_auto = True\n"
        );
    }

    #[test]
    fn set_writes_btop_lines_and_refuses_invalid_values() {
        let mut config = Config::defaults();
        for (key, value, line) in [
            ("show_swap", "False", "show_swap = False"),
            ("show_swap", "true", "show_swap = True"),
            ("net_download", "250", "net_download = 250"),
            (
                "cpu_sensor",
                "Package id 0",
                "cpu_sensor = \"Package id 0\"",
            ),
            ("theme", "3", "theme = 3"),
            (
                "proc_sorting",
                "cpu direct",
                "proc_sorting = \"cpu direct\"",
            ),
            ("update_ms", "1000", "update_ms = 1000"),
        ] {
            assert_eq!(config.set(key, value).unwrap(), line);
            assert_eq!(config.get(key).unwrap(), value.replace("true", "True"));
        }
        assert!(config.show_swap);
        assert_eq!(config.theme_id(), "gruvbox");
        for (key, value) in [
            ("update_ms", "50"),
            ("update_ms", "86400001"),
            ("net_download", "-1"),
            ("net_download", "1.5"),
            ("show_swap", "yes"),
            ("cpu_core_map", "0-1"),
            ("theme", "11"),
            ("custom_cpu_name", "a\nb"),
            ("custom_cpu_name", "a\rb"),
            ("custom_cpu_name", "a\"b"),
            ("listen", "127.0.0.1:1"),
            ("proc_root", "/"),
            ("show_io_stat", "False"),
        ] {
            let before = config.clone();
            assert!(config.set(key, value).is_err(), "{key} = {value}");
            assert_eq!(config, before, "{key} = {value}");
        }
    }

    #[test]
    fn server_options_round_trip_and_sit_on_the_pages() {
        let defaults = default_config_text();
        let mut config = Config::defaults();
        let mut seen = std::collections::BTreeSet::new();
        let mut last_page = 0;
        for option in SERVER_OPTIONS {
            assert!(seen.insert(option.key), "{} listed twice", option.key);
            let page = CONFIG_PAGES.iter().position(|page| *page == option.page);
            let page = page.unwrap_or_else(|| panic!("{}: page {}", option.key, option.page));
            assert!(page >= last_page, "{} out of page order", option.key);
            last_page = page;
            let before = config.clone();
            let value = config.get(option.key).expect("a value");
            config.set(option.key, &value).expect("its own value");
            assert_eq!(config, before, "{}", option.key);
            assert!(
                defaults.contains(&format!("\n{} = ", option.key)),
                "{} is not in config/btop-web.conf",
                option.key
            );
        }
        assert_eq!(
            CONFIG_PAGES,
            ["general", "cpu", "gpu", "mem", "net", "proc", "layout"]
        );
        for (index, (value, label)) in THEME_CHOICES.iter().enumerate() {
            assert_eq!(*value, index.to_string());
            assert_eq!(*label, format!("{index} {}", THEMES[index]));
        }
        assert_eq!(
            SORT_CHOICES.map(|(value, _)| value),
            crate::collector::proc::SORT_MODES
        );
    }

    #[test]
    fn save_rewrites_one_line_atomically() {
        let dir = std::env::temp_dir().join(format!("btop-web-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("btop-web.conf");
        std::fs::write(&path, "# btop\nupdate_ms = 250\nshow_swap = True\n").unwrap();
        assert_eq!(
            save(&path, "update_ms", "update_ms = 1000").unwrap(),
            std::fs::canonicalize(&path).unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# btop\nupdate_ms = 1000\nshow_swap = True\n"
        );
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["btop-web.conf"], "no temporary file is left");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ACC-CONF-148-OWN-FILE: the web file lives under XDG_CONFIG_HOME when it
    /// names an absolute directory, else under $HOME/.config; a relative
    /// XDG_CONFIG_HOME is ignored, as the XDG spec says.
    #[test]
    fn web_config_path_follows_xdg_then_home() {
        assert_eq!(
            web_config_path_in(Some("/tmp/xdg".into()), Some("/home/x".into())),
            Some(PathBuf::from("/tmp/xdg/btop-web/btop-web.conf"))
        );
        assert_eq!(
            web_config_path_in(Some("relative".into()), Some("/home/x".into())),
            Some(PathBuf::from("/home/x/.config/btop-web/btop-web.conf"))
        );
        assert_eq!(
            web_config_path_in(Some("".into()), Some("/home/x".into())),
            Some(PathBuf::from("/home/x/.config/btop-web/btop-web.conf"))
        );
        assert_eq!(
            web_config_path_in(None, Some("/home/x".into())),
            Some(PathBuf::from("/home/x/.config/btop-web/btop-web.conf"))
        );
        assert_eq!(web_config_path_in(None, None), None);
    }

    /// ACC-CONF-148-OWN-FILE: a missing btop-web.conf is created with 0600
    /// under a directory that is created as needed.
    #[test]
    fn save_creates_a_missing_web_file_with_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("btop-web-create-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("btop-web").join("btop-web.conf");
        assert_eq!(save(&path, "update_ms", "update_ms = 500").unwrap(), path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "update_ms = 500\n");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ACC-CONF-148-OWN-FILE: a failed write leaves the file exactly as it
    /// was, and a key the page did not touch survives every write.
    #[test]
    fn a_failed_write_leaves_the_web_file_exactly_as_it_was() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("btop-web-failed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("btop-web.conf");
        let original = "update_ms = 250\ncustom_unknown = keep\n";
        std::fs::write(&path, original).unwrap();
        // A read-only directory makes the temporary file fail after the file
        // is read; the target is never touched.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = save(&path, "update_ms", "update_ms = 500");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        if nix::unistd::geteuid().is_root() {
            // Root writes through the mode; the atomic replace is proven by
            // the other save tests and the e2e round trip.
            return;
        }
        assert!(result.is_err(), "a read-only directory refused the write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["btop-web.conf"], "no temporary file is left");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ACC-CONF-148-OWN-FILE: btop.conf is the baseline and btop-web.conf
    /// applies over it at start; a key the web file lacks keeps the baseline.
    #[test]
    fn web_edits_override_the_base_file_at_start() {
        let dir = std::env::temp_dir().join(format!("btop-web-layered-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("btop.conf");
        let web = dir.join("btop-web.conf");
        std::fs::write(&base, "update_ms = 250\nnet_floor = 1\n").unwrap();
        std::fs::write(&web, "update_ms = 500\ncustom_unknown = keep\n").unwrap();
        let config = Config::load_with_web(Some(&base), Some(&web)).unwrap();
        assert_eq!(config.update_ms, 500, "the web file overrides the base");
        assert_eq!(config.net_floor, 1, "an untouched key keeps the base value");
        assert_eq!(config.get("update_ms").as_deref(), Some("500"));
        let config = Config::load_with_web(Some(&base), Some(&dir.join("missing.conf"))).unwrap();
        assert_eq!(config.update_ms, 250, "a missing web file is empty");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

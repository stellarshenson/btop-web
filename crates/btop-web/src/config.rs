//! Configuration in btop.conf's `key = value` dialect.
//!
//! Precedence is embedded defaults, then a file given with `--config`, then
//! `BTOP_WEB_*` environment variables. Unknown keys are ignored and invalid
//! values stop startup. With the read-only guard off, the config menu changes
//! the [`SERVER_OPTIONS`] live and rewrites only the changed key's line of the
//! loaded file ([`save`]); flags and the environment still outrank the file at
//! the next start.
//!
//! The parser is local: no maintained INI crate implements btop's stream
//! dialect or its per-key validation (see [`Config::apply`]).

use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use rust_embed::RustEmbed;

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

/// The shipped themes in picker order. Index 0 is the built-in Default.
pub const THEMES: [&str; 9] = [
    "default",
    "everforest-medium",
    "flexoki",
    "gruvbox",
    "solarized",
    "synthwave",
    "terracotta",
    "ultraviolet",
    "cyberpunk",
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

const THEME_CHOICES: [(&str, &str); 9] = [
    ("0", "0 default"),
    ("1", "1 everforest-medium"),
    ("2", "2 flexoki"),
    ("3", "3 gruvbox"),
    ("4", "4 solarized"),
    ("5", "5 synthwave"),
    ("6", "6 terracotta"),
    ("7", "7 ultraviolet"),
    ("8", "8 cyberpunk"),
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
pub const SERVER_OPTIONS: [ServerOption; 38] = [
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

/// Rewrite `key`'s line of the file at `path` as `line`, atomically: a
/// temporary file in the same directory, then a rename over the target. A
/// symlinked path keeps its link and the file it points to changes. Returns
/// the file written.
pub fn save(path: &Path, key: &str, line: &str) -> std::io::Result<PathBuf> {
    let target = std::fs::canonicalize(path)?;
    let next = replace_key_line(&std::fs::read_to_string(&target)?, key, line);
    let directory = target.parent().unwrap_or(Path::new("/"));
    let (fd, temporary) = nix::unistd::mkstemp(&directory.join(".btop-web.XXXXXX"))?;
    let written = (|| {
        let mut file = std::fs::File::from(fd);
        file.write_all(next.as_bytes())?;
        file.set_permissions(std::fs::metadata(&target)?.permissions())?;
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
    pub update_ms: u64,
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
    /// ACC-SCOPE-64-CONTAINER: the container btop-web runs in, set once at
    /// startup by `collector::container::detect`; never read from a file or
    /// the environment, `None` on a host.
    pub container: Option<crate::collector::container::Container>,
}

impl Config {
    /// Load the embedded defaults, then `path` when given, then the
    /// environment, and validate the result.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut config = Self::from_text(&default_config_text(), "embedded defaults")?;
        if let Some(path) = path {
            let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
                path: path.to_path_buf(),
                source,
            })?;
            config.apply(&text, &path.display().to_string())?;
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
            Control::Int { .. } => format!("{key} = {value}"),
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
            update_ms: DEFAULT_UPDATE_MS,
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
                "update_ms" => {
                    self.update_ms = value.parse().map_err(|_| ConfigError::Invalid {
                        origin: origin.to_string(),
                        line: index + 1,
                        key: key.to_string(),
                        value: value.to_string(),
                    })?;
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

    fn validate(&self) -> Result<(), ConfigError> {
        if !(100..=86_400_000).contains(&self.update_ms) {
            return Err(ConfigError::Validation(format!(
                "update_ms must be in 100..=86400000, got {}",
                self.update_ms
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
                "cyberpunk"
            ]
        );
        let mut config = Config::from_text(&default_config_text(), "test").unwrap();
        config.apply("theme = 4\n", "test.conf").unwrap();
        assert_eq!(config.theme_id(), "solarized");
        config.apply("theme = 7\n", "test.conf").unwrap();
        assert_eq!(config.theme_id(), "ultraviolet");
        config.apply("theme = 8\n", "test.conf").unwrap();
        assert_eq!(config.theme_id(), "cyberpunk");
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
        let error = config.apply("theme = 9\n", "test.conf").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("test.conf:1"), "{message}");
        assert!(
            message.contains("theme must be in 0..=8, got 9"),
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
            ("theme", "9"),
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
        let path = dir.join("btop.conf");
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
        assert_eq!(names, ["btop.conf"], "no temporary file is left");
        assert!(save(&dir.join("missing.conf"), "update_ms", "update_ms = 1").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

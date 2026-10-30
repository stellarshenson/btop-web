//! Snapshot types, the sampling scheduler and the per-metric collectors.
//!
//! One sampler runs at `update_ms` and publishes a [`Frame`] through a
//! `tokio::watch` channel; every SSE client shares that value, so viewers do
//! not multiply `/proc` reads or template renders.
//!
//! ACC-PERF-149-SPLIT-CADENCE: one sampler, two rates. Every tick runs the
//! fast collectors (CPU, memory, network, load) and emits their values; a slow
//! pass on the [`Config::proc_refresh_ms`] boundary rebuilds the process walk,
//! the process counts and the chart paths, and every frame until the next slow
//! pass reuses those results unchanged. The fast collectors run as before, so
//! the slow pass changes when a value is recomputed, not how. A viewer that
//! connects between slow passes gets the held slow result in its first frame
//! (ACC-PERF-137-WAKE-BURST).
//!
//! ACC-PERF-136-SLEEP: with no SSE stream open the sampler parks on [`Wake`],
//! holding no timer and reading no source, until `/` or `/events` wakes it for
//! an immediate tick. ACC-PERF-137-WAKE-BURST: a subscriber receives the held
//! snapshot at once, and after a sleep it waits at most one interval for the
//! post-wake sample so the first frame carries current values.
//! ACC-PERF-142-WAKE-FAST: the first sample after a sleep runs the fast
//! collectors only and reuses the held walk and charts, so waking repeats no
//! known work; its GPU sweep starts with the wake and lands in a later frame.
//!
//! Each collector mirrors the named btop function in
//! `src/linux/btop_collect.cpp`: the same raw file, formula, units and delta
//! window. A value the host cannot supply stays `None`, which renders as
//! unavailable, never as zero.
//!
//! Byte sizes and rates use [`human`], which is local: no maintained crate
//! reproduces btop 1.3.0's `floating_humanizer` output byte for byte.
//!
//! Tests point `BTOP_WEB_PROC_ROOT` at a recorded tree; sysfs is then read from
//! the sibling `sys` directory, so `tests/fixtures/proc` pairs with
//! `tests/fixtures/sys`. Live runs use `/proc` and `/sys`.
//!
//! Inside a container ([`container`], ACC-SCOPE-64-CONTAINER) each collector
//! reads the container's cgroup and namespaces instead of the host-wide files,
//! departing from btop parity on the owner's order; a metric with no
//! container-scoped source stays `None`. `Snapshot::container` tells the
//! templates which panels are withheld.

pub mod container;
pub mod cpu;
pub mod frame;
pub mod gpu;
pub mod mem;
pub mod net;
pub mod proc;

pub use frame::Frame;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::chart::{ChartPath, ScaleFrame};
use crate::config::{Config, LoadScale};

/// Everything the dashboard renders for one sample.
///
/// The server-rendered PCHIP paths ride on the panel's own snapshot - the CPU
/// history on `cpu.chart`, the memory history on `mem.used_chart`, the network
/// plots on `net.*_chart` and each GPU sparkline on `gpu.devices[i].chart` - so
/// a template renders them without doing per-client chart work.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub cpu: CpuSnapshot,
    pub mem: MemSnapshot,
    pub net: NetSnapshot,
    pub proc: ProcSnapshot,
    pub gpu: GpuSnapshot,
    /// The sampling interval this snapshot was taken at; 0 before the first
    /// sample.
    pub update_ms: u64,
    /// The time the charts cover at this sample's interval and configured
    /// window (ACC-VIEW-110-TIME-AXIS); 0 before the first sample.
    pub window_ms: u64,
    /// The slow pass's interval, the larger of `proc_refresh_ms` and
    /// `update_ms`; 0 before the first sample (ACC-PERF-149-SPLIT-CADENCE).
    pub slow_ms: u64,
    /// The number of completed slow passes; 0 before the first sample. A
    /// stream sends the process fragment when this changes, so the table and
    /// the lines move at the slow rate while the values keep the fast one.
    pub slow_seq: u64,
    /// btop-web runs in a container: the panels without a container source
    /// render as withheld (ACC-SCOPE-64-CONTAINER).
    pub container: bool,
}

impl Snapshot {
    /// The shared time axis under the charts: the window, then each quarter of
    /// it, in seconds, oldest first. Empty before the first sample.
    pub fn axis_labels(&self) -> Vec<String> {
        if self.window_ms == 0 {
            return Vec::new();
        }
        let window_ms = self.window_ms;
        (1..=4)
            .rev()
            .map(|quarter| format!("{} s", window_ms * quarter / 4 / 1000))
            .collect()
    }

    /// The labels under a sparkline's interior vertical grid ticks, oldest
    /// first (ACC-VIEW-128-TICK-LABELS): [`Self::axis_labels`] without its
    /// left-edge label, because the plot boxes draw no tick at their left
    /// edge. The page adds `now` at the right edge.
    pub fn tick_labels(&self) -> Vec<String> {
        self.axis_labels().into_iter().skip(1).collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct CpuSnapshot {
    pub total_pct: Option<f64>,
    /// The CPU gauge arc's value: the history's 1 s EMA, `None` whenever this
    /// tick's raw value is unavailable (ACC-GRAPH-47-GAUGE-SMOOTH).
    pub gauge_pct: Option<f64>,
    pub cores: Vec<CoreUsage>,
    pub frequency_mhz: Option<u64>,
    pub load_avg: Option<[f64; 3]>,
    /// The reporting scale the page shows (ACC-VIEW-130-LOAD-SCALE), set
    /// from the configuration on every sample.
    pub load_scale: LoadScale,
    pub temperature_c: Option<f64>,
    pub battery_pct: Option<f64>,
    /// btop `Cpu::cpuName`, the short model name from `/proc/cpuinfo`.
    pub name: Option<String>,
    /// btop `Cpu::cpuHz`, formatted with btop's own rounding.
    pub cpu_hz: Option<String>,
    /// btop `cpu_info::cpu_percent` for the ten `/proc/stat` fields.
    pub fields: BTreeMap<String, Option<f64>>,
    /// btop `cpu_info::temp[1..]`, aligned with `cores`.
    pub core_temperatures_c: Vec<Option<f64>>,
    /// btop `cpu_info::temp_max`, the package sensor's critical point.
    pub temperature_max_c: Option<f64>,
    /// btop `current_bat` seconds to empty or full.
    pub battery_seconds: Option<i64>,
    /// btop `current_bat` status: charging, discharging, full or unknown.
    pub battery_status: Option<String>,
    /// PCHIP SVG paths for the CPU history panel.
    pub chart: ChartPath,
    /// The container's effective CPUs E (ACC-SCOPE-64); `None` on a host.
    pub effective_cpus: Option<f64>,
}

impl CpuSnapshot {
    pub fn total_label(&self) -> String {
        pct(self.total_pct)
    }

    pub fn total_bar(&self) -> Option<u8> {
        bar(self.total_pct)
    }

    pub fn frequency_label(&self) -> String {
        if let Some(hz) = &self.cpu_hz {
            return hz.clone();
        }
        self.frequency_mhz.map_or_else(
            || "unavailable".to_string(),
            |mhz| format!("{:.2} GHz", mhz as f64 / 1000.0),
        )
    }

    pub fn load_label(&self) -> String {
        self.load_avg.map_or_else(
            || "unavailable".to_string(),
            |[one, five, fifteen]| format!("{one:.2} {five:.2} {fifteen:.2}"),
        )
    }

    /// The 1, 5 and 15 minute load averages divided by [`Self::core_count`],
    /// so 1.00 is one running or waiting task per core (ACC-VIEW-50-CPU-LOAD);
    /// in a container that is E (DEF-CPU-44).
    pub fn load_per_core(&self) -> Option<[f64; 3]> {
        let cores = self.core_count()?;
        let load = self.load_avg?;
        (cores > 0.0).then(|| load.map(|value| value / cores))
    }

    /// [`Self::load_per_core`] for the page, each value four characters wide:
    /// two decimals below 10 (`9.99`), one from 10 (`10.0`). Under tabular
    /// figures the facts column keeps its width, so the CPU arc, which gives
    /// up width to that column, keeps its size (ACC-LAYOUT-74-RESERVED-WIDTH).
    /// A `00.00` reservation would shrink the arc by a sixth at 1024 px.
    pub fn load_per_core_label(&self) -> Option<String> {
        self.load_per_core().map(|load| {
            load.map(|value| {
                if value < 9.995 {
                    format!("{value:.2}")
                } else {
                    format!("{value:.1}")
                }
            })
            .join(" ")
        })
    }

    /// The three load averages in the active reporting scale
    /// (ACC-VIEW-130-LOAD-SCALE), as whole percent: of the whole machine
    /// when normalised, of one core summed over the cores when aggregated.
    pub fn load_percent(&self) -> Option<[f64; 3]> {
        let load = self.load_avg?;
        match self.load_scale {
            LoadScale::Normalised => {
                let cores = self.core_count()?;
                (cores > 0.0).then(|| load.map(|value| value / cores * 100.0))
            }
            LoadScale::Aggregate => Some(load.map(|value| value * 100.0)),
        }
    }

    /// The load line both places render (ACC-VIEW-129-LOAD-LABELS): each
    /// average in the active scale with its kernel window, the widest figure
    /// the line reserves (ACC-VIEW-131-LOAD-WIDTH), and the tooltip. `None`
    /// when the load, or the core count the scale needs, is unreadable.
    pub fn load_readout(&self) -> Option<LoadReadout> {
        let percent = self.load_percent()?;
        let reserve = match self.load_scale {
            LoadScale::Normalised => String::from("100%"),
            LoadScale::Aggregate => {
                let cores = self.core_count()?;
                (cores > 0.0).then(|| format!("{}%", (cores * 100.0).round() as u64))?
            }
        };
        let cell = |index: usize, window: &'static str| LoadCell {
            figure: format!("{:.0}%", percent[index]),
            window,
        };
        let cores = self.cores_label().unwrap_or_default();
        let tip = match self.load_scale {
            LoadScale::Normalised => format!(
                "Load average, unit percent of all cores\n\
                 tasks runnable or blocked in I/O wait\n\
                 1, 5 and 15 minute averages divided by {cores} cores\n\
                 source: /proc/loadavg; container: its own tasks"
            ),
            LoadScale::Aggregate => format!(
                "Load average, unit percent of one core\n\
                 summed over all cores; {cores} cores cap it at {reserve}\n\
                 1, 5 and 15 minute averages, times 100\n\
                 source: /proc/loadavg; container: its own tasks"
            ),
        };
        Some(LoadReadout {
            one: cell(0, "1m"),
            five: cell(1, "5m"),
            fifteen: cell(2, "15m"),
            reserve,
            tip,
        })
    }

    /// The aggregate load of the cores: total usage times the core count /
    /// 100, as busy cores out of [`Self::core_count`].
    pub fn cores_busy(&self) -> Option<f64> {
        Some(self.total_pct? * self.core_count()? / 100.0)
    }

    /// The cores the total is a share of: the container's effective CPUs E,
    /// otherwise the `/proc/stat` cpuN count.
    pub fn core_count(&self) -> Option<f64> {
        self.effective_cpus
            .or_else(|| (!self.cores.is_empty()).then_some(self.cores.len() as f64))
    }

    /// [`Self::core_count`] for the page: `64`, or `1.5` for a fractional E.
    pub fn cores_label(&self) -> Option<String> {
        self.core_count().map(container::cpus_label)
    }

    pub fn temperature_label(&self) -> String {
        celsius(self.temperature_c)
    }

    pub fn battery_label(&self) -> String {
        pct(self.battery_pct)
    }

    /// Rows of the core list, filled column by column as btop lists cores:
    /// at most 16, so 64 cores make 4 columns and the meter keeps at least
    /// 70 px of a 1440-wide row (ACC-VIEW-48-CORE-BARS,
    /// ACC-VIEW-163-CORE-METER).
    pub fn core_rows(&self) -> usize {
        let cores = self.cores.len().min(64);
        let columns = cores.div_ceil(16).max(1);
        cores.div_ceil(columns).max(1)
    }
}

/// One load figure with its kernel window (ACC-VIEW-129-LOAD-LABELS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadCell {
    pub figure: String,
    pub window: &'static str,
}

/// The load line's parts, shared by the CPU sparkline header and the CPU
/// gauge (ACC-VIEW-129-LOAD-LABELS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadReadout {
    /// The 1, 5 and 15 minute averages, each with its window.
    pub one: LoadCell,
    pub five: LoadCell,
    pub fifteen: LoadCell,
    /// The widest figure of the active scale, which every figure reserves
    /// (ACC-VIEW-131-LOAD-WIDTH): `100%` normalised, the core count times
    /// 100 aggregate.
    pub reserve: String,
    /// The stat tooltip: the value's name, then its source lines.
    pub tip: String,
}

#[derive(Debug, Clone, Default)]
pub struct CoreUsage {
    pub name: String,
    pub usage_pct: Option<f64>,
    /// The core's 1 s EMA, which its bar and number show so the panel stops
    /// jumping (ACC-VIEW-48-CORE-BARS); `None` while the raw value is.
    pub smoothed_pct: Option<f64>,
}

impl CoreUsage {
    pub fn pct_label(&self) -> String {
        pct(self.usage_pct)
    }

    pub fn bar(&self) -> Option<u8> {
        bar(self.usage_pct)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemSnapshot {
    pub ram_total: Option<u64>,
    pub ram_used: Option<u64>,
    pub ram_free: Option<u64>,
    pub ram_available: Option<u64>,
    /// The Memory bar's cache segment (DEF-VIEW-50-MEM-TERMS): MemAvailable -
    /// MemFree on a host, the cgroup's `inactive_file` in a container,
    /// docker's cache term (DEF-VIEW-96-MEM-WRONG).
    pub ram_cache: Option<u64>,
    /// The memory used outside this container; `None` on a host and when a
    /// container's `memory.max` binds, where the bar has no such segment.
    pub ram_other: Option<u64>,
    /// The container's `memory.max` when it is below MemTotal: the bar's total
    /// is this limit, free is the room under it, and the header total is the
    /// container's, not the host's. `None` on a host and when the limit is
    /// `max` or at least MemTotal.
    pub ram_limit: Option<u64>,
    pub ram_percent: Option<f64>,
    /// The memory gauge arc's value: the used history's 1 s EMA, `None`
    /// whenever this tick's raw value is unavailable (ACC-GRAPH-47).
    pub gauge_pct: Option<f64>,
    /// btop `mem.percent["free"]`.
    pub ram_free_percent: Option<f64>,
    /// btop `mem.percent["available"]`.
    pub ram_available_percent: Option<f64>,
    pub ram_cache_percent: Option<f64>,
    pub ram_other_percent: Option<f64>,
    pub swap_total: Option<u64>,
    pub swap_used: Option<u64>,
    pub swap_free: Option<u64>,
    pub swap_percent: Option<f64>,
    /// btop `mem.percent["swap_free"]`.
    pub swap_free_percent: Option<f64>,
    /// btop `Mem::has_swap` (`show_swap` and a non-zero swap total).
    pub has_swap: bool,
    /// btop `show_swap` or `swap_disk`: swap is configured, so the panel may
    /// report it unavailable; with both keys off btop hides swap entirely.
    pub swap_enabled: bool,
    pub disks: Vec<DiskUsage>,
    /// PCHIP history path for the used percentage.
    pub used_chart: ChartPath,
}

impl MemSnapshot {
    pub fn ram_label(&self) -> String {
        usage(self.ram_used, self.ram_total)
    }

    /// The used RAM at stat size as its number and unit, `("23.8", "GiB")`,
    /// for the two readouts that set the unit smaller than the value: the
    /// Memory gauge's readout and the Memory tile's big value. One decimal
    /// while the integer part is below three digits, none at three
    /// (ACC-VIEW-161-MEM-SIZE); every other memory label keeps btop's
    /// [`human`] form.
    pub fn ram_used_parts(&self) -> Option<(String, String)> {
        let label = stat_human(self.ram_used?);
        let (value, unit) = label.split_once(' ')?;
        Some((value.to_string(), unit.to_string()))
    }

    /// The Memory tile's full scale in the unit [`Self::ram_used_parts`]
    /// prints, for the number effect's rate threshold (ACC-VIEW-126): the
    /// effective total, so a change under one percent of it updates the text
    /// without starting an effect. Empty when the value is unavailable.
    pub fn ram_used_range(&self) -> String {
        let Some((_, unit)) = self.ram_used_parts() else {
            return String::new();
        };
        let Some(total) = self.ram_total else {
            return String::new();
        };
        let Some(start) = UNITS.iter().position(|name| *name == unit) else {
            return String::new();
        };
        format!("{:.0}", total as f64 / 1024_f64.powi(start as i32))
    }

    pub fn ram_bar(&self) -> Option<u8> {
        bar(self.ram_percent)
    }

    pub fn ram_free_label(&self) -> String {
        bytes(self.ram_legend()[2])
    }

    /// free + cache as the legend shows them, so the line adds up on screen;
    /// in bytes available is free + cache in both modes.
    pub fn ram_available_label(&self) -> String {
        match self.ram_legend() {
            [Some(cache), _, Some(free)] => human((shown(cache).0 + shown(free).0).ceil() as u64),
            _ => bytes(self.ram_available),
        }
    }

    pub fn ram_cache_label(&self) -> String {
        bytes(self.ram_legend()[0])
    }

    pub fn ram_other_label(&self) -> String {
        bytes(self.ram_legend()[1])
    }

    /// The cache, other and free values the legend prints. btop's strings
    /// truncate to their last digit, so four parts can print up to 2 GiB less
    /// than the total. The bytes the truncations drop go back one step at a
    /// time to the part that dropped the most (largest remainder) while half
    /// that part's step is left, so the printed parts add up to the total
    /// within one step and each part stays within one step of its value
    /// (DEF-VIEW-50-MEM-TERMS). used keeps btop's string, as the gauge and the
    /// Memory tile print it. Own code: apportionment crates split one integer
    /// by weights, and these parts have floating_humanizer's mixed steps.
    fn ram_legend(&self) -> [Option<u64>; 3] {
        let parts = [self.ram_cache, self.ram_other, self.ram_free];
        let (Some(total), Some(used), Some(_), Some(_)) =
            (self.ram_total, self.ram_used, self.ram_cache, self.ram_free)
        else {
            return parts;
        };
        let mut left = total as f64 - shown(used).0;
        let mut order = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            if let Some(value) = *part {
                let (printed, step) = shown(value);
                left -= printed;
                order.push((value as f64 - printed, printed + step, step, index));
            }
        }
        order.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut legend = parts;
        for (_, up, step, index) in order {
            if left >= step / 2.0 {
                left -= step;
                legend[index] = Some(up.ceil() as u64);
            }
        }
        legend
    }

    /// The Memory bar's segments in order with their unrounded width in
    /// percent of the total; empty when a part is unknown.
    pub fn ram_segments(&self) -> Vec<(&'static str, f64)> {
        let (Some(total), Some(used), Some(cache), Some(free)) =
            (self.ram_total, self.ram_used, self.ram_cache, self.ram_free)
        else {
            return Vec::new();
        };
        let share = |value: u64| value as f64 * 100.0 / total as f64;
        let mut segments = vec![("used", share(used)), ("cache", share(cache))];
        if let Some(other) = self.ram_other {
            segments.push(("other", share(other)));
        }
        segments.push(("free", share(free)));
        segments
    }

    pub fn swap_label(&self) -> String {
        usage(self.swap_used, self.swap_total)
    }

    /// btop draws swap rows only when a swap key is on and swap exists; a
    /// requested swap the kernel does not report stays visible as unavailable.
    /// With `show_swap` and `swap_disk` both off, btop hides swap entirely.
    pub fn swap_visible(&self) -> bool {
        self.swap_enabled && (self.has_swap || self.swap_total.is_none())
    }

    pub fn swap_bar(&self) -> Option<u8> {
        bar(self.swap_percent)
    }
}

#[derive(Debug, Clone, Default)]
pub struct DiskUsage {
    pub name: String,
    pub mount: String,
    /// btop `disk_info::dev`.
    pub dev: Option<String>,
    /// btop `disk_info::fstype`.
    pub fstype: Option<String>,
    pub used: Option<u64>,
    pub total: Option<u64>,
    /// btop `disk_info::free` (`f_bavail`, or `f_bfree` with `disk_free_priv`).
    pub free: Option<u64>,
    /// btop `disk_info::used_percent`.
    pub used_percent: Option<f64>,
    /// btop `disk_info::free_percent`.
    pub free_percent: Option<f64>,
    /// btop `disk_info::io_read`, the bytes read in the last sample.
    pub io_read: Option<u64>,
    /// btop `disk_info::io_write`, the bytes written in the last sample.
    pub io_write: Option<u64>,
    /// btop `disk_info::io_activity`, busy time as a percentage.
    pub io_activity: Option<f64>,
}

impl DiskUsage {
    pub fn usage_label(&self) -> String {
        usage(self.used, self.total)
    }

    pub fn bar(&self) -> Option<u8> {
        bar(self.used_percent)
    }
}

#[derive(Debug, Clone, Default)]
pub struct NetSnapshot {
    pub interfaces: Vec<NetInterface>,
    /// btop `Net::selected_iface`, the interface btop displays.
    pub selected_iface: Option<String>,
    pub total_down: Option<f64>,
    pub total_up: Option<f64>,
    /// btop `graph_max["download"]`, the graph ceiling in bytes per second.
    pub graph_max_down: Option<u64>,
    /// btop `graph_max["upload"]`, the graph ceiling in bytes per second.
    pub graph_max_up: Option<u64>,
    /// PCHIP history paths for the selected interface's download and upload.
    pub down_chart: ChartPath,
    pub up_chart: ChartPath,
    /// The shown ceilings and the series groups' change, ACC-GRAPH-67.
    pub scale_down: ScaleFrame,
    pub scale_up: ScaleFrame,
}

impl NetSnapshot {
    pub fn total_down_label(&self) -> String {
        rate(self.total_down)
    }

    pub fn total_up_label(&self) -> String {
        rate(self.total_up)
    }

    /// The download plot's scale gutter, top to bottom: the shown ceiling,
    /// its half and zero (DEF-GRAPH-63-GRID-UNITS).
    pub fn down_scale_labels(&self) -> [String; 3] {
        scale_labels(self.scale_down.label_max)
    }

    /// The upload plot's scale gutter, as [`Self::down_scale_labels`].
    pub fn up_scale_labels(&self) -> [String; 3] {
        scale_labels(self.scale_up.label_max)
    }
}

/// The three labels of a plot's scale gutter for a shown ceiling, through
/// btop's short humanizer: the ceiling, its half and zero. With no ceiling
/// the gutter is empty, never the word "unavailable" over the plot.
fn scale_labels(label_max: Option<u64>) -> [String; 3] {
    let Some(max) = label_max else {
        return Default::default();
    };
    [
        bytes_short(Some(max)),
        bytes_short(Some(max / 2)),
        bytes_short(Some(0)),
    ]
}

#[derive(Debug, Clone, Default)]
pub struct NetInterface {
    pub name: String,
    pub ip: Option<String>,
    /// btop `net_info::ipv4`.
    pub ipv4: Option<String>,
    /// btop `net_info::ipv6`.
    pub ipv6: Option<String>,
    /// btop `net_info::connected` (`IFF_RUNNING`).
    pub connected: Option<bool>,
    pub down_rate: Option<f64>,
    pub up_rate: Option<f64>,
    /// btop `net_stat::total` for download (counter plus rollover minus offset).
    pub total_down: Option<u64>,
    /// btop `net_stat::total` for upload (counter plus rollover minus offset).
    pub total_up: Option<u64>,
    /// btop `net_stat::top` for download, the highest observed speed.
    pub top_down: Option<f64>,
    /// btop `net_stat::top` for upload, the highest observed speed.
    pub top_up: Option<f64>,
}

impl NetInterface {
    pub fn ip_label(&self) -> &str {
        self.ip.as_deref().unwrap_or("unavailable")
    }

    pub fn down_label(&self) -> String {
        rate(self.down_rate)
    }

    pub fn up_label(&self) -> String {
        rate(self.up_rate)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProcSnapshot {
    pub total: Option<usize>,
    pub running: Option<usize>,
    /// The Processes tile's facts (ACC-VIEW-53-PROC-TILE), btop-web additions;
    /// [`proc`] records their sources.
    pub sleeping: Option<usize>,
    pub waiting: Option<usize>,
    pub zombie: Option<usize>,
    pub threads: Option<usize>,
    /// The row with the highest Cpu%.
    pub top: Option<ProcEntry>,
    /// The fewest rows whose Cpu% sums reach 80 percent of all rows' Cpu%.
    pub cpu_80: Option<usize>,
    /// The fewest rows whose MemB sums reach 80 percent of all rows' MemB.
    pub mem_80: Option<usize>,
    pub entries: Vec<ProcEntry>,
    /// The process count history, 0 to `chart_max`.
    pub chart: ChartPath,
    pub chart_max: u64,
    /// A change of `chart_max` runs through [`crate::chart::Scale`], as the
    /// network plots do (ACC-GRAPH-67-SMOOTH-SCALE).
    pub scale: ScaleFrame,
}

impl ProcSnapshot {
    pub fn total_label(&self) -> String {
        count(self.total)
    }

    pub fn running_label(&self) -> String {
        count(self.running)
    }

    pub fn threads_label(&self) -> String {
        count(self.threads)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProcEntry {
    pub pid: i32,
    /// btop `proc_info::name`, the `comm` value. Shared by pointer between the
    /// cache and every sample's row, so a tick copies no string
    /// (DEF-PERF-76-SERVER-COST).
    pub name: Option<Arc<str>>,
    pub user: Option<Arc<str>>,
    /// btop `proc_info::cpu_p`.
    pub cpu_pct: Option<f64>,
    /// btop `proc_info::cpu_c`, cumulative CPU over the process age.
    pub cpu_c: Option<f64>,
    pub mem_bytes: Option<u64>,
    /// btop `proc_info::ppid`.
    pub ppid: Option<i32>,
    /// btop `proc_info::threads`.
    pub threads: Option<usize>,
    /// btop `proc_info::p_nice`.
    pub nice: Option<i64>,
    /// btop `proc_info::depth`, the tree-view indentation level.
    pub depth: usize,
    pub state: Option<String>,
    pub command: Arc<str>,
    /// The row graph left of Cpu% (ACC-VIEW-52-PROC-GRAPH); empty without one.
    pub graph: ChartPath,
    /// The graph's tone class: the band of the highest sample in its window,
    /// so the bar keeps the tone of the history it draws (DEF-VIEW-67).
    pub graph_tone: &'static str,
    /// ACC-VIEW-58-PROC-VIEW: the subtree totals a collapsed row shows; `Some`
    /// on rows with at least one child.
    pub sum: Option<ProcSum>,
}

/// btop's collapsed-tree accumulation (`btop_shared.cpp:163-171`): own plus
/// every descendant's value. A total is `None` only when no row of the
/// subtree has that value.
#[derive(Debug, Clone, Default)]
pub struct ProcSum {
    pub cpu_pct: Option<f64>,
    pub mem_bytes: Option<u64>,
    pub threads: Option<usize>,
    /// The summed CPU graph; empty while no descendant used CPU in the last
    /// 10 updates, and on a row below the top level.
    pub graph: ChartPath,
    /// The summed graph's tone class, from its own window's peak (DEF-VIEW-67).
    pub graph_tone: &'static str,
}

impl ProcSum {
    /// `data-sum`: `{cpu} {mem} {threads} {tone}` with the row's own label
    /// rules, `-` for an unavailable total. No label contains a space.
    pub fn labels(&self) -> String {
        format!(
            "{} {} {} {}",
            self.cpu_pct
                .map_or_else(|| "-".to_string(), |_| cpu_text(self.cpu_pct)),
            self.mem_bytes
                .map_or_else(|| "-".to_string(), |_| bytes_short(self.mem_bytes)),
            self.threads
                .map_or_else(|| "-".to_string(), |_| threads_text(self.threads)),
            cpu_tone(self.cpu_pct)
        )
    }
}

/// btop's process CPU column (`btop_draw.cpp:1872-1879`): one decimal below
/// 100, a truncated integer from 100 to 9999, and whole thousands with a `k`
/// suffix from 10000 (`1234.5` -> `1234`, `12345.6` -> `12k`).
pub fn cpu_text(value: Option<f64>) -> String {
    match value {
        None => "unavailable".to_string(),
        Some(value) if value >= 10_000.0 => format!("{}k", (value / 1000.0) as u64),
        Some(value) if value >= 100.0 => format!("{}", value as u64),
        Some(value) => format!("{value:.1}"),
    }
}

/// btop truncates the thread count to thousands with a `K` suffix above 9999
/// (`btop_draw.cpp:1889-1898`): 10000 and 10500 both render `10K`.
pub fn threads_text(value: Option<usize>) -> String {
    match value {
        None => "unavailable".to_string(),
        Some(threads) if threads > 9999 => format!("{}K", threads / 1000),
        Some(threads) => threads.to_string(),
    }
}

/// The Cpu% cell's colour class: `cpu-crit` from 66, `cpu-warn` from 33,
/// `cpu-ok` above 0, `cpu-zero` at 0 and `-` when unavailable.
pub fn cpu_tone(value: Option<f64>) -> &'static str {
    match value {
        None => "-",
        Some(pct) if pct >= 66.0 => "cpu-crit",
        Some(pct) if pct >= 33.0 => "cpu-warn",
        Some(pct) if pct > 0.0 => "cpu-ok",
        Some(_) => "cpu-zero",
    }
}

impl ProcEntry {
    pub fn user_label(&self) -> &str {
        self.user.as_deref().unwrap_or("?")
    }

    pub fn cpu_label(&self) -> String {
        cpu_text(self.cpu_pct)
    }

    pub fn threads_label(&self) -> String {
        threads_text(self.threads)
    }

    pub fn cpu_tone(&self) -> &'static str {
        cpu_tone(self.cpu_pct)
    }

    /// btop draws the process RSS with `floating_humanizer(mem, true)`
    /// (`btop_draw.cpp:1880`), the shortened three-character form.
    pub fn mem_label(&self) -> String {
        bytes_short(self.mem_bytes)
    }

    pub fn state_label(&self) -> &str {
        self.state.as_deref().unwrap_or("?")
    }
}

#[derive(Debug, Clone, Default)]
pub struct GpuSnapshot {
    pub devices: Vec<GpuDevice>,
    /// btop `shared_gpu_percent["gpu-average"]`.
    pub average_utilization_pct: Option<f64>,
    /// btop `shared_gpu_percent["gpu-vram-total"]`.
    pub vram_percent: Option<f64>,
    /// btop `shared_gpu_percent["gpu-pwr-total"]`.
    pub power_percent: Option<f64>,
    /// Distinct pids using any GPU, a btop-web addition (ACC-VIEW-53-PROC-TILE);
    /// in a container, its processes holding a GPU node (DEF-VIEW-48-GPU-FACT).
    /// [`gpu`] records the sources and where it is unavailable.
    pub process_count: Option<usize>,
    /// Container processes whose fds could not be read; above 0 the count is
    /// a lower bound.
    pub process_unreadable: usize,
}

impl GpuSnapshot {
    pub fn available(&self) -> bool {
        !self.devices.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct GpuDevice {
    pub name: String,
    /// btop `gpu_percent["gpu-totals"]`.
    pub utilization_pct: Option<f64>,
    /// btop `mem_utilization_percent`.
    pub memory_utilization_pct: Option<f64>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
    /// btop `pwr_usage` in watts.
    pub power_w: Option<f64>,
    /// btop `pwr_max_usage` in watts.
    pub power_max_w: Option<f64>,
    /// btop `gpu_percent["gpu-pwr-totals"]`.
    pub power_percent: Option<f64>,
    pub temperature_c: Option<f64>,
    /// btop `temp_max`.
    pub temperature_max_c: Option<f64>,
    /// btop `gpu_clock_speed`, MHz.
    pub gpu_clock_mhz: Option<u64>,
    /// btop `mem_clock_speed`, MHz.
    pub memory_clock_mhz: Option<u64>,
    /// btop `pwr_state` (NVML performance state).
    pub pstate: Option<i32>,
    /// btop `pcie_tx`.
    pub pcie_tx: Option<u64>,
    /// btop `pcie_rx`.
    pub pcie_rx: Option<u64>,
    /// PCHIP path for this device's utilization series on the GPU load plot.
    pub chart: ChartPath,
}

impl GpuDevice {
    pub fn utilization_label(&self) -> String {
        pct(self.utilization_pct)
    }

    pub fn utilization_bar(&self) -> Option<u8> {
        bar(self.utilization_pct)
    }

    pub fn vram_label(&self) -> String {
        usage(self.vram_used, self.vram_total)
    }

    pub fn vram_bar(&self) -> Option<u8> {
        match (self.vram_used, self.vram_total) {
            (Some(used), Some(total)) if total > 0 => bar(Some(used as f64 * 100.0 / total as f64)),
            _ => None,
        }
    }

    /// btop's pwr meter: power usage of the power management limit, the same
    /// ratio as `power_percent`; `None` when either reading or the limit is
    /// missing, so the meter draws its unknown state.
    pub fn power_bar(&self) -> Option<u8> {
        match (self.power_w, self.power_max_w) {
            (Some(watts), Some(max)) if max > 0.0 => bar(Some(watts * 100.0 / max)),
            _ => None,
        }
    }

    /// btop draws `to_string(pwr_usage / 1000)` (`btop_draw.cpp:1024`), the
    /// milliwatt reading in whole watts by integer division.
    pub fn power_label(&self) -> String {
        self.power_w.map_or_else(
            || "unavailable".to_string(),
            |watts| format!("{} W", watts as u64),
        )
    }

    /// btop draws a whole degree with its unit (`btop_draw.cpp:1010`):
    /// 45.6 °C renders as `45°C`.
    pub fn temperature_label(&self) -> String {
        self.temperature_c.map_or_else(
            || "unavailable".to_string(),
            |value| format!("{}°C", value as i64),
        )
    }
}

/// ACC-PERF-136-SLEEP: the sleep gate between the sampler and the request
/// handlers. It counts the open SSE streams, remembers the page requests no
/// sample has served yet, and knows when the last sample landed.
///
/// The sampler waits on [`Wake::changes`] while nothing is subscribed, so a
/// parked server holds no timer and reads no source. `/` and `/events` bump
/// the gate, and the sampler wakes to an immediate tick.
pub struct Wake {
    state: watch::Sender<WakeState>,
    /// Whether a sampler is behind this gate. The one [`crate::AppState::new`]
    /// builds has none, so its handlers never wait for a sample.
    attached: bool,
    /// When the last sample was published; `None` before the first one. Kept
    /// out of the watch value so a tick does not wake the sleepers.
    sampled_at: Mutex<Option<Instant>>,
    /// Holds the channel open, so a change is never lost to a closed channel.
    _keep: watch::Receiver<WakeState>,
}

#[derive(Clone, Copy, Default)]
struct WakeState {
    /// Open SSE streams.
    subscribers: usize,
    /// Page requests no sample has served yet.
    requests: usize,
    /// The sampler has stopped its timer and waits for a change.
    parked: bool,
}

impl Wake {
    /// A gate with no sampler behind it.
    pub fn detached() -> Arc<Self> {
        Self::new(false)
    }

    fn attached() -> Arc<Self> {
        Self::new(true)
    }

    fn new(attached: bool) -> Arc<Self> {
        // A fresh server has nobody connected, so its sampler starts parked.
        let (state, keep) = watch::channel(WakeState {
            parked: attached,
            ..WakeState::default()
        });
        Arc::new(Self {
            state,
            attached,
            sampled_at: Mutex::new(None),
            _keep: keep,
        })
    }

    /// One open SSE stream; dropping the guard releases it.
    pub fn subscribe(self: &Arc<Self>) -> Subscription {
        self.modify(|state| state.subscribers += 1);
        Subscription {
            wake: Arc::clone(self),
        }
    }

    /// A page request: the sampler produces a sample even with no stream open.
    pub fn request(&self) {
        self.modify(|state| state.requests += 1);
    }

    /// Whether the newest sample is missing or older than `interval`, so a
    /// handler must wait for the next one instead of showing it.
    pub fn stale(&self, interval: Duration) -> bool {
        self.attached
            && self
                .sampled_at
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_none_or(|at| at.elapsed() > interval)
    }

    fn modify(&self, modify: impl FnOnce(&mut WakeState)) {
        self.state.send_modify(modify);
    }

    /// A receiver that wakes on every subscription, request and park change.
    fn changes(&self) -> watch::Receiver<WakeState> {
        self.state.subscribe()
    }

    fn idle(&self) -> bool {
        let state = self.state.borrow();
        state.subscribers == 0 && state.requests == 0
    }

    fn parked(&self) -> bool {
        self.state.borrow().parked
    }

    fn set_parked(&self, parked: bool) {
        if self.parked() != parked {
            self.modify(|state| state.parked = parked);
        }
    }

    /// This tick serves every page request waiting for one.
    fn take_requests(&self) {
        if self.state.borrow().requests > 0 {
            self.modify(|state| state.requests = 0);
        }
    }

    fn sampled(&self) {
        *self
            .sampled_at
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Instant::now());
    }
}

/// One open SSE stream. The sampler stays awake while it lives.
pub struct Subscription {
    wake: Arc<Wake>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.wake
            .modify(|state| state.subscribers = state.subscribers.saturating_sub(1));
    }
}

/// ACC-PERF-149-SPLIT-CADENCE: the results of the last slow pass - the process
/// walk, the process counts and every chart path - held so each fast frame
/// reuses them unchanged and only the values move.
#[derive(Default)]
struct SlowPass {
    /// Completed passes, counting from the first one.
    seq: u64,
    /// The interval this pass ran at, [`Config::proc_refresh_ms`] raised to
    /// at least `update_ms` and rounded up to a tick boundary, because a slow
    /// pass can never outrun a tick.
    period_ms: u64,
    proc: ProcSnapshot,
    cpu_chart: ChartPath,
    mem_chart: ChartPath,
    net_down: ChartPath,
    net_up: ChartPath,
    scale_down: ScaleFrame,
    scale_up: ScaleFrame,
    gpu_charts: Vec<ChartPath>,
}

impl SlowPass {
    /// Hold `snapshot`'s charts and process walk as pass `seq`.
    fn new(snapshot: &Snapshot, seq: u64, period_ms: u64) -> Self {
        Self {
            seq,
            period_ms,
            proc: snapshot.proc.clone(),
            cpu_chart: snapshot.cpu.chart.clone(),
            mem_chart: snapshot.mem.used_chart.clone(),
            net_down: snapshot.net.down_chart.clone(),
            net_up: snapshot.net.up_chart.clone(),
            scale_down: snapshot.net.scale_down.clone(),
            scale_up: snapshot.net.scale_up.clone(),
            gpu_charts: snapshot
                .gpu
                .devices
                .iter()
                .map(|device| device.chart.clone())
                .collect(),
        }
    }

    /// Put the held slow results back over a fast frame's freshly built ones.
    fn apply(&self, snapshot: &mut Snapshot) {
        snapshot.cpu.chart = self.cpu_chart.clone();
        snapshot.mem.used_chart = self.mem_chart.clone();
        snapshot.net.down_chart = self.net_down.clone();
        snapshot.net.up_chart = self.net_up.clone();
        snapshot.net.scale_down = self.scale_down.clone();
        snapshot.net.scale_up = self.scale_up.clone();
        snapshot.proc = self.proc.clone();
        // A device the sweep added since the pass keeps the chart it just
        // built, because the pass holds nothing for it.
        for (device, chart) in snapshot.gpu.devices.iter_mut().zip(&self.gpu_charts) {
            device.chart = chart.clone();
        }
    }
}

/// The running sampler: the snapshot channel, the sleep gate and the task.
pub struct Sampler {
    /// The latest frame behind an `Arc`; `None` until the first one lands.
    /// Every client clones the pointer, not the sample or the fragments
    /// (ACC-PERF-16-NO-OVERHEAD).
    pub snapshots: watch::Receiver<Option<Arc<Frame>>>,
    /// The gate `/` and `/events` wake.
    pub wake: Arc<Wake>,
    /// Aborting it drops the sender, which ends every SSE stream.
    pub task: JoinHandle<()>,
}

/// Start the sampler, parked until the first request or stream.
///
/// ACC-CONF-61-CONFIG-PAGES: a new `config` value applies from the next tick;
/// a new `update_ms` starts a new interval at once, so every client sees the
/// new cadence within two ticks. A GPU sweep in flight finishes on the old one.
/// ACC-PERF-149-SPLIT-CADENCE: a new `proc_refresh_ms` makes the next tick a
/// slow pass, which anchors the new period.
pub fn spawn(mut config_updates: watch::Receiver<Config>) -> Sampler {
    let config = config_updates.borrow_and_update().clone();
    // btop's `Shared::init` collects the CPU counters before the runner
    // starts, so its first drawn value is a delta over milliseconds. This
    // one read happens before the first park; nothing reads while parked.
    match &config.container {
        Some(container) => cpu::seed_container(&config.proc_root, container, config.update_ms),
        None => cpu::seed(&config.proc_root, config.update_ms),
    }
    let wake = Wake::attached();
    // DEF-CPU-44: the container's load average counts its tasks every
    // LOAD_FREQ on its own timer, off the tick path, one count at a time; the
    // first count ran in `seed_container`. ACC-PERF-136-SLEEP: it parks with
    // the sampler, so a sleeping server counts nothing.
    if let Some(cgroup) = config.container.as_ref().and_then(|c| c.cgroup.clone()) {
        spawn_load_counter(Arc::clone(&wake), config.proc_root.clone(), cgroup);
    }
    let (sender, receiver) = watch::channel(None);
    let sampler_wake = Arc::clone(&wake);
    let task = tokio::spawn(async move {
        let mut config = Arc::new(config);
        let mut update_ms = config.update_ms;
        let proc_root = config.proc_root.clone();
        let mut mem = mem::MemCollector::default();
        // GPU sweeps run off the tick path (see `gpu`), at most one at a time
        // and one per slow pass, plus one started by each wake so its values
        // land in a later frame without holding the first one
        // (ACC-PERF-142-WAKE-FAST).
        let mut gpu_latest = GpuSnapshot::default();
        let mut gpu_sweep: Option<JoinHandle<GpuSnapshot>> = None;
        let mut wake_sweep = false;
        let mut ticker: Option<tokio::time::Interval> = None;
        // ACC-PERF-149-SPLIT-CADENCE: the held slow results and the number of
        // fast ticks before the next pass; 0 makes the next tick slow. The
        // first tick after startup is slow; the first tick after a sleep is
        // fast and the walk follows on the next tick (ACC-PERF-142-WAKE-FAST).
        let mut slow = SlowPass::default();
        let mut ticks_to_slow = 0_u64;
        loop {
            if sampler_wake.idle() {
                // ACC-PERF-136-SLEEP: nothing is connected, so no timer stays
                // registered, no collector reads and no load count runs.
                // Drop any pending timer, so a parked server holds none.
                drop(ticker.take());
                if let Some(sweep) = gpu_sweep.take() {
                    sweep.abort();
                }
                // The held GPU values are dropped so the next frame never
                // shows a value from before the sleep. ACC-PERF-142-WAKE-FAST:
                // the held slow results stay, so the first post-wake frame
                // reuses the walk and the charts it already knows instead of
                // rebuilding them; the next slow pass refreshes them.
                gpu_latest = GpuSnapshot::default();
                sampler_wake.set_parked(true);
                if sampler_wake.idle() {
                    // Subscribe first, then check again: a change before the
                    // receiver exists is caught by the check, one after it
                    // wakes `changed`, so no wake is lost.
                    let mut changes = sampler_wake.changes();
                    if sampler_wake.idle() && changes.changed().await.is_err() {
                        return;
                    }
                }
                sampler_wake.set_parked(false);
                // The first tick after a wake runs at once and reads the
                // configuration that changed while parked.
                config = Arc::new(config_updates.borrow_and_update().clone());
                update_ms = config.update_ms;
                // ACC-PERF-142-WAKE-FAST: after a sleep the first tick runs
                // only the fast collectors and starts the GPU sweep, so the
                // first frame waits for neither the walk nor the sweep; the
                // held slow result rides along and the next tick refreshes
                // it. The first wake after startup runs the slow pass at
                // once, when no result is held.
                let held = slow.seq > 0;
                ticks_to_slow = u64::from(held);
                wake_sweep = held;
                ticker = Some(self::ticker(update_ms));
            }
            let ticker = ticker.get_or_insert_with(|| self::ticker(update_ms));
            ticker.tick().await;
            // ACC-CONF-61-CONFIG-PAGES: apply a new configuration from the next
            // tick; a new interval starts a fresh timer that ticks at once.
            if config_updates.has_changed().unwrap_or(false) {
                config = Arc::new(config_updates.borrow_and_update().clone());
                if config.update_ms != update_ms {
                    update_ms = config.update_ms;
                    *ticker = self::ticker(update_ms);
                }
                // A new slow period applies from the next tick, which runs a
                // slow pass at once and anchors the new period.
                ticks_to_slow = 0;
                continue;
            }
            sampler_wake.take_requests();
            if let Some(sweep) = gpu_sweep.take_if(|sweep| sweep.is_finished())
                && let Ok(gpu) = sweep.await
            {
                gpu_latest = gpu;
            }
            // ACC-PERF-149-SPLIT-CADENCE and DEF-PERF-76-SERVER-COST: the GPU
            // sweep runs once per slow pass, not every tick. The readings are
            // instantaneous, so a held sweep is the value the next one would
            // return; the values move at the slow rate, and every tick still
            // draws the held value (sample and hold), so the chart keeps the
            // tick's time axis. btop at a 250 ms update reads NVML every
            // cycle: this is the walk's stated cadence deviation, extended to
            // the GPU, not a different formula or source.
            // ACC-PERF-142-WAKE-FAST: a wake starts its own sweep at once,
            // so its values land in a later frame of the same stream.
            let sweep_due = std::mem::take(&mut wake_sweep) || ticks_to_slow == 0;
            if sweep_due && gpu_sweep.is_none() {
                let config = Arc::clone(&config);
                gpu_sweep = Some(tokio::task::spawn_blocking(move || {
                    gpu::collect(&config.proc_root, &config)
                }));
            }
            let mut gpu = gpu_latest.clone();
            gpu::attach_charts(&proc_root, &mut gpu.devices, update_ms, config.window_ms());
            let mut snapshot = collect_fast(&proc_root, &config, &mut mem, gpu);
            // ACC-PERF-149-SPLIT-CADENCE: the slow pass rebuilds the process
            // walk and the chart paths; a slow pass never outruns a tick, so
            // its period is at least `update_ms` and lands on a tick boundary,
            // which the page's chart clock reads as `slow_ms`. Counting ticks
            // rather than wall time keeps the 1 s cadence off the timer's
            // jitter: at 250 ms and 1000 ms every fourth tick is slow.
            let every = config
                .proc_refresh_ms
                .max(config.update_ms)
                .div_ceil(config.update_ms);
            let period = every * config.update_ms;
            if ticks_to_slow == 0 {
                // The process collector's histories span wall-clock time, so
                // they are told the slow period, not the tick.
                let mut slow_config = (*config).clone();
                slow_config.update_ms = period;
                snapshot.proc = proc::collect(&proc_root, &slow_config);
                slow = SlowPass::new(&snapshot, slow.seq + 1, period);
                ticks_to_slow = every - 1;
            } else {
                ticks_to_slow -= 1;
                slow.apply(&mut snapshot);
            }
            snapshot.slow_ms = slow.period_ms;
            snapshot.slow_seq = slow.seq;
            sampler_wake.sampled();
            // ACC-PERF-16-NO-OVERHEAD: the fragments are rendered once, here,
            // and every client shares them, so a viewer adds no render work.
            if sender.send(Some(Arc::new(Frame::new(snapshot)))).is_err() {
                break;
            }
        }
    });
    Sampler {
        snapshots: receiver,
        wake,
        task,
    }
}

/// DEF-CPU-44 and ACC-PERF-136-SLEEP: the container's load count every
/// [`cpu::LOAD_FREQ`] while the sampler is awake, one count at a time, and
/// parked with it. The first count after a wake runs at once, so the shown
/// load never predates the sleep.
fn spawn_load_counter(wake: Arc<Wake>, proc_root: PathBuf, cgroup: PathBuf) {
    spawn_load_counter_with(wake, move || {
        cpu::count_container_load(&proc_root, &cgroup);
    });
}

/// [`spawn_load_counter`] with the count injected, so a test can observe the
/// steps without a `/proc` tree.
fn spawn_load_counter_with(wake: Arc<Wake>, count: impl Fn() + Clone + Send + Sync + 'static) {
    tokio::spawn(async move {
        let mut changes = wake.changes();
        loop {
            while wake.parked() {
                if changes.changed().await.is_err() {
                    return;
                }
            }
            let mut at = tokio::time::Instant::now();
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(at) => {}
                    // Only a park ends the loop; a subscription, request or
                    // request reset goes back to the timer without counting.
                    _ = changes.changed() => {
                        if wake.parked() {
                            break;
                        }
                        continue;
                    }
                }
                if wake.parked() {
                    break;
                }
                let count = count.clone();
                let _ = tokio::task::spawn_blocking(count).await;
                at += cpu::LOAD_FREQ;
            }
        }
    });
}

/// A sampling interval whose first tick fires at once; a late tick is skipped.
fn ticker(update_ms: u64) -> tokio::time::Interval {
    let mut ticker = tokio::time::interval(Duration::from_millis(update_ms));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ticker
}

/// The fast collectors, run every tick at `update_ms`; the process walk and
/// the chart strings come from the last slow pass (ACC-PERF-149-SPLIT-CADENCE).
fn collect_fast(
    proc_root: &Path,
    config: &Config,
    mem: &mut mem::MemCollector,
    gpu: GpuSnapshot,
) -> Snapshot {
    let mut cpu = cpu::collect(proc_root, config);
    cpu.load_scale = config.cpu_load;
    Snapshot {
        cpu,
        mem: mem.collect(proc_root, config),
        net: net::collect(proc_root, config),
        gpu,
        update_ms: config.update_ms,
        window_ms: crate::chart::window(config.window_ms(), config.update_ms),
        container: config.container.is_some(),
        ..Snapshot::default()
    }
}

/// The sysfs tree that belongs to `proc_root`. A recorded tree keeps `/sys`
/// beside `/proc` (`tests/fixtures/proc` pairs with `tests/fixtures/sys`); live
/// `/proc` resolves to `/sys`.
pub fn sys_root(proc_root: &Path) -> PathBuf {
    match proc_root.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join("sys"),
        _ => PathBuf::from("/sys"),
    }
}

/// The ok, warn and crit band btop's colour gradient marks at 33% and 66%.
pub fn band(pct: f64) -> &'static str {
    if pct >= 66.0 {
        "crit"
    } else if pct >= 33.0 {
        "warn"
    } else {
        "ok"
    }
}

/// The widest form of a count that never exceeds `max`, for a `data-w`
/// reservation (ACC-LAYOUT-74-RESERVED-WIDTH): one `0` per digit of `max`, at
/// least three, since tabular figures make every digit as wide as `0`. The
/// Processes tile passes the count graph's ceiling: it is at or above the
/// total, so no process count exceeds it, and it changes only with the scale.
pub fn count_width(max: u64) -> String {
    "0".repeat(max.to_string().len().max(3))
}

pub fn pct(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.1}%"),
        None => "unavailable".to_string(),
    }
}

/// `None` when the value is unavailable, so a meter draws its unknown state
/// instead of an empty bar.
pub fn bar(value: Option<f64>) -> Option<u8> {
    value.map(|value| value.clamp(0.0, 100.0).round() as u8)
}

pub fn bytes(value: Option<u64>) -> String {
    value.map_or_else(|| "unavailable".to_string(), human)
}

/// The process table's shortened byte form, `floating_humanizer(value, true)`.
pub fn bytes_short(value: Option<u64>) -> String {
    value.map_or_else(|| "unavailable".to_string(), human_short)
}

pub fn rate(value: Option<f64>) -> String {
    value.map_or_else(
        || "unavailable".to_string(),
        // btop's `net_stat::speed` is u64, and the collector rounds before here.
        |value| format!("{}/s", human(value as u64)),
    )
}

pub fn celsius(value: Option<f64>) -> String {
    value.map_or_else(
        || "unavailable".to_string(),
        |value| format!("{value:.1} °C"),
    )
}

fn usage(used: Option<u64>, total: Option<u64>) -> String {
    match (used, total) {
        (Some(used), Some(total)) => format!("{} / {}", bytes(Some(used)), bytes(Some(total))),
        _ => "unavailable".to_string(),
    }
}

fn count(value: Option<usize>) -> String {
    value.map_or_else(|| "unavailable".to_string(), |value| value.to_string())
}

/// btop 1.3.0 `floating_humanizer` (`src/btop_tools.cpp:399`) for the call shape
/// this page uses: `start=0`, `bit=false`, and the per-second suffix left to
/// [`rate`]. btop works in integer hundredths of a byte, shifts right by 10
/// while the value is at least 102400, then truncates the decimal string to
/// three significant digits: a four-digit value above the base unit loses its
/// last digit and gains a dot at 2 (`10.0`), a three-digit value above the
/// base unit gains a dot at 1 (`1.00`), and any other value of two or more
/// digits loses its last two (`167`; below 1024 this leaves the exact byte
/// count, `1023 Byte`). With `shorten` the result is refolded to three
/// characters and the unit's first letter (`1.0K`, `1.5M`), the process
/// table's MemB cell.
///
/// Kept local: no crate reproduces this truncation. humansize 2.1.3 and
/// byte-unit 5.2.6 format a fixed number of decimal places (`10.00 KiB`), add
/// decimals below 1024 (`1023.00 B`) and name the base unit `B`.
///
/// The hundredths product uses u128 because btop's u64 multiply wraps above
/// `u64::MAX / 100`; u128 holds every u64 input without wrapping, and the
/// digits are identical wherever btop does not wrap.
fn human(value: u64) -> String {
    human_with(value, false)
}

/// `floating_humanizer(value, shorten=true, 0, false, false)`.
fn human_short(value: u64) -> String {
    human_with(value, true)
}

const UNITS: [&str; 11] = [
    "Byte", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB", "BiB", "GEB",
];

/// A [`human`] string's value and the step of its last digit, in bytes:
/// `13.1 GiB` is 13.1 GiB with a 0.1 GiB step.
fn shown(value: u64) -> (f64, f64) {
    let label = human(value);
    let (number, unit) = label.split_once(' ').unwrap_or_default();
    let scale = 1024_f64.powi(UNITS.iter().position(|name| *name == unit).unwrap_or(0) as i32);
    let decimals = number.split_once('.').map_or(0, |(_, digits)| digits.len());
    let step = scale / 10_f64.powi(decimals as i32);
    (number.parse::<f64>().unwrap_or(0.0) * scale, step)
}

fn human_with(value: u64, shorten: bool) -> String {
    let mut value = u128::from(value) * 100;
    let mut start = 0;
    let mut out = String::new();
    while value >= 102_400 {
        value >>= 10;
        if value < 100 {
            out = value.to_string();
            break;
        }
        start += 1;
    }
    if out.is_empty() {
        out = value.to_string();
        match out.len() {
            4 if start > 0 => {
                out.pop();
                out.insert(2, '.');
            }
            3 if start > 0 => {
                out.insert(1, '.');
            }
            len if len >= 2 => out.truncate(len - 2),
            _ => {}
        }
    }
    if !shorten {
        return format!("{out} {}", UNITS[start]);
    }
    let rounded = out.parse::<f64>().unwrap_or_default();
    if out.find('.') == Some(1) && out.len() > 3 {
        // `to_string(round(stod(out) * 10) / 10).substr(0, 3)`: btop's
        // std::to_string keeps six decimals, so 10.0 becomes the `10.` quirk.
        out = format!("{:.6}", (rounded * 10.0).round() / 10.0);
        out.truncate(3);
    } else if out.contains('.') {
        out = format!("{:.0}", rounded.round());
    }
    if out.len() > 3 {
        out = format!("{}.0", &out[..1]);
        start += 1;
    }
    out.push_str(&UNITS[start][..1]);
    out
}

/// The stat-size memory form (ACC-VIEW-161-MEM-SIZE): one decimal while the
/// integer part is below three digits, none at three, so `23.8 GiB` and
/// `99.9 GiB` keep the tenth and `100 GiB` and `123 GiB` drop it. Only the
/// Memory gauge's readout and the Memory tile's big value use it; btop's
/// [`human`] still prints the table cells, legend rows, tooltips and the
/// header's total. Rounds rather than truncates, so the printed value is the
/// nearest tenth and stays within the four-character reserve.
fn stat_human(value: u64) -> String {
    let mut start = 0;
    let mut scaled = value as f64;
    loop {
        while scaled >= 1024.0 && start + 1 < UNITS.len() {
            scaled /= 1024.0;
            start += 1;
        }
        // The tenth is rounded before the width is chosen, so 99.96 prints
        // `100` and not the five-character `100.0`.
        let text = if start == 0 || (scaled * 10.0).round() >= 1000.0 {
            format!("{scaled:.0}")
        } else {
            format!("{scaled:.1}")
        };
        if text != "1024" || start + 1 == UNITS.len() {
            return format!("{text} {}", UNITS[start]);
        }
        // A value rounding up to the next unit's base takes the step rather
        // than printing `1024 GiB`.
        scaled /= 1024.0;
        start += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ACC-VIEW-110-TIME-AXIS and ACC-VIEW-128-TICK-LABELS: the axis labels
    /// read the window the charts cover, and a sparkline drops the left-edge
    /// label because its plot box draws no tick there.
    #[test]
    fn the_axis_labels_follow_the_window() {
        let snapshot = Snapshot {
            update_ms: 250,
            window_ms: 600_000,
            ..Snapshot::default()
        };
        assert_eq!(snapshot.axis_labels(), ["600 s", "450 s", "300 s", "150 s"]);
        assert_eq!(snapshot.tick_labels(), ["450 s", "300 s", "150 s"]);
        assert!(Snapshot::default().axis_labels().is_empty());
        assert!(Snapshot::default().tick_labels().is_empty());
    }

    /// ACC-VIEW-163-CORE-METER: the list fills at most 16 rows, so 64 cores
    /// make 4 columns and every row keeps a meter wide enough to read; a
    /// shorter list keeps one core per row until it reaches the cap.
    #[test]
    fn core_rows_stop_at_sixteen() {
        let with = |n: usize| CpuSnapshot {
            cores: (0..n)
                .map(|i| CoreUsage {
                    name: format!("cpu{i}"),
                    usage_pct: None,
                    smoothed_pct: None,
                })
                .collect(),
            ..CpuSnapshot::default()
        };
        assert_eq!(with(64).core_rows(), 16);
        assert_eq!(with(33).core_rows(), 11);
        assert_eq!(with(17).core_rows(), 9);
        assert_eq!(with(16).core_rows(), 16);
        assert_eq!(with(8).core_rows(), 8);
        assert_eq!(with(0).core_rows(), 1);
    }

    #[test]
    fn load_per_core_label_keeps_four_characters_per_value() {
        let cpu = CpuSnapshot {
            load_avg: Some([0.5, 19.98, 25.0]),
            effective_cpus: Some(2.0),
            ..CpuSnapshot::default()
        };
        assert_eq!(cpu.load_per_core_label().as_deref(), Some("0.25 9.99 12.5"));
    }

    /// ACC-VIEW-129-LOAD-LABELS, ACC-VIEW-130-LOAD-SCALE and
    /// ACC-VIEW-131-LOAD-WIDTH: each average carries its window, the figure
    /// is a whole percent in the active scale, and the line reserves that
    /// scale's widest form - 100% normalised, the core count times 100
    /// aggregate.
    #[test]
    fn load_readout_carries_windows_and_the_scales_widest_form() {
        let cpu = CpuSnapshot {
            load_avg: Some([32.0, 16.0, 9.6]),
            effective_cpus: Some(64.0),
            ..CpuSnapshot::default()
        };
        let normalised = cpu.load_readout().expect("a normalised readout");
        assert_eq!(
            (normalised.one.window, normalised.one.figure.as_str()),
            ("1m", "50%")
        );
        assert_eq!(
            (normalised.five.window, normalised.five.figure.as_str()),
            ("5m", "25%")
        );
        assert_eq!(
            (
                normalised.fifteen.window,
                normalised.fifteen.figure.as_str()
            ),
            ("15m", "15%")
        );
        assert_eq!(normalised.reserve, "100%");
        let aggregate = CpuSnapshot {
            load_scale: LoadScale::Aggregate,
            ..cpu.clone()
        }
        .load_readout()
        .expect("an aggregate readout");
        assert_eq!(aggregate.one.figure, "3200%");
        assert_eq!(aggregate.five.figure, "1600%");
        assert_eq!(aggregate.fifteen.figure, "960%");
        assert_eq!(aggregate.reserve, "6400%");
        // A load without a core count has no normalised percent to show.
        let uncounted = CpuSnapshot {
            load_avg: Some([1.0, 1.0, 1.0]),
            ..CpuSnapshot::default()
        };
        assert!(uncounted.load_readout().is_none());
    }

    #[test]
    fn count_width_reserves_the_digits_of_the_bound() {
        for (max, width) in [
            (0, "000"),
            (8, "000"),
            (999, "000"),
            (1000, "0000"),
            (54321, "00000"),
        ] {
            assert_eq!(count_width(max), width, "{max}");
        }
    }

    /// Expected strings are emitted by
    /// `tmp/workflows/btop-web-humanizer7/checkpoints/humanizer.cpp`, which runs btop
    /// 1.3.0's `floating_humanizer` body verbatim with `mega=false`,
    /// `shorten=false`, `start=0` and `bit=false`.
    #[test]
    fn human_matches_btop_1_3_0_floating_humanizer() {
        const VECTORS: [(u64, &str); 27] = [
            (0, "0 Byte"),
            (1, "1 Byte"),
            (9, "9 Byte"),
            (10, "10 Byte"),
            (99, "99 Byte"),
            (100, "100 Byte"),
            (999, "999 Byte"),
            (1023, "1023 Byte"),
            (1024, "1.00 KiB"),
            (1025, "1.00 KiB"),
            (1536, "1.50 KiB"),
            (2028, "1.98 KiB"),
            (2048, "2.00 KiB"),
            (10239, "9.99 KiB"),
            (10240, "10.0 KiB"),
            (102400, "100 KiB"),
            (104857, "102 KiB"),
            (104858, "102 KiB"),
            (1048575, "1023 KiB"),
            (1048576, "1.00 MiB"),
            (1100000, "1.04 MiB"),
            (12345678, "11.7 MiB"),
            (104857600, "100 MiB"),
            (1073741824, "1.00 GiB"),
            (180177931468, "167 GiB"),
            (25501368320, "23.7 GiB"),
            (1099511627776, "1.00 TiB"),
        ];
        for (value, expected) in VECTORS {
            assert_eq!(human(value), expected, "human({value})");
        }
        // btop's u64 multiply wraps at this input; the u128 product keeps it
        // exact, so the call returns instead of panicking on a wrapped unit.
        assert_eq!(human(u64::MAX), "15.9 EiB");
    }

    #[test]
    fn human_helpers_keep_the_unavailable_contract() {
        assert_eq!(bytes(Some(10 * 1024 * 1024)), "10.0 MiB");
        assert_eq!(bytes(Some(3 * 1024 * 1024 * 1024)), "3.00 GiB");
        assert_eq!(bytes(Some(2 * 1024_u64.pow(4))), "2.00 TiB");
        assert_eq!(bytes(None), "unavailable");
        assert_eq!(rate(None), "unavailable");
        assert_eq!(rate(Some(1024.0)), "1.00 KiB/s");
    }

    /// ACC-VIEW-161-MEM-SIZE: the stat-size memory form behind the gauge
    /// readout and the Memory tile keeps one decimal while the integer part
    /// is below three digits and drops it at three, so the value never
    /// outgrows the four-character reserve; every other label keeps btop's
    /// truncated humanizer form.
    #[test]
    fn stat_size_memory_drops_the_decimal_at_three_digits() {
        fn gib(value: f64) -> u64 {
            (value * 1024.0 * 1024.0 * 1024.0) as u64
        }
        for (value, expected) in [
            (23.8, "23.8 GiB"),
            (99.9, "99.9 GiB"),
            (100.0, "100 GiB"),
            (123.4, "123 GiB"),
        ] {
            assert_eq!(stat_human(gib(value)), expected, "{value} GiB");
        }
        // the table cells, legend rows, tooltips and the header's total keep
        // btop's truncation
        assert_eq!(human(gib(23.8)), "23.7 GiB");
        assert_eq!(human(gib(99.9)), "99.8 GiB");
        // a tenth that rounds into three digits drops the fraction instead of
        // printing `100.0`, and a value that rounds to the next unit's base
        // takes the step instead of printing `1024 GiB`
        assert_eq!(stat_human(gib(99.96)), "100 GiB");
        assert_eq!(stat_human(gib(1023.99)), "1.0 TiB");
        // below the base unit the number stays an integer byte count
        assert_eq!(stat_human(512), "512 Byte");
        // the two call sites read the stat form through ram_used_parts
        let snapshot = MemSnapshot {
            ram_used: Some(gib(123.4)),
            ..MemSnapshot::default()
        };
        assert_eq!(
            snapshot.ram_used_parts(),
            Some(("123".to_string(), "GiB".to_string()))
        );
        // every value the formatter prints fits the four-character reserve
        // and carries at most one decimal
        for tenths in 1..=12_000 {
            let label = stat_human(tenths * (1 << 30) / 10);
            let number = label.split_once(' ').expect("a unit").0;
            assert!(number.len() <= 4, "{label}");
            assert!(number.matches('.').count() <= 1, "{label}");
        }
    }

    /// ACC-VIEW-126: the Memory tile's animation range is the effective total
    /// in the same unit as the stat value, so the rate threshold divides the
    /// displayed change by the display range; an unavailable value has none.
    #[test]
    fn memory_animation_range_follows_the_displayed_unit() {
        let gib = 1u64 << 30;
        let snapshot = MemSnapshot {
            ram_used: Some(9 * gib),
            ram_total: Some(16 * gib),
            ..MemSnapshot::default()
        };
        assert_eq!(
            snapshot.ram_used_parts(),
            Some(("9.0".to_string(), "GiB".to_string()))
        );
        assert_eq!(snapshot.ram_used_range(), "16");
        let snapshot = MemSnapshot {
            ram_used: Some(300 * (1 << 20)),
            ram_total: Some(2 * gib),
            ..MemSnapshot::default()
        };
        assert_eq!(
            snapshot.ram_used_parts(),
            Some(("300".to_string(), "MiB".to_string()))
        );
        assert_eq!(snapshot.ram_used_range(), "2048");
        assert_eq!(MemSnapshot::default().ram_used_range(), "");
    }

    /// Expected strings are emitted by
    /// `tmp/workflows/btop-web-parity8/probe/humanizer-short.cpp`, which runs
    /// btop 1.3.0's `floating_humanizer` body verbatim with `mega=false`,
    /// `shorten=true`, `start=0` and `bit=false`.
    #[test]
    fn human_short_matches_btop_1_3_0_floating_humanizer_shorten() {
        const VECTORS: [(u64, &str); 28] = [
            (0, "0B"),
            (1, "1B"),
            (9, "9B"),
            (10, "10B"),
            (99, "99B"),
            (100, "100B"),
            (999, "999B"),
            (1023, "1.0K"),
            (1024, "1.0K"),
            (1025, "1.0K"),
            (1536, "1.5K"),
            (2028, "2.0K"),
            (2048, "2.0K"),
            (10239, "10.K"),
            (10240, "10K"),
            (102400, "100K"),
            (104857, "102K"),
            (104858, "102K"),
            (1048575, "1.0M"),
            (1048576, "1.0M"),
            (1100000, "1.0M"),
            (1572864, "1.5M"),
            (12345678, "12M"),
            (104857600, "100M"),
            (1073741824, "1.0G"),
            (180177931468, "167G"),
            (25501368320, "24G"),
            (1099511627776, "1.0T"),
        ];
        for (value, expected) in VECTORS {
            assert_eq!(human_short(value), expected, "human_short({value})");
        }
        assert_eq!(bytes_short(Some(1572864)), "1.5M");
        assert_eq!(bytes_short(None), "unavailable");
    }

    /// btop `btop_draw.cpp:1872-1898`: the process CPU column keeps one
    /// decimal below 100, truncates to an integer from 100 to 9999, and above
    /// 9999 prints whole thousands with `k`; threads above 9999 print whole
    /// thousands with `K`; MemB uses the shortened humanizer.
    #[test]
    fn process_labels_match_btop_process_row() {
        let cpu = |value| {
            ProcEntry {
                cpu_pct: Some(value),
                ..ProcEntry::default()
            }
            .cpu_label()
        };
        assert_eq!(cpu(0.0), "0.0");
        assert_eq!(cpu(9.9), "9.9");
        assert_eq!(cpu(10.0), "10.0");
        assert_eq!(cpu(42.5), "42.5");
        assert_eq!(cpu(100.0), "100");
        assert_eq!(cpu(123.4), "123");
        assert_eq!(cpu(999.9), "999");
        assert_eq!(cpu(1000.0), "1000");
        assert_eq!(cpu(1234.5), "1234");
        assert_eq!(cpu(9999.9), "9999");
        assert_eq!(cpu(10_000.0), "10k");
        assert_eq!(cpu(12_345.6), "12k");
        assert_eq!(ProcEntry::default().cpu_label(), "unavailable");

        let threads = |value| {
            ProcEntry {
                threads: Some(value),
                ..ProcEntry::default()
            }
            .threads_label()
        };
        assert_eq!(threads(9999), "9999");
        assert_eq!(threads(10_000), "10K");
        assert_eq!(threads(10_500), "10K");
        assert_eq!(ProcEntry::default().threads_label(), "unavailable");

        let mem = |value| {
            ProcEntry {
                mem_bytes: Some(value),
                ..ProcEntry::default()
            }
            .mem_label()
        };
        assert_eq!(mem(1023), "1.0K");
        assert_eq!(mem(1572864), "1.5M");
        assert_eq!(mem(10239), "10.K");
        assert_eq!(mem(10240), "10K");
        assert_eq!(ProcEntry::default().mem_label(), "unavailable");
    }

    /// ACC-VIEW-58-PROC-VIEW: the summed labels a collapsed row shows.
    #[test]
    fn sum_labels_tokens() {
        let sum = ProcSum {
            cpu_pct: Some(45.0),
            mem_bytes: Some(1_572_864),
            threads: Some(37),
            ..ProcSum::default()
        };
        assert_eq!(sum.labels(), "45.0 1.5M 37 cpu-warn");
        assert_eq!(ProcSum::default().labels(), "- - - -");
    }

    #[test]
    fn cpu_tone_thresholds() {
        assert_eq!(cpu_tone(Some(0.0)), "cpu-zero");
        assert_eq!(cpu_tone(Some(0.1)), "cpu-ok");
        assert_eq!(cpu_tone(Some(33.0)), "cpu-warn");
        assert_eq!(cpu_tone(Some(66.0)), "cpu-crit");
        assert_eq!(cpu_tone(None), "-");
    }

    /// btop `btop_draw.cpp:1010,1024`: the GPU temperature is a whole degree
    /// with its unit, and the GPU power is the milliwatt reading in whole
    /// watts by integer division.
    #[test]
    fn gpu_labels_match_btop_draw() {
        let device = |power_w: Option<f64>, temperature_c: Option<f64>| GpuDevice {
            power_w,
            temperature_c,
            ..GpuDevice::default()
        };
        assert_eq!(device(Some(123.456), None).power_label(), "123 W");
        assert_eq!(device(Some(123.0), None).power_label(), "123 W");
        assert_eq!(device(Some(0.999), None).power_label(), "0 W");
        assert_eq!(device(None, Some(45.6)).temperature_label(), "45°C");
        assert_eq!(device(None, Some(45.0)).temperature_label(), "45°C");
        assert_eq!(GpuDevice::default().power_label(), "unavailable");
        assert_eq!(GpuDevice::default().temperature_label(), "unavailable");
    }

    /// DEF-VIEW-50-MEM-TERMS: btop's truncated strings alone printed
    /// 13.1 + 37.4 + 225 + 226 GiB of a 503.2 GiB total.
    #[test]
    fn memory_legend_adds_up_to_the_total_within_one_step() {
        let total: u64 = 540_312_911_872;
        let mut short_by_more_than_a_step = 0;
        for i in 0..400_u64 {
            let used = 14_000_000_000 + i * 7_919_113;
            let cache = 40_000_000_000 + i * 104_729_011;
            let other = (i % 2 == 0).then(|| 240_000_000_000 + i * 15_485_863);
            let free = total - used - cache - other.unwrap_or(0);
            let mem = MemSnapshot {
                ram_total: Some(total),
                ram_used: Some(used),
                ram_cache: Some(cache),
                ram_other: other,
                ram_free: Some(free),
                ram_available: Some(free + cache),
                ..MemSnapshot::default()
            };
            let truncated: f64 = [Some(used), Some(cache), other, Some(free)]
                .into_iter()
                .flatten()
                .map(|value| shown(value).0)
                .sum();
            if total as f64 - truncated > (1u64 << 30) as f64 {
                short_by_more_than_a_step += 1;
            }
            let legend = mem.ram_legend();
            let mut printed = shown(used).0;
            let mut largest_step = shown(used).1;
            for (part, value) in legend.iter().zip([Some(cache), other, Some(free)]) {
                let (Some(part), Some(value)) = (*part, value) else {
                    continue;
                };
                let (part_printed, step) = shown(part);
                assert!(
                    (part_printed - value as f64).abs() < step,
                    "{i}: {part} vs {value}"
                );
                printed += part_printed;
                largest_step = largest_step.max(step);
            }
            assert!(
                (total as f64 - printed).abs() < largest_step,
                "{i}: {printed}"
            );
            assert_eq!(mem.ram_free_label(), human(legend[2].unwrap()));
            let free_cache = shown(legend[0].unwrap()).0 + shown(legend[2].unwrap()).0;
            let (available, step) = shown(free_cache.ceil() as u64);
            assert_eq!(mem.ram_available_label(), human(free_cache.ceil() as u64));
            assert!(available <= free_cache && free_cache < available + step);
        }
        assert!(short_by_more_than_a_step > 0);
    }

    #[test]
    fn swap_visibility_follows_the_swap_keys() {
        let snapshot = |swap_enabled, has_swap, swap_total| MemSnapshot {
            swap_enabled,
            has_swap,
            swap_total,
            ..MemSnapshot::default()
        };
        // Both keys off: btop reads no swap and draws no swap rows.
        assert!(!snapshot(false, false, None).swap_visible());
        assert!(!snapshot(false, false, Some(1 << 30)).swap_visible());
        // A swap key on but no readable total stays visible as unavailable.
        assert!(snapshot(true, false, None).swap_visible());
        // A swap key on with swap present.
        assert!(snapshot(true, true, Some(1 << 30)).swap_visible());
        // Zero swap total: btop's has_swap is false, so there is no row.
        assert!(!snapshot(true, false, Some(0)).swap_visible());
    }

    /// ACC-CONF-61-CONFIG-PAGES: a new interval from the config menu reaches
    /// the snapshots within two ticks and sets the next gap. An open stream
    /// keeps the sampler awake to see it (ACC-PERF-136-SLEEP).
    #[tokio::test]
    async fn sampler_follows_a_new_update_interval() {
        let mut config = Config::defaults();
        config.proc_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proc");
        config.update_ms = 100;
        let (updates, receiver) = watch::channel(config.clone());
        let sampler = spawn(receiver);
        let _stream = sampler.wake.subscribe();
        let mut snapshots = sampler.snapshots;
        snapshots.changed().await.expect("first snapshot");
        config.update_ms = 300;
        updates.send_replace(config);
        let mut seen = 0;
        while snapshots
            .borrow()
            .as_ref()
            .map(|frame| frame.snapshot.update_ms)
            != Some(300)
        {
            assert!(
                seen < 2,
                "update_ms 300 did not arrive within two snapshots"
            );
            snapshots.changed().await.expect("snapshot");
            seen += 1;
        }
        let at = std::time::Instant::now();
        snapshots.changed().await.expect("next snapshot");
        assert!(
            at.elapsed() >= Duration::from_millis(250),
            "{:?}",
            at.elapsed()
        );
        sampler.task.abort();
    }

    /// ACC-VIEW-130-LOAD-SCALE: the configured load scale reaches the
    /// sampler's snapshot and its rendered fragments every tick.
    #[tokio::test]
    async fn sampler_renders_the_configured_load_scale() {
        let root = temp_proc_tree("load-scale");
        // The fixture's loadavg is read-only; a fresh file replaces it.
        let loadavg = root.join("loadavg");
        std::fs::remove_file(&loadavg).expect("loadavg");
        std::fs::write(&loadavg, "32.00 16.00 9.60 2/99 1234\n").expect("loadavg");
        let mut config = Config::defaults();
        config.proc_root = root;
        config.update_ms = 100;
        config.cpu_load = LoadScale::Aggregate;
        let (_updates, receiver) = watch::channel(config);
        let sampler = spawn(receiver);
        let _stream = sampler.wake.subscribe();
        let mut snapshots = sampler.snapshots;
        snapshots.changed().await.expect("first snapshot");
        let frame = snapshots.borrow().clone().expect("a frame");
        assert_eq!(frame.snapshot.cpu.load_scale, LoadScale::Aggregate);
        let cpu = frame
            .events
            .iter()
            .find(|(name, _)| *name == "cpu")
            .map(|(_, html)| html.as_str())
            .expect("the cpu fragment");
        assert!(
            cpu.contains(r#"<span data-w="6400%" data-num>3200%</span>/1m"#),
            "{cpu}"
        );
        sampler.task.abort();
    }

    /// A private copy of the recorded `/proc` tree, so a test can add a
    /// process without touching the fixture or another test's collector state.
    fn temp_proc_tree(name: &str) -> PathBuf {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proc");
        let base =
            std::env::temp_dir().join(format!("btop-web-split-{}-{name}", std::process::id()));
        let root = base.join("proc");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&root).expect("tree");
        copy_tree(&source, &root);
        root
    }

    fn copy_tree(source: &Path, target: &Path) {
        for entry in std::fs::read_dir(source).expect("fixture") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let destination = target.join(entry.file_name());
            if path.is_dir() {
                std::fs::create_dir_all(&destination).expect("directory");
                copy_tree(&path, &destination);
            } else {
                std::fs::copy(&path, &destination).expect("file");
            }
        }
    }

    /// ACC-PERF-149-SPLIT-CADENCE: a process that appears between slow passes
    /// is listed by the next slow pass, never by the fast frames before it,
    /// which reuse the held walk; the configured refresh period reaches the
    /// sampler as each snapshot's `slow_ms`.
    #[tokio::test]
    async fn fast_frames_reuse_the_process_walk_until_the_next_slow_pass() {
        let root = temp_proc_tree("slow-pass");
        let mut config = Config::defaults();
        config.proc_root = root.clone();
        config.update_ms = 50;
        config.proc_refresh_ms = 300;
        let (_updates, receiver) = watch::channel(config);
        let sampler = spawn(receiver);
        let _stream = sampler.wake.subscribe();
        let mut snapshots = sampler.snapshots;
        snapshots.changed().await.expect("first snapshot");
        let first = snapshots.borrow_and_update().clone().expect("snapshot");
        assert_eq!(
            first.snapshot.slow_seq, 1,
            "the first tick is not a slow pass"
        );
        assert_eq!(first.snapshot.slow_ms, 300);
        assert_eq!(first.snapshot.update_ms, 50);
        let before = first.snapshot.proc.total.expect("a process total");
        // A new process appears between slow passes.
        std::fs::create_dir_all(root.join("999")).expect("pid directory");
        copy_tree(&root.join("222"), &root.join("999"));
        let mut second = None;
        for _ in 0..40 {
            snapshots.changed().await.expect("a snapshot");
            let frame = snapshots.borrow_and_update().clone().expect("snapshot");
            assert_eq!(frame.snapshot.slow_ms, 300, "the slow period changed");
            if frame.snapshot.slow_seq == 1 {
                assert_eq!(
                    frame.snapshot.proc.total,
                    Some(before),
                    "a fast frame rebuilt the process walk"
                );
            } else {
                second = Some(frame);
                break;
            }
        }
        let second = second.expect("no second slow pass within 40 ticks");
        assert_eq!(second.snapshot.slow_seq, 2);
        assert_eq!(
            second.snapshot.proc.total,
            Some(before + 1),
            "the slow pass missed the process that appeared"
        );
        sampler.task.abort();
        let _ = std::fs::remove_dir_all(root.parent().expect("tree base"));
    }

    /// ACC-PERF-149-SPLIT-CADENCE and ACC-CONF-61-CONFIG-PAGES: a new
    /// `proc_refresh_ms` from the config menu reaches the sampler and its
    /// snapshots within two ticks, and anchors a fresh slow pass.
    #[tokio::test]
    async fn sampler_follows_a_new_process_refresh_period() {
        let mut config = Config::defaults();
        config.proc_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proc");
        config.update_ms = 100;
        let (updates, receiver) = watch::channel(config.clone());
        let sampler = spawn(receiver);
        let _stream = sampler.wake.subscribe();
        let mut snapshots = sampler.snapshots;
        snapshots.changed().await.expect("first snapshot");
        assert_eq!(
            snapshots
                .borrow()
                .as_ref()
                .map(|frame| frame.snapshot.slow_ms),
            Some(1000)
        );
        config.set("proc_refresh_ms", "500").expect("valid");
        updates.send_replace(config);
        let mut seen = 0;
        while snapshots
            .borrow()
            .as_ref()
            .map(|frame| frame.snapshot.slow_ms)
            != Some(500)
        {
            assert!(
                seen < 2,
                "proc_refresh_ms 500 did not arrive within two snapshots"
            );
            snapshots.changed().await.expect("snapshot");
            seen += 1;
        }
        assert_eq!(
            snapshots
                .borrow()
                .as_ref()
                .map(|frame| frame.snapshot.slow_seq),
            Some(2)
        );
        sampler.task.abort();
    }

    /// DEF-CPU-44 and ACC-PERF-136-SLEEP: only the LOAD_FREQ timer adds a
    /// load step. A page request, an SSE connect or a request reset wakes the
    /// gate too, and each one had been falling through to a full count.
    #[tokio::test]
    async fn a_wake_burst_adds_no_load_step() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let wake = Wake::attached();
        let steps = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&steps);
        spawn_load_counter_with(Arc::clone(&wake), move || {
            count.fetch_add(1, Ordering::Relaxed);
        });
        // Unpark: the first step runs at once.
        wake.set_parked(false);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while steps.load(Ordering::Relaxed) == 0 {
            assert!(tokio::time::Instant::now() < deadline, "no load step");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let first = steps.load(Ordering::Relaxed);
        // A burst that leaves the sampler awake.
        for _ in 0..8 {
            wake.request();
            drop(wake.subscribe());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            steps.load(Ordering::Relaxed),
            first,
            "a wake notification added a load step"
        );
        // Parking still ends the count loop.
        wake.set_parked(true);
    }
}

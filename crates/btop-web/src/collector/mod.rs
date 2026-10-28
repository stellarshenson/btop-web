//! Snapshot types, the sampling scheduler and the per-metric collectors.
//!
//! One sampler runs at `update_ms` and publishes a [`Snapshot`] through a
//! `tokio::watch` channel; every SSE client shares that value, so viewers do
//! not multiply `/proc` reads.
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
pub mod gpu;
pub mod mem;
pub mod net;
pub mod proc;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::chart::{ChartPath, ScaleFrame};
use crate::config::Config;

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
    /// btop-web runs in a container: the panels without a container source
    /// render as withheld (ACC-SCOPE-64-CONTAINER).
    pub container: bool,
}

impl Snapshot {
    /// The shared time axis under the network plots: the window, then each
    /// quarter of it, in seconds, oldest first. Empty before the first sample.
    pub fn axis_labels(&self) -> Vec<String> {
        if self.update_ms == 0 {
            return Vec::new();
        }
        let window_ms = crate::chart::capacity(self.update_ms) as u64 * self.update_ms;
        (1..=4)
            .rev()
            .map(|quarter| format!("{} s", window_ms * quarter / 4 / 1000))
            .collect()
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
    /// at most 11, so 64 cores make 6 columns, C0-C10 in the first
    /// (ACC-VIEW-48-CORE-BARS).
    pub fn core_rows(&self) -> usize {
        let cores = self.cores.len().min(64);
        let columns = cores.div_ceil(11).max(1);
        cores.div_ceil(columns).max(1)
    }
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
    pub ram_cached: Option<u64>,
    pub ram_percent: Option<f64>,
    /// The memory gauge arc's value: the used history's 1 s EMA, `None`
    /// whenever this tick's raw value is unavailable (ACC-GRAPH-47).
    pub gauge_pct: Option<f64>,
    /// btop `mem.percent["free"]`.
    pub ram_free_percent: Option<f64>,
    /// btop `mem.percent["available"]`.
    pub ram_available_percent: Option<f64>,
    /// btop `mem.percent["cached"]`.
    pub ram_cached_percent: Option<f64>,
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

    /// The used RAM as btop's humanized number and its unit, `("39.7",
    /// "GiB")`, for readouts that set the unit smaller than the value.
    pub fn ram_used_parts(&self) -> Option<(String, String)> {
        let label = bytes(Some(self.ram_used?));
        let (value, unit) = label.split_once(' ')?;
        Some((value.to_string(), unit.to_string()))
    }

    pub fn ram_bar(&self) -> Option<u8> {
        bar(self.ram_percent)
    }

    pub fn ram_free_label(&self) -> String {
        bytes(self.ram_free)
    }

    pub fn ram_available_label(&self) -> String {
        bytes(self.ram_available)
    }

    pub fn ram_cached_label(&self) -> String {
        bytes(self.ram_cached)
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
    /// btop `proc_info::name`, the `comm` value.
    pub name: Option<String>,
    pub user: Option<String>,
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
    pub command: String,
    /// The row graph left of Cpu% (ACC-VIEW-52-PROC-GRAPH); empty without one.
    pub graph: ChartPath,
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
    /// [`gpu`] records where it is unavailable.
    pub process_count: Option<usize>,
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

/// Start the sampler. The returned receiver always holds the latest snapshot;
/// `None` means the first sample has not landed yet. Aborting the returned
/// task drops the sender, which ends every SSE stream.
///
/// ACC-CONF-61-CONFIG-PAGES: a new `config` value applies from the next tick;
/// a new `update_ms` starts a new interval at once, so every client sees the
/// new cadence within two ticks. A GPU sweep in flight finishes on the old one.
pub fn spawn(
    mut config_updates: watch::Receiver<Config>,
) -> (watch::Receiver<Option<Snapshot>>, JoinHandle<()>) {
    let config = config_updates.borrow_and_update().clone();
    // btop's `Shared::init` collects the CPU counters before the runner
    // starts, so its first drawn value is a delta over milliseconds.
    match &config.container {
        Some(container) => cpu::seed_container(&config.proc_root, container, config.update_ms),
        None => cpu::seed(&config.proc_root, config.update_ms),
    }
    // DEF-CPU-44: the container's load average counts its tasks every
    // LOAD_FREQ on its own timer, off the tick path, one count at a time; the
    // first count ran in `seed_container`.
    if let Some(cgroup) = config.container.as_ref().and_then(|c| c.cgroup.clone()) {
        let proc_root = config.proc_root.clone();
        tokio::spawn(async move {
            let start = tokio::time::Instant::now() + cpu::LOAD_FREQ;
            let mut ticker = tokio::time::interval_at(start, cpu::LOAD_FREQ);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let (proc_root, cgroup) = (proc_root.clone(), cgroup.clone());
                let count = move || cpu::count_container_load(&proc_root, &cgroup);
                let _ = tokio::task::spawn_blocking(count).await;
            }
        });
    }
    let (sender, receiver) = watch::channel(None);
    let sampler = tokio::spawn(async move {
        let mut config = Arc::new(config);
        let mut update_ms = config.update_ms;
        let proc_root = config.proc_root.clone();
        let mut mem = mem::MemCollector::default();
        // The first GPU sweep runs before the first tick, as btop's init does;
        // later sweeps run off the tick path (see `gpu`), at most one at a time.
        let mut gpu_latest = gpu::collect(&proc_root, &config);
        let mut gpu_sweep: Option<JoinHandle<GpuSnapshot>> = None;
        let mut ticker = ticker(update_ms);
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                Ok(()) = config_updates.changed() => {
                    config = Arc::new(config_updates.borrow_and_update().clone());
                    if config.update_ms != update_ms {
                        update_ms = config.update_ms;
                        ticker = self::ticker(update_ms);
                    }
                    continue;
                }
            }
            if let Some(sweep) = gpu_sweep.take_if(|sweep| sweep.is_finished())
                && let Ok(gpu) = sweep.await
            {
                gpu_latest = gpu;
            }
            if gpu_sweep.is_none() {
                let config = Arc::clone(&config);
                gpu_sweep = Some(tokio::task::spawn_blocking(move || {
                    gpu::collect(&config.proc_root, &config)
                }));
            }
            let mut gpu = gpu_latest.clone();
            gpu::attach_charts(&proc_root, &mut gpu.devices, update_ms);
            let snapshot = collect(&proc_root, &config, &mut mem, gpu);
            if sender.send(Some(snapshot)).is_err() {
                break;
            }
        }
    });
    (receiver, sampler)
}

/// A sampling interval whose first tick fires at once; a late tick is skipped.
fn ticker(update_ms: u64) -> tokio::time::Interval {
    let mut ticker = tokio::time::interval(Duration::from_millis(update_ms));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ticker
}

fn collect(
    proc_root: &Path,
    config: &Config,
    mem: &mut mem::MemCollector,
    gpu: GpuSnapshot,
) -> Snapshot {
    Snapshot {
        cpu: cpu::collect(proc_root, config),
        mem: mem.collect(proc_root, config),
        net: net::collect(proc_root, config),
        proc: proc::collect(proc_root, config),
        gpu,
        update_ms: config.update_ms,
        container: config.container.is_some(),
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

fn human_with(value: u64, shorten: bool) -> String {
    const UNITS: [&str; 11] = [
        "Byte", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB", "BiB", "GEB",
    ];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_per_core_label_keeps_four_characters_per_value() {
        let cpu = CpuSnapshot {
            load_avg: Some([0.5, 19.98, 25.0]),
            effective_cpus: Some(2.0),
            ..CpuSnapshot::default()
        };
        assert_eq!(cpu.load_per_core_label().as_deref(), Some("0.25 9.99 12.5"));
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
    /// the snapshots within two ticks and sets the next gap.
    #[tokio::test]
    async fn sampler_follows_a_new_update_interval() {
        let mut config = Config::defaults();
        config.proc_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proc");
        config.update_ms = 100;
        let (updates, receiver) = watch::channel(config.clone());
        let (mut snapshots, sampler) = spawn(receiver);
        snapshots.changed().await.expect("first snapshot");
        config.update_ms = 300;
        updates.send_replace(config);
        let mut seen = 0;
        while snapshots
            .borrow()
            .as_ref()
            .map(|snapshot| snapshot.update_ms)
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
        sampler.abort();
    }
}

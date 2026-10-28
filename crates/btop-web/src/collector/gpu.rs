//! GPU collector, mirroring btop 1.3.0's `Gpu::collect` in
//! `src/linux/btop_collect.cpp`.
//!
//! btop 1.3.0 has two Linux GPU backends: NVML for NVIDIA (`Gpu::Nvml`) and
//! ROCm SMI for AMD (`Gpu::Rsmi`). This module calls NVML through the safe
//! `nvml-wrapper` crate, in NVML index order, and maps btop's fields: device
//! name with the `NVIDIA`, `Nvidia`, `(R)` and `(TM)` tokens stripped, GPU and
//! memory utilization percent, graphics and memory clocks in MHz, power usage
//! and power cap in mW, the P-state, the shutdown temperature threshold, GPU
//! temperature in whole degrees Celsius, VRAM in bytes and PCIe TX/RX in KB/s.
//! `power_w` and `power_max_w` carry those milliwatt readings converted to
//! watts, the unit the snapshot field names.
//!
//! btop probes every NVML field once at init and caches the resulting
//! `supported_functions` flags (`Nvml::collect<1>`); later samples refresh
//! only the supported fields, and a call that fails after init leaves the
//! previous value in place. This module does the same. `check_temp` gates the
//! per-sample GPU temperature read, `nvml_measure_pcie_speeds` gates the
//! per-sample PCIe reads (btop still measures PCIe once at init to detect
//! support), and `custom_gpu_name0..5` replace the displayed device name by
//! index. `show_gpu_info` is draw-side only in btop
//! (`btop_draw.cpp:538-540` and `2011-2013`): it decides whether the CPU box
//! repeats GPU data, so it changes no collected value; the web dashboard has
//! its own GPU panel.
//!
//! AMD devices are read through the optional `rocm` feature, which mirrors
//! `Rsmi::init` and `Rsmi::collect<1>/<0>` in `src/linux/btop_collect.cpp`
//! over the safe `rocm_smi_lib` wrapper. The DRM sysfs path stays the default:
//! btop 1.3.0 dlopens `librocm_smi64.so` and skips AMD when it is missing, so
//! the sysfs values are real host data but not btop's source. This host has no
//! AMD GPU.
//!
//! Aggregates reproduce `Gpu::collect` exactly, including its two 1.3.0
//! accumulator quirks: `pwr_usage` (mW) is added to `mem_total`, and
//! `pwr_total` is never accumulated, so `gpu-pwr-total` is always 0.
//!
//! A recorded tree (`BTOP_WEB_PROC_ROOT` other than `/proc`) cannot record
//! NVML, so NVML is skipped there and the fixtures exercise the sysfs path.
//!
//! Period: every sample, `update_ms` (250 ms by default; btop's 2000).
//! The NVML sweep runs off the tick path: the sampler starts one blocking
//! sweep per tick when none is in flight, and each snapshot carries the newest
//! completed sweep. btop collects GPUs inline; here a sweep of three NVIDIA
//! cards takes 0.2-1.3 s, which would hold a 250 ms tick back. The readings are
//! instantaneous, so the values are unchanged. The utilization history takes
//! one sample per tick from that newest sweep ([`attach_charts`]) and draws
//! each device in its own 0-100% lane, [`crate::chart::gpu_lane`]
//! (DEF-GRAPH-51-GPU-LANES).
//!
//! Container (ACC-SCOPE-64-CONTAINER, DEF-GPU-40-CONTAINER-HIDDEN): the
//! devices NVML, ROCm SMI and DRM sysfs list inside a container are the
//! devices passed into it (`NVIDIA_VISIBLE_DEVICES`, `--gpus`, `--device`), so
//! they are the container's own GPUs. Inside a container this collector runs
//! exactly as on the host, with the same sweep, sources and btop values.
//!
//! GPU process count (ACC-VIEW-53-PROC-TILE): btop 1.3.0 has no such value;
//! it is a btop-web addition the owner asked for on 2026-10-27. Source: NVML
//! `running_compute_processes` and `running_graphics_processes` of every
//! device, called in the device loop of the NVML sweep (off the tick).
//! Formula: the number of distinct pids over all those lists. It is
//! unavailable when any call fails, when any device is not NVML (DRM sysfs
//! and ROCm SMI cannot list processes) and on WSL2, where NVML returns
//! success with empty lists while processes hold VRAM.
//!
//! Inside a container NVML reports host pid-namespace pids the container
//! cannot attribute, so the count has its own source there
//! (DEF-VIEW-48-GPU-FACT, owner 2026-10-28): the members of the container's
//! cgroup and every cgroup below it (`cgroup.procs`), less btop-web itself;
//! for each, the targets of `<proc_root>/<pid>/fd/*`. Formula: the number of
//! members holding a GPU node open: `/dev/nvidia<N>` (not `nvidiactl`,
//! `nvidia-uvm` or `nvidia-modeset`), `/dev/dxg` (WSL2, where CUDA opens no
//! `/dev/nvidia*` node), `/dev/kfd` (AMD compute) or `/dev/dri/renderD<N>`.
//! A member that exits mid-sweep holds nothing and is skipped; a member whose
//! fd table cannot be read (another uid, no CAP_SYS_PTRACE) is counted as
//! unreadable and the count renders as a lower bound, `gpu n+`. No readable
//! cgroup makes it unavailable. Period: [`HOLDER_PERIOD`], inside the
//! off-tick sweep. The fd walk uses `nix`'s `Dir` and `readlinkat`, one
//! syscall per fd. The `procfs` crate opens and stats every fd as well, for an
//! open mode the count does not use: 7.5-8.2 against 4.6 us CPU per fd here.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::{OFlag, readlinkat};
use nix::sys::stat::Mode;
use nvml_wrapper::Device as NvmlDevice;
use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{
    Clock, PcieUtilCounter, TemperatureSensor, TemperatureThreshold,
};

use super::{GpuDevice, GpuSnapshot, sys_root};
use crate::chart::{History, gpu_lane};
use crate::config::Config;

/// btop `gpu_info::pwr_max_usage` default, mW.
const DEFAULT_POWER_MAX_MW: u32 = 255_000;
/// btop `gpu_info::temp_max` default, degrees Celsius.
const DEFAULT_TEMPERATURE_MAX_C: u32 = 110;

pub fn collect(proc_root: &Path, config: &Config) -> GpuSnapshot {
    let mut devices = Vec::new();
    let mut power_cap_known = false;
    let mut process_count = None;
    if proc_root == Path::new("/proc")
        && let Some(state) = nvml(config)
    {
        let (nvml_devices, nvml_power_cap, nvml_processes) = state.collect(config);
        devices.extend(nvml_devices);
        power_cap_known = nvml_power_cap;
        process_count = nvml_processes;
    }
    let (amd, amd_power_cap_known) = amd_devices(&sys_root(proc_root), config);
    if !amd.is_empty() {
        // AMD devices cannot list their processes, so no count covers every GPU.
        process_count = None;
    }
    devices.extend(amd);
    apply_custom_names(&mut devices, config);
    let mut process_unreadable = 0;
    if let Some(container) = &config.container {
        // The fd sweep covers AMD holders too, so it replaces the rule above.
        let cgroup = container.cgroup.as_deref().filter(|_| !devices.is_empty());
        let holders = cgroup.and_then(|cgroup| cached_holders(proc_root, cgroup));
        process_count = holders.map(|(count, _)| count);
        process_unreadable = holders.map_or(0, |(_, unreadable)| unreadable);
    }
    GpuSnapshot {
        process_count,
        process_unreadable,
        ..summarize(devices, power_cap_known || amd_power_cap_known)
    }
}

/// DEF-VIEW-48-GPU-FACT: the container holder count's period. One sweep costs
/// about 4.6 us CPU per open fd in the container (27-29 ms over 5,800-6,200
/// fds), against 8.5-11 ms CPU per 250 ms tick for the whole sampler, so a
/// sweep every tick would more than double the dashboard's load
/// (ACC-PERF-16). Every 5 s it added 10.1-17.0 percent CPU per tick at
/// 5,400-7,100 container fds; the cost grows with that fd count.
const HOLDER_PERIOD: Duration = Duration::from_secs(5);

/// A holder sweep's result: the holders and the unreadable members, or
/// `None` when the cgroup cannot be read ([`gpu_holders`]).
type Holders = Option<(usize, usize)>;

/// The last holder sweep per collected tree and when it ran.
static HOLDERS: Mutex<BTreeMap<PathBuf, (Instant, Holders)>> = Mutex::new(BTreeMap::new());

/// The holder count, swept again when the last sweep is older than
/// [`HOLDER_PERIOD`]; `gpu::collect` runs once per tick at most, so the period
/// is never shorter than `update_ms`.
fn cached_holders(proc_root: &Path, cgroup: &Path) -> Holders {
    let mut cache = HOLDERS.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((swept, holders)) = cache.get(proc_root)
        && swept.elapsed() < HOLDER_PERIOD
    {
        return *holders;
    }
    let holders = gpu_holders(proc_root, cgroup, std::process::id() as i32);
    cache.insert(proc_root.to_path_buf(), (Instant::now(), holders));
    holders
}

/// One holder sweep (see the module docs): the members of `cgroup` and every
/// cgroup below it, except `own_pid`, that hold a GPU node open, and the
/// members whose fd table could not be read. `None` when `cgroup.procs`
/// cannot be read.
fn gpu_holders(proc_root: &Path, cgroup: &Path, own_pid: i32) -> Holders {
    let (mut holders, mut unreadable) = (0, 0);
    for pid in super::proc::cgroup_members(cgroup, "cgroup.procs")? {
        if pid == own_pid {
            continue;
        }
        let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC;
        match Dir::open(&proc_root.join(format!("{pid}/fd")), flags, Mode::empty()) {
            Ok(mut fds) => {
                // `iter` borrows `fds`, which `readlinkat` needs, so list the names first.
                let names: Vec<_> = fds
                    .iter()
                    .flatten()
                    .map(|entry| entry.file_name().to_owned())
                    .filter(|name| !matches!(name.to_bytes(), b"." | b".."))
                    .collect();
                if names.iter().any(|name| {
                    readlinkat(&fds, name.as_c_str())
                        .is_ok_and(|target| is_gpu_node(Path::new(&target)))
                }) {
                    holders += 1;
                }
            }
            // The process exited after `cgroup.procs` was read.
            Err(Errno::ENOENT | Errno::ESRCH) => {}
            Err(_) => unreadable += 1,
        }
    }
    Some((holders, unreadable))
}

/// A GPU device node an fd can point to (see the module docs).
fn is_gpu_node(target: &Path) -> bool {
    let numbered = |prefix: &str| {
        target
            .to_str()
            .and_then(|target| target.strip_prefix(prefix))
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
    };
    target == Path::new("/dev/dxg")
        || target == Path::new("/dev/kfd")
        || numbered("/dev/nvidia")
        || numbered("/dev/dri/renderD")
}

/// The number of distinct pids over every process list; `None` when any list
/// could not be read.
fn distinct_pids<E>(lists: impl IntoIterator<Item = Result<Vec<u32>, E>>) -> Option<usize> {
    let mut pids = HashSet::new();
    for list in lists {
        pids.extend(list.ok()?);
    }
    Some(pids.len())
}

/// NVML on WSL2 returns success with empty process lists while processes hold
/// VRAM, so a count there would read 0 falsely. `osrelease` is
/// `/proc/sys/kernel/osrelease`.
fn lists_processes(osrelease: &str) -> bool {
    !osrelease.to_ascii_lowercase().contains("microsoft")
}

/// btop's draw-time `custom_gpu_nameN` override (`btop_draw.cpp:2115`): by
/// panel index, used only when non-empty. An index past the detected devices
/// has no effect.
fn apply_custom_names(devices: &mut [GpuDevice], config: &Config) {
    let names = [
        &config.custom_gpu_name0,
        &config.custom_gpu_name1,
        &config.custom_gpu_name2,
        &config.custom_gpu_name3,
        &config.custom_gpu_name4,
        &config.custom_gpu_name5,
    ];
    for (device, name) in devices.iter_mut().zip(names) {
        if !name.is_empty() {
            device.name = name.clone();
        }
    }
}

/// btop's GPU graph history: one utilization sample per device per tick. A
/// device that disappears loses its history; a new device starts empty.
pub fn attach_charts(proc_root: &Path, devices: &mut [GpuDevice], update_ms: u64) {
    let mut states = HISTORY.lock().unwrap_or_else(|error| error.into_inner());
    let histories = states.entry(proc_root.to_path_buf()).or_default();
    histories.truncate(devices.len());
    while histories.len() < devices.len() {
        histories.push(History::new());
    }
    // DEF-GRAPH-51-GPU-LANES: each device draws in its own 0-100% lane.
    let lane = gpu_lane(devices.len());
    for (history, device) in histories.iter_mut().zip(devices.iter_mut()) {
        history.push(device.utilization_pct, update_ms);
        device.chart = history.chart(lane, 0.0, 100.0);
    }
}

/// btop's per-device `gpu_info_supported` (`btop_shared.hpp:112`): every flag
/// starts true and an init-time probe failure turns it off for the process.
struct Supported {
    gpu_utilization: bool,
    mem_utilization: bool,
    gpu_clock: bool,
    mem_clock: bool,
    pwr_usage: bool,
    pwr_state: bool,
    temp_info: bool,
    mem_total: bool,
    mem_used: bool,
    pcie_txrx: bool,
}

impl Default for Supported {
    fn default() -> Self {
        Self {
            gpu_utilization: true,
            mem_utilization: true,
            gpu_clock: true,
            mem_clock: true,
            pwr_usage: true,
            pwr_state: true,
            temp_info: true,
            mem_total: true,
            mem_used: true,
            pcie_txrx: true,
        }
    }
}

impl Supported {
    /// btop's failed-handle path (`btop_collect.cpp:1050`): every flag off.
    fn none() -> Self {
        Self {
            gpu_utilization: false,
            mem_utilization: false,
            gpu_clock: false,
            mem_clock: false,
            pwr_usage: false,
            pwr_state: false,
            temp_info: false,
            mem_total: false,
            mem_used: false,
            pcie_txrx: false,
        }
    }
}

/// NVML state initialized once, as btop's `Nvml::init` does: the device
/// count, the init-time support flags and the latest sample of every device.
struct NvmlState {
    nvml: Nvml,
    /// One entry per NVML index, as btop's resized `gpus` vector. A handle
    /// that cannot be opened at init keeps its slot with every flag off.
    devices: Mutex<Vec<NvmlDeviceState>>,
    /// NVML's process lists are trusted on this kernel (not WSL2).
    lists_processes: bool,
}

/// btop `gpu_info` plus `gpu_info_supported` for one device: the flags
/// probed at init, the last successful value of each field, and the
/// init-time power maximum.
struct NvmlDeviceState {
    supported: Supported,
    sample: GpuDevice,
    power_max_mw: u32,
    power_max_known: bool,
}

static NVML: OnceLock<Option<NvmlState>> = OnceLock::new();

/// Per-device utilization histories, keyed by the collected tree.
static HISTORY: Mutex<BTreeMap<PathBuf, Vec<History>>> = Mutex::new(BTreeMap::new());

/// btop initializes NVML once at startup and shuts it down at exit. The
/// config is process-wide, so only the first call's values reach init.
fn nvml(config: &Config) -> Option<&'static NvmlState> {
    NVML.get_or_init(|| init_nvml(config)).as_ref()
}

fn init_nvml(config: &Config) -> Option<NvmlState> {
    let nvml = Nvml::init().ok()?;
    let count = nvml.device_count().ok()?;
    if count == 0 {
        return None;
    }
    let mut devices = Vec::with_capacity(count as usize);
    for index in 0..count {
        devices.push(match nvml.device_by_index(index) {
            Ok(device) => NvmlDeviceState::init(&device, config),
            Err(_) => NvmlDeviceState {
                supported: Supported::none(),
                sample: GpuDevice::default(),
                power_max_mw: DEFAULT_POWER_MAX_MW,
                power_max_known: false,
            },
        });
    }
    Some(NvmlState {
        nvml,
        devices: Mutex::new(devices),
        lists_processes: std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|osrelease| lists_processes(&osrelease)),
    })
}

impl NvmlDeviceState {
    /// btop's `Nvml::collect<1>`: read the once-only fields (name, power
    /// maximum, shutdown threshold) and probe every per-sample field,
    /// clearing the support flag of each field whose call fails.
    fn init(device: &NvmlDevice<'_>, config: &Config) -> Self {
        let power_max_mw = device.power_management_limit().ok();
        let temperature_max_c = device
            .temperature_threshold(TemperatureThreshold::Shutdown)
            .ok();
        let mut supported = Supported::default();
        let mut sample = GpuDevice {
            name: device
                .name()
                .map(|name| normalize_nvml_name(&name))
                .unwrap_or_default(),
            power_max_w: Some(f64::from(power_max_mw.unwrap_or(DEFAULT_POWER_MAX_MW)) / 1000.0),
            temperature_max_c: Some(f64::from(
                temperature_max_c.unwrap_or(DEFAULT_TEMPERATURE_MAX_C),
            )),
            ..GpuDevice::default()
        };

        // PCIe is always measured once at init to detect support, even when
        // `nvml_measure_pcie_speeds` is off (btop's `or is_init`).
        match device.pcie_throughput(PcieUtilCounter::Send) {
            Ok(tx) => sample.pcie_tx = Some(u64::from(tx)),
            Err(_) => supported.pcie_txrx = false,
        }
        // btop clears the flag only on a failed TX call; a failed RX read
        // leaves the flag on with no value.
        if let Ok(rx) = device.pcie_throughput(PcieUtilCounter::Receive) {
            sample.pcie_rx = Some(u64::from(rx));
        }
        match device.utilization_rates() {
            Ok(utilization) => {
                sample.utilization_pct = Some(f64::from(utilization.gpu));
                sample.memory_utilization_pct = Some(f64::from(utilization.memory));
            }
            Err(_) => {
                supported.gpu_utilization = false;
                supported.mem_utilization = false;
            }
        }
        match device.clock_info(Clock::Graphics) {
            Ok(clock) => sample.gpu_clock_mhz = Some(u64::from(clock)),
            Err(_) => supported.gpu_clock = false,
        }
        match device.clock_info(Clock::Memory) {
            Ok(clock) => sample.memory_clock_mhz = Some(u64::from(clock)),
            Err(_) => supported.mem_clock = false,
        }
        match device.power_usage() {
            Ok(milliwatts) => set_power(
                &mut sample,
                milliwatts,
                power_max_mw.unwrap_or(DEFAULT_POWER_MAX_MW),
            ),
            Err(_) => supported.pwr_usage = false,
        }
        match device.performance_state() {
            Ok(state) => sample.pstate = Some(state.as_c() as i32),
            Err(_) => supported.pwr_state = false,
        }
        // btop reads the shutdown threshold above regardless, and probes the
        // temperature itself only when `check_temp` is on.
        if config.check_temp {
            match device.temperature(TemperatureSensor::Gpu) {
                Ok(temperature) => sample.temperature_c = Some(f64::from(temperature)),
                Err(_) => supported.temp_info = false,
            }
        }
        match device.memory_info() {
            Ok(memory) => {
                sample.vram_used = Some(memory.total - memory.free);
                sample.vram_total = Some(memory.total);
            }
            Err(_) => {
                supported.mem_total = false;
                supported.mem_used = false;
            }
        }
        Self {
            supported,
            sample,
            power_max_mw: power_max_mw.unwrap_or(DEFAULT_POWER_MAX_MW),
            power_max_known: power_max_mw.is_some(),
        }
    }

    /// btop's `Nvml::collect<0>`: refresh only the supported fields and keep
    /// the last sample for a field whose call fails.
    fn refresh(&mut self, device: &NvmlDevice<'_>, config: &Config) {
        let sample = &mut self.sample;
        if self.supported.pcie_txrx && config.nvml_measure_pcie_speeds {
            keep_last(
                &mut sample.pcie_tx,
                device
                    .pcie_throughput(PcieUtilCounter::Send)
                    .ok()
                    .map(u64::from),
            );
            keep_last(
                &mut sample.pcie_rx,
                device
                    .pcie_throughput(PcieUtilCounter::Receive)
                    .ok()
                    .map(u64::from),
            );
        }
        if self.supported.gpu_utilization
            && let Ok(utilization) = device.utilization_rates()
        {
            sample.utilization_pct = Some(f64::from(utilization.gpu));
            sample.memory_utilization_pct = Some(f64::from(utilization.memory));
        }
        if self.supported.gpu_clock {
            keep_last(
                &mut sample.gpu_clock_mhz,
                device.clock_info(Clock::Graphics).ok().map(u64::from),
            );
        }
        if self.supported.mem_clock {
            keep_last(
                &mut sample.memory_clock_mhz,
                device.clock_info(Clock::Memory).ok().map(u64::from),
            );
        }
        if self.supported.pwr_usage
            && let Ok(milliwatts) = device.power_usage()
        {
            set_power(sample, milliwatts, self.power_max_mw);
        }
        if self.supported.pwr_state {
            keep_last(
                &mut sample.pstate,
                device
                    .performance_state()
                    .ok()
                    .map(|state| state.as_c() as i32),
            );
        }
        if self.supported.temp_info && config.check_temp {
            keep_last(
                &mut sample.temperature_c,
                device
                    .temperature(TemperatureSensor::Gpu)
                    .ok()
                    .map(f64::from),
            );
        }
        if self.supported.mem_total
            && let Ok(memory) = device.memory_info()
        {
            sample.vram_used = Some(memory.total - memory.free);
            sample.vram_total = Some(memory.total);
        }
    }
}

impl NvmlState {
    /// btop's `Gpu::collect` NVML half. The bool matches
    /// `gpu_pwr_total_max != 0`, the condition for pushing `gpu-pwr-total`.
    /// The count is the GPU process count (see the module docs).
    fn collect(&self, config: &Config) -> (Vec<GpuDevice>, bool, Option<usize>) {
        let mut states = match self.devices.lock() {
            Ok(states) => states,
            Err(poisoned) => poisoned.into_inner(),
        };
        // A container counts its own holders instead (`gpu_holders`).
        let count_processes = self.lists_processes && config.container.is_none();
        let mut process_lists = Vec::new();
        let mut devices = Vec::with_capacity(states.len());
        for (index, state) in states.iter_mut().enumerate() {
            match self.nvml.device_by_index(index as u32) {
                Ok(device) => {
                    state.refresh(&device, config);
                    if count_processes {
                        for list in [
                            device.running_compute_processes(),
                            device.running_graphics_processes(),
                        ] {
                            process_lists.push(list.map(|processes| {
                                processes.into_iter().map(|process| process.pid).collect()
                            }));
                        }
                    }
                }
                // A handle that cannot be opened now keeps the last sample, as
                // btop's init-time handle would; its processes are unknown.
                Err(error) => process_lists.push(Err(error)),
            }
            devices.push(state.sample.clone());
        }
        let power_cap_known = states
            .iter()
            .any(|state| state.power_max_known && state.power_max_mw != 0);
        let process_count = count_processes
            .then(|| distinct_pids(process_lists))
            .flatten();
        (devices, power_cap_known, process_count)
    }
}

/// btop's `pwr_usage` mapping: milliwatts to watts, plus the per-device
/// power percent clamped to 0..=100.
fn set_power(sample: &mut GpuDevice, milliwatts: u32, maximum_mw: u32) {
    sample.power_w = Some(f64::from(milliwatts) / 1000.0);
    sample.power_percent = Some(
        (f64::from(milliwatts) * 100.0 / f64::from(maximum_mw))
            .round()
            .clamp(0.0, 100.0),
    );
}

/// A failed read after init leaves the previous sample in place; only a
/// successful read replaces it.
fn keep_last<T>(slot: &mut Option<T>, reading: Option<T>) {
    if let Some(value) = reading {
        *slot = Some(value);
    }
}

/// btop's `Gpu::Nvml::init` name normalization: strip the brand tokens, then
/// trim the ends.
fn normalize_nvml_name(name: &str) -> String {
    let mut name = name.to_string();
    for brand in ["NVIDIA", "Nvidia", "(R)", "(TM)"] {
        name = name.replace(brand, "");
    }
    name.trim().to_string()
}

/// btop's `Rsmi::collect` readings, one backend method per field. A method
/// returns `None` when the device does not support the call or the call
/// fails, which the mapping layer renders as unavailable.
#[cfg(any(test, feature = "rocm"))]
trait RsmiBackend {
    /// `rsmi_dev_name_get`; btop `gpu_names`.
    fn name(&mut self) -> Option<String>;
    /// `rsmi_dev_power_cap_get`, microwatts; btop `pwr_max_usage`.
    fn power_cap_uw(&mut self) -> Option<u64>;
    /// `rsmi_dev_temp_metric_get` EDGE/MAX, degrees Celsius; btop `temp_max`.
    fn temperature_max_c(&mut self) -> Option<f64>;
    /// `rsmi_dev_busy_percent_get`; btop `gpu-totals`.
    fn utilization_pct(&mut self) -> Option<u32>;
    /// `rsmi_dev_memory_busy_percent_get`; btop `mem_utilization_percent`.
    fn memory_utilization_pct(&mut self) -> Option<u32>;
    /// `rsmi_dev_gpu_clk_freq_get` SYS, hertz; btop `gpu_clock_speed`.
    fn gpu_clock_hz(&mut self) -> Option<u64>;
    /// `rsmi_dev_gpu_clk_freq_get` MEM, hertz; btop `mem_clock_speed`.
    fn memory_clock_hz(&mut self) -> Option<u64>;
    /// `rsmi_dev_power_ave_get`, microwatts; btop `pwr_usage`.
    fn power_ave_uw(&mut self) -> Option<u64>;
    /// `rsmi_dev_temp_metric_get` EDGE/CURRENT, degrees Celsius; btop `temp`.
    fn temperature_c(&mut self) -> Option<f64>;
    /// `rsmi_dev_memory_total_get` VRAM, bytes; btop `mem_total`.
    fn vram_total(&mut self) -> Option<u64>;
    /// `rsmi_dev_memory_usage_get` VRAM, bytes; btop `mem_used`.
    fn vram_used(&mut self) -> Option<u64>;
    /// `rsmi_dev_pci_throughput_get` sent, btop `pcie_tx`.
    fn pcie_tx(&mut self) -> Option<u64>;
    /// `rsmi_dev_pci_throughput_get` received, btop `pcie_rx`.
    fn pcie_rx(&mut self) -> Option<u64>;
}

/// btop's `Rsmi::collect` mapping: backend readings to the page's fields in
/// btop's units. The bool records `gpu_pwr_total_max` gaining this device's
/// cap, btop's condition for reporting `gpu-pwr-total`.
#[cfg(any(test, feature = "rocm"))]
fn rsmi_device(backend: &mut impl RsmiBackend, check_temp: bool) -> (GpuDevice, bool) {
    let power_cap_mw = backend.power_cap_uw().map(|microwatts| microwatts / 1000);
    let mut device = GpuDevice {
        name: backend.name().unwrap_or_default(),
        utilization_pct: backend.utilization_pct().map(f64::from),
        memory_utilization_pct: backend.memory_utilization_pct().map(f64::from),
        vram_total: backend.vram_total(),
        vram_used: backend.vram_used(),
        power_max_w: power_cap_mw.map(|milliwatts| milliwatts as f64 / 1000.0),
        temperature_c: if check_temp {
            backend.temperature_c()
        } else {
            None
        },
        temperature_max_c: backend.temperature_max_c(),
        gpu_clock_mhz: backend.gpu_clock_hz().map(|hertz| hertz / 1_000_000),
        memory_clock_mhz: backend.memory_clock_hz().map(|hertz| hertz / 1_000_000),
        pcie_tx: backend.pcie_tx(),
        pcie_rx: backend.pcie_rx(),
        ..GpuDevice::default()
    };
    // Power draw and its percent need both readings; without a cap the fields
    // stay unavailable instead of dividing by btop's 255 W default.
    if let (Some(power_uw), Some(cap_mw)) = (backend.power_ave_uw(), power_cap_mw) {
        set_power(&mut device, (power_uw / 1000) as u32, cap_mw as u32);
    }
    (device, power_cap_mw.is_some())
}

/// btop `Rsmi::init` and `Rsmi::collect<1>/<0>` over `rocm_smi_lib` 0.3.2:
/// library calls only, no mapping.
#[cfg(feature = "rocm")]
mod rocm {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use rocm_smi_lib::{RocmSmi, RsmiClkType, RsmiTemperatureMetric, RsmiTemperatureType};

    use super::{GpuDevice, RsmiBackend, rsmi_device};

    /// btop initializes ROCm SMI once; every `rsmi_init` counts as a
    /// reference the library holds.
    static SMI: OnceLock<Option<Mutex<RocmSmi>>> = OnceLock::new();

    fn smi() -> Option<MutexGuard<'static, RocmSmi>> {
        SMI.get_or_init(|| RocmSmi::init().ok().map(Mutex::new))
            .as_ref()
            .map(|smi| smi.lock().unwrap_or_else(|error| error.into_inner()))
    }

    /// btop's `Rsmi::init` device list; `None` when the library reports no
    /// AMD device, where btop detects no AMD GPU.
    pub(super) fn devices(check_temp: bool) -> Option<(Vec<GpuDevice>, bool)> {
        let mut smi = smi()?;
        let count = smi.get_device_count();
        if count == 0 {
            return None;
        }
        let mut devices = Vec::with_capacity(count as usize);
        let mut power_cap_known = false;
        for id in 0..count {
            let mut backend = Backend { smi: &mut smi, id };
            let (device, cap_known) = rsmi_device(&mut backend, check_temp);
            power_cap_known |= cap_known;
            devices.push(device);
        }
        Some((devices, power_cap_known))
    }

    struct Backend<'a> {
        smi: &'a mut RocmSmi,
        id: u32,
    }

    impl RsmiBackend for Backend<'_> {
        fn name(&mut self) -> Option<String> {
            self.smi.get_device_identifiers(self.id).ok()?.name.ok()
        }

        fn power_cap_uw(&mut self) -> Option<u64> {
            Some(
                self.smi
                    .get_device_power_data(self.id)
                    .ok()?
                    .default_power_cap,
            )
        }

        fn temperature_max_c(&mut self) -> Option<f64> {
            self.smi
                .get_device_temperature_metric(
                    self.id,
                    RsmiTemperatureType::Edge,
                    RsmiTemperatureMetric::Max,
                )
                .ok()
        }

        fn utilization_pct(&mut self) -> Option<u32> {
            self.smi.get_device_busy_percent(self.id).ok()
        }

        fn memory_utilization_pct(&mut self) -> Option<u32> {
            Some(self.smi.get_device_memory_data(self.id).ok()?.busy_percent)
        }

        fn gpu_clock_hz(&mut self) -> Option<u64> {
            Some(
                self.smi
                    .get_device_frequency(self.id, RsmiClkType::RsmiClkTypeSys)
                    .ok()?
                    .current,
            )
        }

        fn memory_clock_hz(&mut self) -> Option<u64> {
            Some(
                self.smi
                    .get_device_frequency(self.id, RsmiClkType::RsmiClkTypeMem)
                    .ok()?
                    .current,
            )
        }

        fn power_ave_uw(&mut self) -> Option<u64> {
            Some(self.smi.get_device_power_data(self.id).ok()?.current_power)
        }

        fn temperature_c(&mut self) -> Option<f64> {
            self.smi
                .get_device_temperature_metric(
                    self.id,
                    RsmiTemperatureType::Edge,
                    RsmiTemperatureMetric::Current,
                )
                .ok()
        }

        fn vram_total(&mut self) -> Option<u64> {
            Some(self.smi.get_device_memory_data(self.id).ok()?.vram_total)
        }

        fn vram_used(&mut self) -> Option<u64> {
            Some(self.smi.get_device_memory_data(self.id).ok()?.vram_used)
        }

        fn pcie_tx(&mut self) -> Option<u64> {
            Some(self.smi.get_device_pcie_data(self.id).ok()?.pkg_sent)
        }

        fn pcie_rx(&mut self) -> Option<u64> {
            Some(self.smi.get_device_pcie_data(self.id).ok()?.pkg_recived)
        }
    }
}

/// The AMD device list. With the `rocm` feature the ROCm SMI backend is tried
/// first, as btop's `Rsmi::init` does; a host where it does not initialize
/// keeps the DRM sysfs reader.
fn amd_devices(sys: &Path, config: &Config) -> (Vec<GpuDevice>, bool) {
    #[cfg(feature = "rocm")]
    if let Some(devices) = rocm::devices(config.check_temp) {
        return devices;
    }
    (amd_sysfs_devices(sys, config.check_temp), false)
}

/// AMD devices through DRM sysfs, after every NVML device. A card is listed
/// when `vendor` is `0x1002`, the `driver` link resolves to `amdgpu` and at
/// least one signal is readable. `check_temp` gates the temperature value
/// only, so a card stays listed when it is the only signal.
fn amd_sysfs_devices(sys: &Path, check_temp: bool) -> Vec<GpuDevice> {
    let Ok(entries) = std::fs::read_dir(sys.join("class/drm")) else {
        return Vec::new();
    };
    let mut cards: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| is_card_node(&name.to_string_lossy()))
        })
        .collect();
    cards.sort();

    let mut devices = Vec::new();
    for card in cards {
        let device = card.join("device");
        if read_trimmed(&device.join("vendor")).as_deref() != Some("0x1002") {
            continue;
        }
        let driver = std::fs::read_link(device.join("driver")).ok();
        if driver
            .as_deref()
            .and_then(Path::file_name)
            .is_none_or(|name| name.to_string_lossy() != "amdgpu")
        {
            continue;
        }

        let busy = read_u64(&device.join("gpu_busy_percent")).map(|value| value.min(100) as f64);
        let vram_total = read_u64(&device.join("mem_info_vram_total"));
        let vram_used = read_u64(&device.join("mem_info_vram_used"));
        let hwmon = find_hwmon(&device);
        let temperature_raw = hwmon
            .as_deref()
            .and_then(|path| read_u64(&path.join("temp1_input")));
        let power = hwmon.as_deref().and_then(|path| {
            let microwatts = read_u64(&path.join("power1_average"))
                .or_else(|| read_u64(&path.join("power1_input")))?;
            Some(microwatts as f64 / 1_000_000.0)
        });
        let frequency = hwmon
            .as_deref()
            .and_then(|path| read_u64(&path.join("freq1_input")))
            .map(|kilohertz| kilohertz / 1_000_000);

        if busy.is_none()
            && vram_total.is_none()
            && temperature_raw.is_none()
            && power.is_none()
            && frequency.is_none()
        {
            continue;
        }
        let temperature = check_temp
            .then_some(temperature_raw)
            .flatten()
            .map(|value| value as f64 / 1000.0);
        let id = read_trimmed(&device.join("device"))
            .and_then(|text| u32::from_str_radix(text.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        devices.push(GpuDevice {
            name: format!("AMD GPU (1002:{id:04x})"),
            utilization_pct: busy,
            memory_utilization_pct: None,
            vram_used: vram_total.and(vram_used),
            vram_total,
            power_w: power,
            power_max_w: None,
            power_percent: None,
            temperature_c: temperature,
            temperature_max_c: None,
            gpu_clock_mhz: frequency,
            memory_clock_mhz: None,
            pstate: None,
            pcie_tx: None,
            pcie_rx: None,
            ..GpuDevice::default()
        });
    }
    devices
}

/// btop's `Gpu::collect` average block. The utilization sum skips unsupported
/// devices but the divisor stays the full device count, `mem_total` gains the
/// mW of power usage, and `pwr_total` is never accumulated.
fn summarize(devices: Vec<GpuDevice>, power_cap_known: bool) -> GpuSnapshot {
    if devices.is_empty() {
        return GpuSnapshot::default();
    }
    let mut utilization = 0_i64;
    let mut vram_used = 0_i64;
    let mut vram_total = 0_i64;
    for device in &devices {
        if let Some(value) = device.utilization_pct {
            utilization += value as i64;
        }
        if let Some(value) = device.vram_used {
            vram_used += value as i64;
        }
        if let Some(value) = device.vram_total {
            vram_total += value as i64;
        }
        if let Some(watts) = device.power_w {
            vram_total += (watts * 1000.0) as i64;
        }
    }
    GpuSnapshot {
        average_utilization_pct: Some((utilization / devices.len() as i64) as f64),
        vram_percent: (vram_total != 0).then(|| (vram_used / vram_total) as f64),
        power_percent: power_cap_known.then_some(0.0),
        devices,
        ..GpuSnapshot::default()
    }
}

/// A DRM card node: `card` followed only by digits.
fn is_card_node(name: &str) -> bool {
    name.strip_prefix("card").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// The first hwmon directory under the device, sorted.
fn find_hwmon(device: &Path) -> Option<PathBuf> {
    let mut directories: Vec<PathBuf> = std::fs::read_dir(device.join("hwmon"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    directories.sort();
    directories.into_iter().next()
}

fn read_u64(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative)
    }

    fn config() -> Config {
        Config::load(None).expect("embedded defaults")
    }

    #[test]
    fn card_nodes_exclude_connectors_and_render_nodes() {
        assert!(is_card_node("card0"));
        assert!(is_card_node("card12"));
        assert!(!is_card_node("card0-DP-1"));
        assert!(!is_card_node("renderD128"));
        assert!(!is_card_node("card"));
    }

    #[test]
    fn nvml_names_strip_the_same_brand_tokens_as_btop() {
        assert_eq!(
            normalize_nvml_name("NVIDIA RTX PRO 4000 Blackwell"),
            "RTX PRO 4000 Blackwell"
        );
        assert_eq!(
            normalize_nvml_name("Nvidia GeForce (R) RTX (TM)"),
            "GeForce  RTX"
        );
    }

    #[test]
    fn supported_flags_start_on_and_a_failed_handle_turns_them_off() {
        let supported = Supported::default();
        assert!(supported.gpu_utilization);
        assert!(supported.mem_utilization);
        assert!(supported.gpu_clock);
        assert!(supported.mem_clock);
        assert!(supported.pwr_usage);
        assert!(supported.pwr_state);
        assert!(supported.temp_info);
        assert!(supported.mem_total);
        assert!(supported.mem_used);
        assert!(supported.pcie_txrx);

        let none = Supported::none();
        assert!(!none.gpu_utilization);
        assert!(!none.mem_utilization);
        assert!(!none.gpu_clock);
        assert!(!none.mem_clock);
        assert!(!none.pwr_usage);
        assert!(!none.pwr_state);
        assert!(!none.temp_info);
        assert!(!none.mem_total);
        assert!(!none.mem_used);
        assert!(!none.pcie_txrx);
    }

    #[test]
    fn a_failed_read_keeps_the_last_sample() {
        let mut slot = Some(41);
        keep_last(&mut slot, None);
        assert_eq!(slot, Some(41));
        keep_last(&mut slot, Some(42));
        assert_eq!(slot, Some(42));
        let mut empty: Option<i32> = None;
        keep_last(&mut empty, None);
        assert_eq!(empty, None);
    }

    #[test]
    fn custom_gpu_names_override_by_index_and_ignore_empty_names() {
        let mut config = config();
        config.custom_gpu_name0 = String::from("first");
        config.custom_gpu_name5 = String::from("sixth");
        let mut devices = vec![
            GpuDevice {
                name: String::from("device 0"),
                ..GpuDevice::default()
            },
            GpuDevice {
                name: String::from("device 1"),
                ..GpuDevice::default()
            },
            GpuDevice {
                name: String::from("device 2"),
                ..GpuDevice::default()
            },
            GpuDevice {
                name: String::from("device 3"),
                ..GpuDevice::default()
            },
            GpuDevice {
                name: String::from("device 4"),
                ..GpuDevice::default()
            },
            GpuDevice {
                name: String::from("device 5"),
                ..GpuDevice::default()
            },
            GpuDevice {
                name: String::from("device 6"),
                ..GpuDevice::default()
            },
        ];
        apply_custom_names(&mut devices, &config);
        assert_eq!(devices[0].name, "first");
        assert_eq!(devices[1].name, "device 1");
        assert_eq!(devices[5].name, "sixth");
        assert_eq!(devices[6].name, "device 6");
    }

    #[test]
    fn sysfs_reads_the_recorded_amd_card() {
        let devices = amd_sysfs_devices(&fixture("gpu/sys"), true);
        assert_eq!(devices.len(), 1, "one readable amdgpu card: {devices:#?}");
        let device = &devices[0];
        assert_eq!(device.name, "AMD GPU (1002:73bf)");
        assert_eq!(device.utilization_pct, Some(37.0));
        assert_eq!(device.vram_used, Some(1_073_741_824));
        assert_eq!(device.vram_total, Some(17_163_091_968));
        assert_eq!(device.power_w, Some(88.0));
        assert_eq!(device.temperature_c, Some(52.0));
        assert_eq!(device.gpu_clock_mhz, Some(2_100));
    }

    #[test]
    fn check_temp_off_leaves_gpu_temperature_unavailable() {
        let devices = amd_sysfs_devices(&fixture("gpu/sys"), false);
        assert_eq!(devices.len(), 1, "card stays listed: {devices:#?}");
        assert_eq!(devices[0].temperature_c, None);
        assert_eq!(devices[0].power_w, Some(88.0));
    }

    #[test]
    fn sysfs_skips_non_amd_non_amdgpu_and_signal_less_cards() {
        let devices = amd_sysfs_devices(&fixture("gpu/sys"), true);
        assert!(
            devices
                .iter()
                .all(|device| device.name.starts_with("AMD GPU (1002:")),
            "card1 is NVIDIA, card2 has no signals, card3 uses radeon: {devices:#?}"
        );
    }

    #[derive(Default)]
    struct FakeRsmi {
        name: Option<String>,
        power_cap_uw: Option<u64>,
        temperature_max_c: Option<f64>,
        utilization_pct: Option<u32>,
        memory_utilization_pct: Option<u32>,
        gpu_clock_hz: Option<u64>,
        memory_clock_hz: Option<u64>,
        power_ave_uw: Option<u64>,
        temperature_c: Option<f64>,
        vram_total: Option<u64>,
        vram_used: Option<u64>,
        pcie_tx: Option<u64>,
        pcie_rx: Option<u64>,
    }

    impl RsmiBackend for FakeRsmi {
        fn name(&mut self) -> Option<String> {
            self.name.clone()
        }

        fn power_cap_uw(&mut self) -> Option<u64> {
            self.power_cap_uw
        }

        fn temperature_max_c(&mut self) -> Option<f64> {
            self.temperature_max_c
        }

        fn utilization_pct(&mut self) -> Option<u32> {
            self.utilization_pct
        }

        fn memory_utilization_pct(&mut self) -> Option<u32> {
            self.memory_utilization_pct
        }

        fn gpu_clock_hz(&mut self) -> Option<u64> {
            self.gpu_clock_hz
        }

        fn memory_clock_hz(&mut self) -> Option<u64> {
            self.memory_clock_hz
        }

        fn power_ave_uw(&mut self) -> Option<u64> {
            self.power_ave_uw
        }

        fn temperature_c(&mut self) -> Option<f64> {
            self.temperature_c
        }

        fn vram_total(&mut self) -> Option<u64> {
            self.vram_total
        }

        fn vram_used(&mut self) -> Option<u64> {
            self.vram_used
        }

        fn pcie_tx(&mut self) -> Option<u64> {
            self.pcie_tx
        }

        fn pcie_rx(&mut self) -> Option<u64> {
            self.pcie_rx
        }
    }

    /// Raw readings of an AMD device in the units ROCm SMI returns them.
    fn full_rsmi() -> FakeRsmi {
        FakeRsmi {
            name: Some(String::from("AMD Radeon RX 7900 XTX")),
            power_cap_uw: Some(300_000_000),
            temperature_max_c: Some(110.0),
            utilization_pct: Some(37),
            memory_utilization_pct: Some(21),
            gpu_clock_hz: Some(2_100_000_000),
            memory_clock_hz: Some(1_250_000_000),
            power_ave_uw: Some(88_000_000),
            temperature_c: Some(52.0),
            vram_total: Some(25_769_803_776),
            vram_used: Some(1_073_741_824),
            pcie_tx: Some(16_384),
            pcie_rx: Some(8_192),
        }
    }

    #[test]
    fn a_full_rsmi_device_maps_every_btop_field() {
        let (device, power_cap_known) = rsmi_device(&mut full_rsmi(), true);
        assert!(power_cap_known);
        assert_eq!(device.name, "AMD Radeon RX 7900 XTX");
        assert_eq!(device.utilization_pct, Some(37.0));
        assert_eq!(device.memory_utilization_pct, Some(21.0));
        assert_eq!(device.vram_total, Some(25_769_803_776));
        assert_eq!(device.vram_used, Some(1_073_741_824));
        assert_eq!(device.power_max_w, Some(300.0));
        assert_eq!(device.power_w, Some(88.0));
        assert_eq!(device.power_percent, Some(29.0));
        assert_eq!(device.temperature_c, Some(52.0));
        assert_eq!(device.temperature_max_c, Some(110.0));
        assert_eq!(device.gpu_clock_mhz, Some(2_100));
        assert_eq!(device.memory_clock_mhz, Some(1_250));
        assert_eq!(device.pcie_tx, Some(16_384));
        assert_eq!(device.pcie_rx, Some(8_192));
        assert_eq!(device.pstate, None);
    }

    #[test]
    fn a_missing_rsmi_backend_leaves_every_amd_field_unavailable() {
        let (device, power_cap_known) = rsmi_device(&mut FakeRsmi::default(), true);
        assert!(!power_cap_known);
        assert_eq!(device.name, "");
        assert_eq!(device.utilization_pct, None);
        assert_eq!(device.memory_utilization_pct, None);
        assert_eq!(device.vram_total, None);
        assert_eq!(device.vram_used, None);
        assert_eq!(device.power_max_w, None);
        assert_eq!(device.power_w, None);
        assert_eq!(device.power_percent, None);
        assert_eq!(device.temperature_c, None);
        assert_eq!(device.temperature_max_c, None);
        assert_eq!(device.gpu_clock_mhz, None);
        assert_eq!(device.memory_clock_mhz, None);
        assert_eq!(device.pcie_tx, None);
        assert_eq!(device.pcie_rx, None);
    }

    #[test]
    fn one_failed_rsmi_call_leaves_only_that_field_unavailable() {
        let mut backend = full_rsmi();
        backend.gpu_clock_hz = None;
        let (device, _) = rsmi_device(&mut backend, true);
        assert_eq!(device.gpu_clock_mhz, None);
        assert_eq!(device.memory_clock_mhz, Some(1_250));
        assert_eq!(device.utilization_pct, Some(37.0));
        assert_eq!(device.vram_used, Some(1_073_741_824));
        assert_eq!(device.power_w, Some(88.0));
        assert_eq!(device.temperature_c, Some(52.0));
        assert_eq!(device.pcie_tx, Some(16_384));
    }

    #[test]
    fn power_cap_and_pcie_keep_btop_units() {
        let (device, _) = rsmi_device(&mut full_rsmi(), true);
        // 300_000_000 uW is btop's 300_000 mW pwr_max_usage, shown as 300 W.
        assert_eq!(device.power_max_w, Some(300.0));
        // 88_000_000 uW of average draw is 88_000 mW, shown as 88.0 W.
        assert_eq!(device.power_w, Some(88.0));
        // btop stores the PCIe throughput call's values unchanged.
        assert_eq!(device.pcie_tx, Some(16_384));
        assert_eq!(device.pcie_rx, Some(8_192));
    }

    #[test]
    fn a_missing_power_cap_leaves_power_unavailable() {
        let mut backend = full_rsmi();
        backend.power_cap_uw = None;
        let (device, power_cap_known) = rsmi_device(&mut backend, true);
        assert!(!power_cap_known);
        assert_eq!(device.power_max_w, None);
        assert_eq!(device.power_w, None);
        assert_eq!(device.power_percent, None);
        assert_eq!(device.utilization_pct, Some(37.0));
    }

    #[test]
    fn aggregates_reproduce_btop_1_3_0_integer_math() {
        let devices = vec![
            GpuDevice {
                utilization_pct: Some(0.0),
                vram_used: Some(4_000),
                vram_total: Some(4_000),
                power_w: Some(4.0),
                ..GpuDevice::default()
            },
            GpuDevice {
                utilization_pct: Some(100.0),
                vram_used: Some(6_000),
                vram_total: Some(6_000),
                power_w: Some(6.0),
                ..GpuDevice::default()
            },
            GpuDevice {
                utilization_pct: Some(1.0),
                ..GpuDevice::default()
            },
        ];
        let snapshot = summarize(devices, true);
        assert_eq!(snapshot.average_utilization_pct, Some(33.0)); // 101 / 3
        // 10_000 used / (10_000 bytes + 10_000 mW of power usage) = 0.
        assert_eq!(snapshot.vram_percent, Some(0.0));
        assert_eq!(snapshot.power_percent, Some(0.0));
    }

    /// DEF-GPU-40: the devices passed into a container are its own, so a
    /// container reads the recorded AMD card exactly as a host does.
    #[test]
    fn container_reads_the_same_devices_as_the_host() {
        let mut config = config();
        let host = collect(&fixture("gpu/proc"), &config);
        config.container = Some(crate::collector::container::Container {
            runtime: String::from("docker"),
            signal: String::from("1 /.dockerenv"),
            cgroup: None,
            own_netns: true,
        });
        let container = collect(&fixture("gpu/proc"), &config);
        assert!(!host.devices.is_empty());
        assert_eq!(format!("{container:?}"), format!("{host:?}"));
    }

    #[test]
    fn gpu_process_count_counts_distinct_pids() {
        let compute: [Result<Vec<u32>, ()>; 2] = [Ok(vec![1, 2]), Ok(vec![2, 3])];
        let graphics = [Ok(vec![3, 4])];
        assert_eq!(distinct_pids(compute.into_iter().chain(graphics)), Some(4));
    }

    #[test]
    fn gpu_process_count_is_zero_for_empty_lists() {
        assert_eq!(
            distinct_pids::<()>([Ok(Vec::new()), Ok(Vec::new())]),
            Some(0)
        );
    }

    #[test]
    fn gpu_process_count_is_unavailable_when_a_call_fails() {
        assert_eq!(distinct_pids([Ok(vec![1]), Err(())]), None);
    }

    #[test]
    fn gpu_lists_processes_only_outside_wsl() {
        assert!(!lists_processes("6.18.54.1-microsoft-standard-WSL2\n"));
        assert!(!lists_processes("4.4.0-19041-Microsoft"));
        assert!(lists_processes("6.8.0-45-generic\n"));
    }

    #[test]
    fn amd_devices_leave_the_gpu_process_count_unavailable() {
        let snapshot = collect(&fixture("gpu/proc"), &config());
        assert!(!snapshot.devices.is_empty());
        assert_eq!(snapshot.process_count, None);
    }

    #[test]
    fn gpu_nodes_are_the_device_nodes_only() {
        for node in [
            "/dev/nvidia0",
            "/dev/nvidia12",
            "/dev/dxg",
            "/dev/kfd",
            "/dev/dri/renderD128",
        ] {
            assert!(is_gpu_node(Path::new(node)), "{node}");
        }
        for other in [
            "/dev/nvidiactl",
            "/dev/nvidia-uvm",
            "/dev/nvidia-uvm-tools",
            "/dev/nvidia-modeset",
            "/dev/nvidia",
            "/dev/dri/card0",
            "/dev/null",
        ] {
            assert!(!is_gpu_node(Path::new(other)), "{other}");
        }
    }

    /// DEF-VIEW-48-GPU-FACT: the container fixture's members are 1 and 7 in
    /// the cgroup and 42 in `worker`; 1 and 42 hold a GPU node, 7 only
    /// NVIDIA control nodes.
    #[test]
    fn container_gpu_holders_count_members_holding_a_gpu_node() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let cgroup = fixture("container/sys/fs/cgroup");
        let root =
            std::env::temp_dir().join(format!("btop-web-gpu-{}-holders", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (pid, targets) in [
            (1, &["/dev/nvidia0", "/dev/nvidiactl"][..]),
            (7, &["/dev/nvidia-uvm", "/dev/nvidiactl"][..]),
            (42, &["/dev/null", "/dev/dxg"][..]),
        ] {
            let fd = root.join(format!("{pid}/fd"));
            std::fs::create_dir_all(&fd).unwrap();
            for (number, target) in targets.iter().enumerate() {
                symlink(target, fd.join(number.to_string())).unwrap();
            }
        }
        assert_eq!(gpu_holders(&root, &cgroup, 0), Some((2, 0)));
        // btop-web itself is not counted.
        assert_eq!(gpu_holders(&root, &cgroup, 42), Some((1, 0)));
        // Members that exited are skipped, not unreadable.
        let empty = root.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(gpu_holders(&empty, &cgroup, 0), Some((0, 0)));
        assert_eq!(gpu_holders(&root, &cgroup.join("missing"), 0), None);

        let denied = root.join("7/fd");
        std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(&denied).is_err() {
            // a privileged test run can read the directory anyway
            assert_eq!(gpu_holders(&root, &cgroup, 0), Some((2, 1)));
        }
        std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn recorded_tree_reports_unavailable_without_reading_live_nvml() {
        let snapshot = collect(&fixture("proc"), &config());
        assert!(snapshot.devices.is_empty(), "{:#?}", snapshot.devices);
        assert_eq!(snapshot.average_utilization_pct, None);
    }

    #[test]
    fn utilization_history_draws_after_two_ticks() {
        let mut devices = vec![GpuDevice {
            utilization_pct: Some(37.0),
            ..GpuDevice::default()
        }];
        let key = Path::new("gpu-utilization-history-test");
        attach_charts(key, &mut devices, 250);
        assert!(devices[0].chart.line.is_empty());
        attach_charts(key, &mut devices, 250);
        assert!(
            devices[0].chart.line.starts_with('M'),
            "{}",
            devices[0].chart.line
        );
    }

    /// ACC-GRAPH-43-GPU-LOAD: the GPU plot shares the network plot's time axis,
    /// so each tick pushes the newest sweep's value once (sample and hold),
    /// not once per sweep. Five ticks over one sweep value push five samples.
    #[test]
    fn utilization_history_takes_one_sample_per_tick() {
        let mut devices = vec![GpuDevice {
            utilization_pct: Some(37.0),
            ..GpuDevice::default()
        }];
        let key = Path::new("gpu-one-sample-per-tick-test");
        for _ in 0..5 {
            attach_charts(key, &mut devices, 250);
        }
        let first = (960 - 5) as f64 * gpu_lane(1).width / 959.0;
        let start = format!("M{first:.2} ");
        assert!(
            devices[0].chart.line.starts_with(&start),
            "expected {start}, got {}",
            &devices[0].chart.line[..devices[0].chart.line.len().min(24)]
        );
    }

    /// An unreadable utilization is a gap, never a zero line.
    #[test]
    fn unreadable_utilization_draws_no_line() {
        let mut devices = vec![GpuDevice::default()];
        let key = Path::new("gpu-unreadable-utilization-test");
        for _ in 0..3 {
            attach_charts(key, &mut devices, 250);
        }
        assert!(
            devices[0].chart.line.is_empty(),
            "{}",
            devices[0].chart.line
        );
        assert!(
            devices[0].chart.area.is_empty(),
            "{}",
            devices[0].chart.area
        );
    }

    /// Live check on this workstation: the live `/proc` root reads NVML. NVML
    /// is absent on hosts without an NVIDIA driver, where the check skips
    /// instead of failing.
    #[test]
    fn live_collect_lists_the_hosts_nvidia_gpus() {
        let config = config();
        if nvml(&config).is_none() {
            eprintln!("NVML unavailable; live NVIDIA GPU check skipped");
            return;
        }
        let snapshot = collect(Path::new("/proc"), &config);
        assert!(!snapshot.devices.is_empty(), "NVML reported no devices");
        assert!(snapshot.average_utilization_pct.is_some());
        // btop 1.3.0 never accumulates pwr_total, so gpu-pwr-total is 0.
        assert_eq!(snapshot.power_percent, Some(0.0));
        for device in &snapshot.devices {
            eprintln!(
                "{}: util {}, vram {}, power {}, temp {}",
                device.name,
                device.utilization_label(),
                device.vram_label(),
                device.power_label(),
                device.temperature_label()
            );
            assert!(
                !device.name.contains("NVIDIA"),
                "name was not normalized: {}",
                device.name
            );
        }
    }
}

//! CPU collector, mirroring `Cpu::collect`, `Cpu::get_cpuName`,
//! `Cpu::get_sensors`, `Cpu::update_sensors`, `Cpu::get_cpuHz`,
//! `Cpu::get_core_mapping` and `Cpu::get_battery` in btop's
//! `src/linux/btop_collect.cpp`.
//!
//! Raw source: `<proc_root>/stat` (`cpu` and `cpuN` lines with user, nice,
//! system, idle, iowait, irq, softirq, steal, guest and guest_nice),
//! `<proc_root>/loadavg` (the file glibc's `getloadavg` reads),
//! `<proc_root>/cpuinfo` (`model name` and the first `cpu MHz` line),
//! `<sys_root>/devices/system/cpu/cpufreq/policy0/scaling_cur_freq` (kHz),
//! `<sys_root>/class/hwmon/*`, `<sys_root>/devices/platform/coretemp.0/hwmon/*`
//! and `<sys_root>/class/thermal/*` (millidegrees), and
//! `<sys_root>/class/power_supply/*`.
//!
//! Formula (btop): totals = max(0, sum of the line's fields - the fields from
//! guest onward); idles = idle + iowait; the aggregate deltas are
//! max(1, totals - previous) and max(1, idles - previous), the per-core deltas
//! max(0, ...); usage = clamp(round((totals_delta - idles_delta) * 100 /
//! totals_delta), 0, 100), and each of the ten fields also gets
//! clamp(round(field_delta * 100 / totals_delta), 0, 100). btop calls
//! `Cpu::collect` once at init and never shows that since-boot average as the
//! current value: [`seed`] mirrors that call at startup, storing the counters
//! and leaving the since-boot average as the oldest history point, so the first
//! published tick is the delta between the seed and that tick. A cpuN missing from
//! `/proc/stat` contributes a zero sample. Frequency is policy0's
//! `scaling_cur_freq` / 1000 MHz, falling back to the first `cpu MHz` line and
//! formatted with btop's own rounding. Sensors and battery devices are
//! discovered once, then re-read every sample with integer millidegrees /
//! 1000, so temperatures are whole degrees Celsius.
//!
//! Config: `show_cpu_freq` gates the frequency read, `check_temp` gates the
//! sensor reads, `show_coretemp` gates the per-core readings, `cpu_sensor`
//! substitutes a discovered sensor name for btop's choice, `cpu_core_map`
//! overrides the core-to-sensor mapping, `show_battery` gates the battery
//! read, `selected_battery` chooses the device and `custom_cpu_name` replaces
//! the model name.
//!
//! Period: every sample, `update_ms` (250 ms by default; btop's 2000).
//!
//! Container (ACC-SCOPE-64-CONTAINER): inside a container this collector
//! departs from btop on the owner's order (2026-10-27) and reads the
//! container's cgroup for the total; `/proc/stat` serves only the cores in the
//! container's cpuset. Every sample, `update_ms`:
//! - total = clamp(round(delta `usage_usec` of `<cgroup>/cpu.stat` * 100 /
//!   (delta t in microseconds * E)), 0, 100), t from a monotonic clock and E
//!   from [`container::effective_cpus`]; the user, nice and system fields use
//!   `user_usec`, `nice_usec` and `system_usec` the same way, and the other
//!   seven fields are `None` (cgroup v2 has no idle, iowait, irq, softirq,
//!   steal or guest time). [`seed_container`] stores the first reading, so
//!   the first published tick is a delta, as on the host
//! - the core count is E; the cores (DEF-CPU-41) are the CPUs in
//!   `<cgroup>/cpuset.cpus.effective` (every CPU when the file is missing),
//!   each with btop's per-core usage from its `/proc/stat` cpuN line and
//!   named by its CPU number; with no cgroup v2 directory `cores` is empty
//! - load average (DEF-CPU-44): `/proc/loadavg` averages every task of the
//!   kernel, so the container's own is computed with the kernel's method,
//!   `calc_load` in `kernel/sched/loadavg.c`. Every [`LOAD_FREQ`], 30.004 s
//!   (six of the kernel's 5 s periods plus its one extra tick of `5*HZ+1`
//!   at HZ = 250, so the count drifts across the phase of periodic tasks as
//!   the kernel's does; the owner chose 15 s on 2026-10-28 to cut the count's
//!   cost to a third, and DEF-PERF-76-SERVER-COST doubled it to 30 s because
//!   the ~6,300 stat reads cost 40-60 ms and cannot be read cheaper), off the
//!   tick ([`count_container_load`]), active = the tids in `cgroup.threads` of
//!   the cgroup and every cgroup below it whose `<proc_root>/<tid>/stat` state
//!   is R or D, less the counting thread itself (`<proc_root>/thread-self`),
//!   which is always R while it reads. Then, in FIXED_1 = 2048 fixed point,
//!   avenrun = (avenrun * e + active * 2048 * (2048 - e) [+ 2047 when the
//!   load rises]) / 2048 with e = 2048 * exp(-30 s / 60 s), exp(-30/300),
//!   exp(-30/900) = 1242, 1853, 1981: the kernel's rule for EXP_1, EXP_5 and
//!   EXP_15, which gives its 1884, 2014, 2037 at 5 s, applied to 30 s. Shown
//!   as `/proc/loadavg` prints it (+ FIXED_1 / 200, two decimals truncated).
//!   The shown value is the last count's, a held sample up to 30 s old. The
//!   kernel starts avenrun at 0 at boot; here the first count, taken
//!   before the first tick, seeds all three averages, because the VM's history
//!   is not the container's. No cgroup v2 directory leaves load unavailable;
//!   frequency and temperatures are hardware sensors, not read; the battery
//!   is not read
//! - the CPU name stays `/proc/cpuinfo`'s model name, the processor's static
//!   identity

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::container::{self, Container};
use super::{CoreUsage, CpuSnapshot, sys_root};
use crate::chart::{CPU_HISTORY, History, smooth};
use crate::config::Config;

/// btop `time_names`, the ten `/proc/stat` fields.
const TIME_NAMES: [&str; 10] = [
    "user",
    "nice",
    "system",
    "idle",
    "iowait",
    "irq",
    "softirq",
    "steal",
    "guest",
    "guest_nice",
];

/// btop `Cpu::cpu_old` and the per-core old totals and idles.
#[derive(Default)]
struct CpuTimes {
    totals: u64,
    idles: u64,
    fields: [u64; 10],
    cores: Vec<(u64, u64)>,
    /// Each core's 1 s EMA (ACC-VIEW-48-CORE-BARS).
    core_smoothed: Vec<Option<f64>>,
    /// btop `Cpu::graph` history, one sample per tick.
    history: History,
    /// Container mode: the last `cpu.stat` reading (usage, user, nice and
    /// system microseconds) and when it was taken.
    usage: Option<(Instant, [Option<u64>; 4])>,
    /// Container mode: the kernel's `avenrun` for the container's tasks, in
    /// FIXED_1 units; `None` until the first count (DEF-CPU-44).
    avenrun: Option<[u64; 3]>,
}

static LAST: Mutex<BTreeMap<PathBuf, CpuTimes>> = Mutex::new(BTreeMap::new());

pub fn collect(proc_root: &Path, config: &Config) -> CpuSnapshot {
    let sys = sys_root(proc_root);
    if let Some(container) = &config.container {
        return collect_container(proc_root, &sys, container, config);
    }
    let sensors = config.check_temp.then(|| sensors(&sys)).flatten();
    let battery = if config.show_battery {
        battery(&sys, &config.selected_battery)
    } else {
        BatteryReading::default()
    };
    let mut snapshot = CpuSnapshot {
        name: model_name(proc_root, &sys, config),
        load_avg: load_average(proc_root),
        temperature_c: sensors
            .as_ref()
            .and_then(|sensors| sensors.temperature(&config.cpu_sensor)),
        temperature_max_c: sensors
            .as_ref()
            .and_then(|sensors| sensors.crit(&config.cpu_sensor))
            .map(|crit| crit as f64),
        battery_pct: battery.percent,
        battery_seconds: battery.seconds,
        battery_status: battery.status,
        ..CpuSnapshot::default()
    };
    if config.show_cpu_freq
        && let Some(frequency) = cpu_frequency(proc_root, &sys)
    {
        snapshot.cpu_hz = Some(frequency.text);
        snapshot.frequency_mhz = Some(frequency.mhz);
    }
    parse_stat(
        proc_root,
        &mut snapshot,
        config.update_ms,
        Some(config.window_ms()),
    );
    if config.show_coretemp
        && let Some(sensors) = &sensors
    {
        snapshot.core_temperatures_c = core_temperatures(
            proc_root,
            sensors,
            snapshot.cores.len(),
            &config.cpu_core_map,
        );
    }
    snapshot
}

fn model_name(proc_root: &Path, sys: &Path, config: &Config) -> Option<String> {
    if config.custom_cpu_name.is_empty() {
        cached_name(proc_root, sys)
    } else {
        Some(config.custom_cpu_name.clone())
    }
}

/// DEF-PERF-76-SERVER-COST: the model name's period. `/proc/cpuinfo` is about
/// 88 KB and the name is the processor's static identity, so it is re-read on
/// the 5 s slow cadence (`super::gpu::SLOW_PERIOD`), not every tick.
static NAMES: Mutex<BTreeMap<PathBuf, (Instant, Option<String>)>> = Mutex::new(BTreeMap::new());

/// [`cpu_name`] from a cache refreshed every `super::gpu::SLOW_PERIOD`.
fn cached_name(proc_root: &Path, sys: &Path) -> Option<String> {
    let mut cache = NAMES.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((read, name)) = cache.get(proc_root)
        && read.elapsed() < super::gpu::SLOW_PERIOD
    {
        return name.clone();
    }
    let name = cpu_name(proc_root, sys);
    cache.insert(proc_root.to_path_buf(), (Instant::now(), name.clone()));
    name
}

/// Container mode: the total from the cgroup's `cpu.stat` over E and the
/// container's own load average; frequency, temperatures and battery are
/// withheld.
fn collect_container(
    proc_root: &Path,
    sys: &Path,
    container: &Container,
    config: &Config,
) -> CpuSnapshot {
    let mut snapshot = CpuSnapshot {
        name: model_name(proc_root, sys, config),
        cores: container
            .cgroup
            .as_deref()
            .map(|cgroup| container_cores(proc_root, cgroup, config.update_ms))
            .unwrap_or_default(),
        ..CpuSnapshot::default()
    };
    // No cgroup v2 directory: every cgroup metric stays unavailable.
    let key = container.cgroup.as_deref().unwrap_or(proc_root);
    let mut states = LAST.lock().unwrap_or_else(|error| error.into_inner());
    let state = states.entry(key.to_path_buf()).or_default();
    if let Some(cgroup) = &container.cgroup {
        let effective = container::effective_cpus(cgroup);
        snapshot.effective_cpus = Some(effective);
        let now = Instant::now();
        let reading = cgroup_cpu_stat(cgroup);
        if let Some((then, previous)) = state.usage {
            let elapsed_usec = now.duration_since(then).as_micros() as f64;
            let share = |index: usize| {
                let delta = reading[index]?.saturating_sub(previous[index]?);
                Some(container_percent(delta as f64, elapsed_usec, effective))
            };
            snapshot.total_pct = share(0);
            // cpu.stat carries user, nice and system time, TIME_NAMES' first three.
            for (index, name) in TIME_NAMES.iter().enumerate() {
                let value = if index < 3 { share(index + 1) } else { None };
                snapshot.fields.insert(name.to_string(), value);
            }
        }
        state.usage = Some((now, reading));
    }
    snapshot.load_avg = state.avenrun.map(|avenrun| avenrun.map(loadavg_value));
    state.history.set_window(config.window_ms());
    state.history.push(snapshot.total_pct, config.update_ms);
    snapshot.gauge_pct = snapshot.total_pct.and(state.history.ema());
    snapshot.chart = state.history.chart(CPU_HISTORY, 0.0, 100.0);
    snapshot
}

/// Container mode's cores (DEF-CPU-41): btop's per-core usage from
/// `/proc/stat` for each CPU in `cpuset.cpus.effective`; every core when that
/// file is missing, as E then falls back to the online count.
fn container_cores(proc_root: &Path, cgroup: &Path, update_ms: u64) -> Vec<CoreUsage> {
    let mut all = CpuSnapshot::default();
    parse_stat(proc_root, &mut all, update_ms, None);
    let Some(cpus) = container::cpuset(cgroup) else {
        return all.cores;
    };
    all.cores
        .into_iter()
        .enumerate()
        .filter_map(|(cpu, core)| cpus.contains(&cpu).then_some(core))
        .collect()
}

/// Container mode's `seed`: store the first `cpu.stat` reading and the
/// `/proc/stat` core counters before the first tick, so that tick publishes
/// deltas, and seed the load average from the first task count.
pub(super) fn seed_container(proc_root: &Path, container: &Container, update_ms: u64) {
    let Some(cgroup) = &container.cgroup else {
        return;
    };
    parse_stat(proc_root, &mut CpuSnapshot::default(), update_ms, None);
    count_container_load(proc_root, cgroup);
    let mut states = LAST.lock().unwrap_or_else(|error| error.into_inner());
    states.entry(cgroup.clone()).or_default().usage =
        Some((Instant::now(), cgroup_cpu_stat(cgroup)));
}

/// The count's period: 30 s, six of the kernel's 5 s LOAD_FREQ periods, plus
/// the kernel's extra tick of `5*HZ+1` at HZ = 250 (this VM's CONFIG_HZ, the
/// common distribution default). The extra tick keeps the count from locking
/// to the phase of periodic tasks, the dashboard's own 250 ms tick included.
/// The owner chose 15 s on 2026-10-28; DEF-PERF-76-SERVER-COST doubled it,
/// because one count opens and reads about 6,300 `<tid>/stat` files, 40-60 ms
/// that no cheaper read can replace, and the shown load is still the same
/// kernel method, a held sample up to 30 s old.
pub const LOAD_FREQ: Duration = Duration::from_millis(30_004);
/// `include/linux/sched/loadavg.h`: FIXED_1, and the decay factors for one
/// 30 s step by its rule `FIXED_1 / exp(step / period)`: 2048 * exp(-30/60),
/// exp(-30/300), exp(-30/900) = 1242, 1853, 1981. At the kernel's 5 s step
/// the same rule gives its EXP_1, EXP_5, EXP_15 = 1884, 2014, 2037.
const FIXED_1: u64 = 1 << 11;
const EXP: [u64; 3] = [1242, 1853, 1981];

/// One LOAD_FREQ step of the container's load average (DEF-CPU-44): count
/// its R and D tasks and fold the count in, or seed all three averages with
/// the first count. Runs off the tick; an unreadable `cgroup.threads` leaves
/// the averages as they are.
pub fn count_container_load(proc_root: &Path, cgroup: &Path) {
    let Some(active) = active_tasks(proc_root, cgroup) else {
        return;
    };
    let active = active * FIXED_1;
    let mut states = LAST.lock().unwrap_or_else(|error| error.into_inner());
    let state = states.entry(cgroup.to_path_buf()).or_default();
    state.avenrun = Some(match state.avenrun {
        Some(load) => std::array::from_fn(|index| calc_load(load[index], EXP[index], active)),
        None => [active; 3],
    });
}

/// The container's tasks in state R or D: the tids in `cgroup.threads` of
/// `cgroup` and every cgroup below it, read from `<proc_root>/<tid>/stat`,
/// less the counting thread. `None` when the cgroup's own list is unreadable.
fn active_tasks(proc_root: &Path, cgroup: &Path) -> Option<u64> {
    let tids = super::proc::cgroup_members(cgroup, "cgroup.threads")?;
    let own = std::fs::read_link(proc_root.join("thread-self"))
        .ok()
        .and_then(|link| link.file_name()?.to_str()?.parse::<i32>().ok());
    // One open and one read per thread, into one buffer: the count reads
    // thousands of threads, and `read_to_string` adds a statx and a read to EOF.
    let mut path = proc_root.to_path_buf();
    let mut buffer = [0u8; 512];
    let active = tids
        .into_iter()
        .filter(|tid| Some(*tid) != own)
        .filter(|tid| {
            path.push(tid.to_string());
            path.push("stat");
            let read = std::fs::File::open(&path).and_then(|mut file| file.read(&mut buffer));
            path.pop();
            path.pop();
            // A task that exits mid-count has no stat file and is skipped.
            let stat = &buffer[..read.unwrap_or(0)];
            let state = stat
                .iter()
                .rposition(|&byte| byte == b')')
                .and_then(|end| stat.get(end + 2));
            matches!(state, Some(b'R' | b'D'))
        })
        .count();
    Some(active as u64)
}

/// `calc_load` in `kernel/sched/loadavg.c`; `load` and `active` in FIXED_1
/// units.
fn calc_load(load: u64, exp: u64, active: u64) -> u64 {
    let mut newload = load * exp + active * (FIXED_1 - exp);
    if active >= load {
        newload += FIXED_1 - 1;
    }
    newload / FIXED_1
}

/// One average as `/proc/loadavg` prints it: `LOAD_INT.LOAD_FRAC` of
/// avenrun + FIXED_1 / 200 (`fs/proc/loadavg.c`).
fn loadavg_value(avenrun: u64) -> f64 {
    let rounded = avenrun + FIXED_1 / 200;
    let fraction = ((rounded & (FIXED_1 - 1)) * 100) >> 11;
    // In hundredths, so 1.84 is the same f64 as the host path's parsed "1.84".
    ((rounded >> 11) * 100 + fraction) as f64 / 100.0
}

/// `usage_usec`, `user_usec`, `nice_usec` and `system_usec` of `cpu.stat`.
fn cgroup_cpu_stat(cgroup: &Path) -> [Option<u64>; 4] {
    let text = std::fs::read_to_string(cgroup.join("cpu.stat")).unwrap_or_default();
    ["usage_usec", "user_usec", "nice_usec", "system_usec"].map(|key| {
        text.lines().find_map(|line| {
            let (name, value) = line.split_once(' ')?;
            (name == key).then(|| value.trim().parse().ok()).flatten()
        })
    })
}

/// clamp(round(delta usage * 100 / (delta t * E)), 0, 100), both in
/// microseconds, with btop's integer rounding.
fn container_percent(usage_usec: f64, elapsed_usec: f64, effective: f64) -> f64 {
    let capacity = elapsed_usec * effective;
    if capacity <= 0.0 {
        return 0.0;
    }
    (usage_usec * 100.0 / capacity).round().clamp(0.0, 100.0)
}

/// btop `Shared::init` calls `Cpu::collect()` once before the runner starts
/// (`src/linux/btop_collect.cpp:253`). That call stores the counters and leaves
/// its since-boot average as the oldest history point, never as the current
/// value. Call this once, before the first tick.
pub(super) fn seed(proc_root: &Path, update_ms: u64) {
    parse_stat(
        proc_root,
        &mut CpuSnapshot::default(),
        update_ms,
        Some(crate::chart::WINDOW_MS),
    );
}

/// `window_ms` `None` skips the total's history and chart: a container shows
/// only the per-core rows of `/proc/stat` (DEF-CPU-41). `Some` records the
/// sample and spans the configured chart window (ACC-VIEW-110-TIME-AXIS); the
/// startup seed uses the default, which the first tick replaces.
fn parse_stat(
    proc_root: &Path,
    snapshot: &mut CpuSnapshot,
    update_ms: u64,
    window_ms: Option<u64>,
) {
    let Ok(text) = std::fs::read_to_string(proc_root.join("stat")) else {
        return;
    };
    let mut states = LAST.lock().unwrap_or_else(|error| error.into_inner());
    let state = states.entry(proc_root.to_path_buf()).or_default();
    let mut cores: BTreeMap<usize, (u64, u64)> = BTreeMap::new();
    let mut aggregate: Option<(Vec<u64>, u64, u64)> = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else { continue };
        let Some(core) = name.strip_prefix("cpu") else {
            continue;
        };
        let times: Vec<u64> = fields.map_while(|value| value.parse().ok()).collect();
        if times.len() < 4 {
            continue;
        }
        let total_sum: u64 = times.iter().sum();
        let guest: u64 = times.iter().skip(8).sum();
        let totals = total_sum.saturating_sub(guest);
        let idles = times[3].saturating_add(times.get(4).copied().unwrap_or(0));
        if core.is_empty() {
            aggregate = Some((times, totals, idles));
        } else if let Ok(index) = core.parse::<usize>() {
            cores.insert(index, (totals, idles));
        }
    }
    if let Some((times, totals, idles)) = aggregate {
        let calc_totals = totals.saturating_sub(state.totals).max(1);
        let calc_idles = idles.saturating_sub(state.idles).max(1);
        state.totals = totals;
        state.idles = idles;
        snapshot.total_pct = Some(percent(calc_totals, calc_idles));
        for (index, name) in TIME_NAMES.iter().enumerate() {
            let Some(value) = times.get(index).copied() else {
                break;
            };
            let delta = value as i64 - state.fields[index] as i64;
            state.fields[index] = value;
            let pct = (delta as f64 * 100.0 / calc_totals as f64)
                .round()
                .clamp(0.0, 100.0);
            snapshot.fields.insert(name.to_string(), Some(pct));
        }
    }
    if let Some((highest, _)) = cores.last_key_value() {
        let target = highest + 1;
        if state.cores.len() < target {
            state.cores.resize(target, (0, 0));
        }
        if state.core_smoothed.len() < target {
            state.core_smoothed.resize(target, None);
        }
        for index in 0..target {
            let usage = match cores.get(&index) {
                Some(&(totals, idles)) => {
                    let calc_totals = totals.saturating_sub(state.cores[index].0);
                    let calc_idles = idles.saturating_sub(state.cores[index].1);
                    state.cores[index] = (totals, idles);
                    Some(percent(calc_totals, calc_idles))
                }
                // btop pushes a zero sample for a core missing from /proc/stat.
                None => Some(0.0),
            };
            if let Some(usage) = usage {
                state.core_smoothed[index] =
                    Some(smooth(state.core_smoothed[index], usage, update_ms));
            }
            snapshot.cores.push(CoreUsage {
                name: format!("cpu{index}"),
                usage_pct: usage,
                smoothed_pct: usage.and(state.core_smoothed[index]),
            });
        }
    }
    if let Some(window_ms) = window_ms {
        state.history.set_window(window_ms);
        state.history.push(snapshot.total_pct, update_ms);
        snapshot.gauge_pct = snapshot.total_pct.and(state.history.ema());
        snapshot.chart = state.history.chart(CPU_HISTORY, 0.0, 100.0);
    }
}

/// btop: `clamp(round((calc_totals - calc_idles) * 100 / calc_totals), 0, 100)`.
fn percent(totals: u64, idles: u64) -> f64 {
    if totals == 0 {
        return 0.0;
    }
    let busy = totals.saturating_sub(idles);
    (busy as f64 * 100.0 / totals as f64)
        .round()
        .clamp(0.0, 100.0)
}

fn load_average(proc_root: &Path) -> Option<[f64; 3]> {
    let text = std::fs::read_to_string(proc_root.join("loadavg")).ok()?;
    let mut fields = text.split_whitespace();
    let one = fields.next()?.parse().ok()?;
    let five = fields.next()?.parse().ok()?;
    let fifteen = fields.next()?.parse().ok()?;
    Some([one, five, fifteen])
}

struct CpuFrequency {
    /// btop `Cpu::cpuHz`, formatted with btop's own rounding.
    text: String,
    mhz: u64,
}

/// btop `Cpu::get_cpuHz`: policy0 only, then the first `cpu MHz` line.
fn cpu_frequency(proc_root: &Path, sys: &Path) -> Option<CpuFrequency> {
    let policy = sys.join("devices/system/cpu/cpufreq/policy0/scaling_cur_freq");
    let mut hz = 0.0;
    if let Some(khz) = read_trimmed(&policy) {
        // btop's stod throws on malformed content and skips the cpuinfo
        // fallback for that sample.
        let khz: f64 = khz.parse().ok()?;
        hz = khz / 1000.0;
    }
    if hz <= 0.0 {
        hz = cpuinfo_mhz(proc_root)?;
    }
    if hz <= 1.0 || hz >= 1_000_000.0 {
        return None;
    }
    Some(CpuFrequency {
        text: format_hz(hz),
        mhz: hz.round() as u64,
    })
}

fn cpuinfo_mhz(proc_root: &Path) -> Option<f64> {
    let text = std::fs::read_to_string(proc_root.join("cpuinfo")).ok()?;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("cpu ") else {
            continue;
        };
        if !rest.starts_with('M') {
            continue;
        }
        let (_, value) = line.split_once(':')?;
        return value.split_whitespace().next()?.parse().ok();
    }
    None
}

/// btop's `get_cpuHz` formatting, including the three-character truncation
/// below 10 GHz.
fn format_hz(hz: f64) -> String {
    if hz >= 10_000.0 {
        format!("{} GHz", (hz / 1000.0).round() as i64)
    } else if hz >= 1000.0 {
        let text = format!("{:.6}", (hz / 100.0).round() / 10.0);
        format!("{} GHz", &text[..3])
    } else {
        format!("{} MHz", hz.round() as i64)
    }
}

/// btop `Cpu::get_cpuName`, including the ARM `sys/devices` fallback and the
/// Xeon, Ryzen and Intel shortening rules.
fn cpu_name(proc_root: &Path, sys: &Path) -> Option<String> {
    let text = std::fs::read_to_string(proc_root.join("cpuinfo")).ok()?;
    let mut name = String::new();
    let mut found = false;
    for line in text.lines() {
        if let Some((key, value)) = line.split_once(':')
            && key.starts_with("model name")
        {
            name = value.trim().to_string();
            found = true;
            break;
        }
    }
    if !found {
        return arm_name(sys);
    }
    let words: Vec<&str> = name.split_whitespace().collect();
    let mut shortened = String::new();
    if (name.contains("Xeon") || words.contains(&"Duo")) && words.contains(&"CPU") {
        shortened = words
            .iter()
            .position(|word| *word == "CPU")
            .and_then(|position| words.get(position + 1))
            .filter(|next| !next.ends_with(')'))
            .map_or_else(String::new, |next| (*next).to_string());
    } else if let Some(position) = words.iter().position(|word| *word == "Ryzen") {
        shortened = String::from("Ryzen");
        for word in words.iter().skip(position + 1).take(2) {
            shortened.push(' ');
            shortened.push_str(word);
        }
    } else if name.contains("Intel") && words.contains(&"CPU") {
        shortened = words
            .iter()
            .position(|word| *word == "CPU")
            .and_then(|position| words.get(position + 1))
            .filter(|next| !next.ends_with(')') && next.len() != 1)
            .map_or_else(String::new, |next| (*next).to_string());
    }
    if shortened.is_empty() && !words.is_empty() {
        let mut fallback = String::new();
        for word in &words {
            if *word == "@" {
                break;
            }
            fallback.push_str(word);
            fallback.push(' ');
        }
        fallback.pop();
        for replace in ["Processor", "CPU", "(R)", "(TM)", "Intel", "AMD", "Core"] {
            fallback = fallback.replace(replace, "");
            fallback = fallback.replace("  ", " ");
        }
        shortened = fallback.trim().to_string();
    }
    (!shortened.is_empty()).then_some(shortened)
}

fn arm_name(sys: &Path) -> Option<String> {
    let entry = std::fs::read_dir(sys.join("devices"))
        .ok()?
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().starts_with("arm"))?;
    let name = entry.file_name().to_string_lossy().into_owned();
    let parts: Vec<&str> = name.split('_').filter(|part| !part.is_empty()).collect();
    if parts.len() < 2 {
        return Some(capitalize(&name));
    }
    let mut shortened = capitalize(parts[1]);
    if let Some(third) = parts.get(2) {
        shortened.push(' ');
        shortened.push_str(&capitalize(third));
    }
    Some(shortened)
}

fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// One entry of btop `Cpu::found_sensors`.
#[derive(Clone)]
struct Sensor {
    path: PathBuf,
    /// btop `crit`, in whole degrees Celsius.
    crit: i64,
}

/// btop's `found_sensors`, `cpu_sensor`, `core_sensors` and `cpu_temp_only`.
#[derive(Clone)]
struct SensorSet {
    sensors: Vec<(String, Sensor)>,
    cpu: usize,
    core: Vec<usize>,
    cpu_temp_only: bool,
}

static SENSORS: Mutex<BTreeMap<PathBuf, Option<SensorSet>>> = Mutex::new(BTreeMap::new());

/// btop discovers its sensors once in `Shared::init` and re-reads them every
/// sample, so the selection is cached per sysfs tree.
fn sensors(sys: &Path) -> Option<SensorSet> {
    let mut cache = SENSORS.lock().unwrap_or_else(|error| error.into_inner());
    if !cache.contains_key(sys) {
        let found = discover_sensors(sys);
        cache.insert(sys.to_path_buf(), found);
    }
    cache.get(sys).cloned().flatten()
}

/// btop `Cpu::get_sensors`.
fn discover_sensors(sys: &Path) -> Option<SensorSet> {
    let mut search_paths: Vec<PathBuf> = Vec::new();
    let mut got_cpu = false;
    let mut got_coretemp = false;
    let mut found: Vec<(String, Sensor)> = Vec::new();
    let mut cpu: Option<usize> = None;
    let mut core: Vec<usize> = Vec::new();

    for directory in sorted_entries(&sys.join("class/hwmon")) {
        let add_path = std::fs::canonicalize(&directory).unwrap_or(directory);
        if search_paths.contains(&add_path) || search_paths.contains(&add_path.join("device")) {
            continue;
        }
        // btop uses the canonical path to spot a coretemp hwmon.
        if add_path.to_string_lossy().contains("coretemp") {
            got_coretemp = true;
        }
        let mut add = false;
        for file in sorted_entries(&add_path) {
            if is_input(&file) {
                add = true;
                break;
            }
            if file.file_name().is_some_and(|name| name == "device")
                && sorted_entries(&file).iter().any(|file| is_input(file))
                && !search_paths.contains(&file)
            {
                search_paths.push(file);
            }
        }
        if add && !search_paths.contains(&add_path) {
            search_paths.push(add_path);
        }
    }
    if !got_coretemp {
        for directory in sorted_entries(&sys.join("devices/platform/coretemp.0/hwmon")) {
            let add_path = std::fs::canonicalize(&directory).unwrap_or(directory);
            if sorted_entries(&add_path).iter().any(|file| is_input(file))
                && !search_paths.contains(&add_path)
            {
                search_paths.push(add_path);
                got_coretemp = true;
            }
        }
    }

    for path in &search_paths {
        let name = read_trimmed(&path.join("name")).unwrap_or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        for file in sorted_entries(path) {
            let file_name = file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Some(base) = file_name.strip_suffix("input") else {
                continue;
            };
            let id: u64 = base
                .strip_prefix("temp")
                .and_then(|rest| rest.strip_suffix('_'))
                .and_then(|rest| rest.parse().ok())
                .unwrap_or(0);
            let label = read_trimmed(&file.with_file_name(format!("{base}label")))
                .unwrap_or_else(|| format!("temp{id}"));
            let crit =
                read_i64(&file.with_file_name(format!("{base}crit"))).unwrap_or(95_000) / 1000;
            let sensor_name = format!("{name}/{label}");
            let sensor = Sensor { path: file, crit };
            let position = match found.iter().position(|(name, _)| *name == sensor_name) {
                Some(position) => {
                    found[position].1 = sensor;
                    position
                }
                None => {
                    found.push((sensor_name, sensor));
                    found.len() - 1
                }
            };
            if !got_cpu && (label.starts_with("Package id") || label.starts_with("Tdie")) {
                got_cpu = true;
                cpu = Some(position);
            } else if label.starts_with("Core") || label.starts_with("Tccd") {
                got_coretemp = true;
                if !core.contains(&position) {
                    core.push(position);
                }
            }
        }
    }

    // btop scans /sys/class/thermal only when hwmon had no Package id or Tdie.
    if !got_cpu {
        for index in 0..1024 {
            let zone = sys.join(format!("class/thermal/thermal_zone{index}"));
            if !zone.exists() {
                break;
            }
            if !zone.join("temp").exists() {
                continue;
            }
            let label = read_trimmed(&zone.join("type")).unwrap_or_else(|| format!("temp{index}"));
            let mut crit = 0;
            for trip in 0..1024 {
                let path = zone.join(format!("trip_point_{trip}_temp"));
                if !path.exists() {
                    break;
                }
                let kind = read_trimmed(&zone.join(format!("trip_point_{trip}_type")));
                if kind.as_deref() != Some("critical") {
                    continue;
                }
                crit = read_i64(&path).unwrap_or(0) / 1000;
            }
            // btop defaults a missing critical trip point to 95 degrees.
            if crit < 1 {
                crit = 95;
            }
            let sensor_name = format!("thermal{index}/{label}");
            if !found.iter().any(|(name, _)| *name == sensor_name) {
                found.push((
                    sensor_name,
                    Sensor {
                        path: zone.join("temp"),
                        crit,
                    },
                ));
            }
        }
    }

    let cpu_temp_only = !got_coretemp || core.is_empty();
    if !cpu_temp_only {
        // btop sorts the names, then stable-sorts by length.
        core.sort_by(|left, right| {
            let left = &found[*left].0;
            let right = &found[*right].0;
            left.len().cmp(&right.len()).then_with(|| left.cmp(right))
        });
    }

    if found.is_empty() {
        return None;
    }
    let cpu = cpu
        .or_else(|| {
            found.iter().position(|(name, _)| {
                let name = name.to_lowercase();
                name.contains("cpu") || name.contains("k10temp")
            })
        })
        .unwrap_or(0);
    Some(SensorSet {
        sensors: found,
        cpu,
        core,
        cpu_temp_only,
    })
}

impl SensorSet {
    /// btop `update_sensors`: a configured name is used only when it was
    /// discovered, otherwise btop's own choice stands.
    fn selected(&self, configured: &str) -> Option<&Sensor> {
        self.sensors
            .iter()
            .find(|(name, _)| name == configured)
            .map(|(_, sensor)| sensor)
            .or_else(|| self.sensors.get(self.cpu).map(|(_, sensor)| sensor))
    }

    /// btop `Cpu::update_sensors` for `current_cpu.temp[0]`.
    fn temperature(&self, configured: &str) -> Option<f64> {
        read_i64(&self.selected(configured)?.path).map(|value| (value / 1000) as f64)
    }

    /// btop `current_cpu.temp_max`, the selected sensor's critical point.
    fn crit(&self, configured: &str) -> Option<i64> {
        self.selected(configured).map(|sensor| sensor.crit)
    }
}

/// btop `Cpu::update_sensors` for `current_cpu.temp[1..]`, aligned with the
/// cores. A core with no mapped sensor stays `None`.
fn core_temperatures(
    proc_root: &Path,
    sensors: &SensorSet,
    core_count: usize,
    custom_map: &str,
) -> Vec<Option<f64>> {
    if sensors.cpu_temp_only || core_count == 0 {
        return Vec::new();
    }
    let cpuinfo = std::fs::read_to_string(proc_root.join("cpuinfo")).unwrap_or_default();
    let mut mapping = core_mapping(&cpuinfo, core_count, sensors.core.len());
    apply_custom_map(&mut mapping, custom_map, sensors.core.len());
    let mut readings: Vec<Option<f64>> = Vec::with_capacity(sensors.core.len());
    for index in &sensors.core {
        readings.push(read_i64(&sensors.sensors[*index].1.path).map(|value| (value / 1000) as f64));
    }
    (0..core_count)
        .map(|core| {
            mapping
                .get(&core)
                .and_then(|index| readings.get(*index))
                .copied()
                .flatten()
        })
        .collect()
}

/// btop `get_core_mapping`'s custom override: whitespace-separated
/// `core:sensor` pairs, skipped when the core is unknown or the sensor index
/// is past the last sensor. A parse failure stops the remaining pairs, as
/// btop's `std::stoi` exception aborts its loop.
fn apply_custom_map(mapping: &mut BTreeMap<usize, usize>, custom_map: &str, sensor_count: usize) {
    for pair in custom_map.split_whitespace() {
        let Some((core, sensor)) = pair.split_once(':') else {
            break;
        };
        let (Ok(core), Ok(sensor)) = (core.parse::<usize>(), sensor.parse::<usize>()) else {
            break;
        };
        if sensor > sensor_count {
            continue;
        }
        if let Some(mapped) = mapping.get_mut(&core) {
            *mapped = sensor;
        }
    }
}

/// btop `Cpu::get_core_mapping`: `processor`/`core id` pairs from
/// `/proc/cpuinfo`, then btop's incomplete-map fallbacks.
fn core_mapping(cpuinfo: &str, core_count: usize, sensor_count: usize) -> BTreeMap<usize, usize> {
    let mut mapping: BTreeMap<usize, usize> = BTreeMap::new();
    if sensor_count == 0 {
        return mapping;
    }
    let mut cpu = 0;
    let mut next = 0;
    for line in cpuinfo.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key == "processor" {
            if let Ok(value) = value.trim().parse() {
                cpu = value;
            }
        } else if key.starts_with("core")
            && let Ok(core) = value.trim().parse::<usize>()
        {
            if core >= sensor_count {
                if next >= sensor_count {
                    next = 0;
                }
                mapping.insert(cpu, next);
                next += 1;
            } else {
                mapping.insert(cpu, core);
            }
        }
    }
    if mapping.len() < core_count {
        if core_count.is_multiple_of(2) && mapping.len() == core_count / 2 {
            for index in 0..core_count / 2 {
                if next >= sensor_count {
                    next = 0;
                }
                mapping.insert(core_count / 2 + index, next);
                next += 1;
            }
        } else {
            mapping.clear();
            next = 0;
            for index in 0..core_count {
                if next >= sensor_count {
                    next = 0;
                }
                mapping.insert(index, next);
                next += 1;
            }
        }
    }
    mapping
}

#[derive(Default)]
struct BatteryReading {
    percent: Option<f64>,
    seconds: Option<i64>,
    status: Option<String>,
}

/// btop `Cpu::get_battery`'s cached device.
#[derive(Clone)]
struct BatteryDevice {
    /// btop's `batteries` map key, the device directory name.
    name: String,
    dir: PathBuf,
    /// btop `device_type == "Battery"`; a UPS is only a fallback.
    is_battery: bool,
    /// btop `energy_now`: `energy_now` or `charge_now`.
    energy_now: Option<PathBuf>,
    /// btop `energy_full`: `energy_full` or `charge_full`.
    energy_full: Option<PathBuf>,
    /// btop `power_now`: `power_now` or `current_now`.
    power_now: Option<PathBuf>,
    online: Option<PathBuf>,
    use_energy: bool,
}

/// btop's discovered `batteries` map and its cached `auto_sel`.
#[derive(Clone)]
struct BatterySet {
    devices: Vec<BatteryDevice>,
    /// Index of the first Battery, else 0. btop walks an unordered_map, so
    /// only the Battery-before-UPS rule is guaranteed; this keeps directory
    /// order and imposes no sort btop does not have.
    auto: usize,
}

static BATTERIES: Mutex<BTreeMap<PathBuf, Option<BatterySet>>> = Mutex::new(BTreeMap::new());

/// btop discovers its batteries once and keeps `has_battery = false` for the
/// process lifetime after the first unreadable percentage, so the outcome is
/// cached per sysfs tree.
fn battery(sys: &Path, selected: &str) -> BatteryReading {
    let set = {
        let mut cache = BATTERIES.lock().unwrap_or_else(|error| error.into_inner());
        if !cache.contains_key(sys) {
            let found = discover_battery(sys);
            cache.insert(sys.to_path_buf(), found);
        }
        cache.get(sys).cloned().flatten()
    };
    let Some(set) = set else {
        return BatteryReading::default();
    };
    // btop uses `selected_battery` only when it names a discovered device.
    let device = set
        .devices
        .iter()
        .find(|device| selected != "Auto" && device.name == selected)
        .or_else(|| set.devices.get(set.auto));
    let Some(device) = device else {
        return BatteryReading::default();
    };
    let Some(percent) = battery_percent(device) else {
        let mut cache = BATTERIES.lock().unwrap_or_else(|error| error.into_inner());
        cache.insert(sys.to_path_buf(), None);
        return BatteryReading::default();
    };
    let status = battery_status(device, percent);
    let seconds = battery_seconds(device, &status);
    BatteryReading {
        percent: Some(percent),
        seconds,
        status: Some(status),
    }
}

/// btop's first-run scan: every present Battery or UPS, in directory order.
fn discover_battery(sys: &Path) -> Option<BatterySet> {
    let entries = std::fs::read_dir(sys.join("class/power_supply")).ok()?;
    let mut devices = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(kind) = read_trimmed(&dir.join("type")) else {
            continue;
        };
        if kind != "Battery" && kind != "UPS" {
            continue;
        }
        if read_i64(&dir.join("present")) != Some(1) {
            continue;
        }
        let energy_now = existing(&dir, "energy_now").or_else(|| existing(&dir, "charge_now"));
        let energy_full = existing(&dir, "energy_full").or_else(|| existing(&dir, "charge_full"));
        let use_energy = energy_now.is_some() && energy_full.is_some();
        if !use_energy && !dir.join("capacity").exists() {
            continue;
        }
        let power_now = existing(&dir, "power_now").or_else(|| existing(&dir, "current_now"));
        let online = existing(&dir, "AC0/online").or_else(|| existing(&dir, "AC/online"));
        devices.push(BatteryDevice {
            name: entry.file_name().to_string_lossy().into_owned(),
            dir,
            is_battery: kind == "Battery",
            energy_now,
            energy_full,
            power_now,
            online,
            use_energy,
        });
    }
    let auto = devices
        .iter()
        .position(|device| device.is_battery)
        .unwrap_or(0);
    (!devices.is_empty()).then_some(BatterySet { devices, auto })
}

fn existing(dir: &Path, name: &str) -> Option<PathBuf> {
    let path = dir.join(name);
    path.exists().then_some(path)
}

/// btop prefers `round(100 * energy_now / energy_full)` and only falls back to
/// the `capacity` file.
fn battery_percent(device: &BatteryDevice) -> Option<f64> {
    let mut percent = None;
    if device.use_energy {
        let now = device
            .energy_now
            .as_ref()
            .and_then(|path| read_i64(path))
            .unwrap_or(-1);
        let full = device
            .energy_full
            .as_ref()
            .and_then(|path| read_i64(path))
            .unwrap_or(1);
        if full != 0 {
            percent = Some((100.0 * now as f64 / full as f64).round());
        }
    }
    let percent = match percent {
        Some(value) if value >= 0.0 => value,
        _ => read_i64(&device.dir.join("capacity"))? as f64,
    };
    (percent >= 0.0).then_some(percent)
}

fn battery_status(device: &BatteryDevice, percent: f64) -> String {
    let mut status = read_trimmed(&device.dir.join("status"))
        .map(|status| status.to_lowercase())
        .unwrap_or_else(|| "unknown".to_string());
    if status == "unknown"
        && let Some(online) = &device.online
    {
        status = if read_trimmed(online).as_deref() == Some("1") {
            if percent < 100.0 {
                "charging".to_string()
            } else {
                "full".to_string()
            }
        } else {
            "discharging".to_string()
        };
    }
    status
}

fn battery_seconds(device: &BatteryDevice, status: &str) -> Option<i64> {
    if status == "charging" || status == "full" {
        return None;
    }
    if device.use_energy
        && let Some(now) = device.energy_now.as_ref().and_then(|path| read_i64(path))
        && let Some(power) = device.power_now.as_ref().and_then(|path| read_i64(path))
        && power > 0
    {
        return Some((now as f64 / power as f64 * 3600.0).round() as i64);
    }
    read_i64(&device.dir.join("time_to_empty")).map(|minutes| minutes * 60)
}

fn sorted_entries(path: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    paths
}

fn is_input(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with("input"))
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn read_i64(path: &Path) -> Option<i64> {
    read_trimmed(path)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative)
    }

    /// btop 1.3.0's embedded defaults, plus any BTOP_WEB_* environment values.
    fn config() -> Config {
        Config::load(None).expect("embedded default configuration")
    }

    /// A writable proc tree so one test can feed two successive samples.
    fn temp_proc(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("btop-web-cpu-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proc")).expect("temp proc tree");
        root.join("proc")
    }

    fn remove_temp_proc(proc_root: &Path) {
        std::fs::remove_dir_all(proc_root.parent().expect("temp root")).expect("cleanup");
    }

    #[test]
    fn uses_btops_rounded_formula() {
        assert_eq!(percent(10, 5), 50.0);
        assert_eq!(percent(3, 1), 67.0);
        assert_eq!(percent(1, 1), 0.0);
        assert_eq!(percent(1, 5), 0.0);
        assert_eq!(percent(0, 0), 0.0);
    }

    #[test]
    fn seed_records_the_since_boot_point_and_the_first_tick_is_the_delta() {
        let config = config();
        let proc_root = temp_proc("first");
        // Since boot: 200 busy of 1000 aggregate (20%), cpu0 100 of 500 (20%).
        std::fs::write(
            proc_root.join("stat"),
            "cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0 0 0\n",
        )
        .expect("seed stat");
        seed(&proc_root, config.update_ms);
        // This tick moved 2400 totals and 2200 idles: 8%, and cpu0 20%.
        std::fs::write(
            proc_root.join("stat"),
            "cpu  200 0 200 3000 0 0 0 0 0 0\ncpu0 100 0 100 800 0 0 0 0 0 0\n",
        )
        .expect("tick stat");
        let first = collect(&proc_root, &config);
        assert_eq!(first.total_pct, Some(8.0));
        assert_eq!(first.cores.len(), 1);
        assert_eq!(first.cores[0].usage_pct, Some(20.0));
        assert_eq!(first.fields.len(), 10);
        // btop's init sample survives only as the oldest graph point. The
        // chart plots the smoothed series; the readout above stays raw.
        let mut expected = crate::chart::History::new();
        expected.push(Some(20.0), config.update_ms);
        expected.push(Some(8.0), config.update_ms);
        assert_eq!(first.chart, expected.chart(CPU_HISTORY, 0.0, 100.0));
        // ACC-GRAPH-47-GAUGE-SMOOTH: the gauge arc draws the same EMA.
        assert_eq!(first.gauge_pct, expected.ema());
        assert_ne!(first.gauge_pct, Some(8.0));
        // ACC-VIEW-48-CORE-BARS: cpu0 smooths 20 (since boot) then 20.
        assert_eq!(first.cores[0].smoothed_pct, Some(20.0));
        assert_ne!(
            first.chart,
            crate::chart::ChartPath::new(
                &[20.0, 8.0],
                0,
                crate::chart::capacity(config.window_ms(), config.update_ms),
                CPU_HISTORY.width,
                CPU_HISTORY.height,
                0.0,
                100.0
            )
        );

        // The counters did not move again: the delta is genuinely zero. The
        // core's number and bar step a fifth of the way, not to zero.
        let second = collect(&proc_root, &config);
        assert_eq!(second.total_pct, Some(0.0));
        assert_eq!(second.cores[0].usage_pct, Some(0.0));
        let alpha = 1.0 - (-(config.update_ms as f64) / 1000.0).exp();
        let smoothed = second.cores[0].smoothed_pct.expect("smoothed core");
        assert!((smoothed - 20.0 * (1.0 - alpha)).abs() < 1e-9, "{smoothed}");
        remove_temp_proc(&proc_root);
    }

    #[test]
    fn first_frame_carries_btop_metadata_and_the_seeded_delta() {
        let config = config();
        let proc_root = fixture("proc");
        seed(&proc_root, config.update_ms);
        let first = collect(&proc_root, &config);
        // The recorded counters do not move between the seed and this tick, so
        // the first frame publishes a measured zero, not a since-boot share.
        assert_eq!(first.total_pct, Some(0.0));
        assert_eq!(first.fields.len(), 10);
        assert_eq!(first.cores.len(), 64);
        assert_eq!(first.cores[0].name, "cpu0");
        assert!(first.cores.iter().all(|core| core.usage_pct == Some(0.0)));
        // Everything not derived from /proc/stat is read on the same frame.
        assert_eq!(first.cpu_hz.as_deref(), Some("4.0 GHz"));
        assert_eq!(first.frequency_mhz, Some(3994));
        assert_eq!(first.name.as_deref(), Some("Ryzen Threadripper PRO"));
        assert_eq!(first.load_avg, Some([88.99, 91.86, 90.59]));
        assert_eq!(first.temperature_c, Some(45.0));
        assert_eq!(first.temperature_max_c, Some(100.0));
        assert_eq!(first.core_temperatures_c.len(), 64);
        assert_eq!(first.core_temperatures_c[0], Some(44.0));
        // The battery exposes energy_now/energy_full and a deliberately
        // different capacity; btop prefers the energy ratio.
        assert_eq!(first.battery_pct, Some(74.0));
        assert_eq!(first.battery_status.as_deref(), Some("discharging"));
        assert_eq!(first.battery_seconds, Some(13_392));

        let second = collect(&proc_root, &config);
        assert_eq!(second.total_pct, Some(0.0));
        assert_eq!(second.fields.len(), 10);
        assert_eq!(second.fields["user"], Some(0.0));
        assert_eq!(second.cores[0].usage_pct, Some(0.0));
    }

    #[test]
    fn missing_cores_get_a_zero_sample_and_new_cores_start_at_zero() {
        let config = config();
        let proc_root = temp_proc("delta");
        std::fs::copy(fixture("cpu/delta-before/stat"), proc_root.join("stat")).expect("before");
        seed(&proc_root, config.update_ms);

        std::fs::copy(fixture("cpu/delta-after/stat"), proc_root.join("stat")).expect("after");
        let after = collect(&proc_root, &config);
        assert_eq!(after.total_pct, Some(20.0));
        assert_eq!(after.cores.len(), 3);
        assert_eq!(after.cores[0].usage_pct, Some(20.0));
        // cpu1 is missing from the second /proc/stat: btop pushes a zero.
        assert_eq!(after.cores[1].usage_pct, Some(0.0));
        // cpu2 is new: its counters start at zero, so this is since-boot.
        assert_eq!(after.cores[2].usage_pct, Some(20.0));
        assert_eq!(after.fields["user"], Some(10.0));
        remove_temp_proc(&proc_root);
    }

    #[test]
    fn total_history_draws_on_the_first_published_frame() {
        let config = config();
        let proc_root = temp_proc("chart");
        std::fs::copy(fixture("cpu/delta-before/stat"), proc_root.join("stat")).expect("before");
        seed(&proc_root, config.update_ms);
        std::fs::copy(fixture("cpu/delta-after/stat"), proc_root.join("stat")).expect("after");
        // The since-boot seed and this tick are two knots, so the first frame
        // draws a curve the way btop's first pass does.
        let first = collect(&proc_root, &config);
        assert!(first.chart.line.starts_with('M'), "{}", first.chart.line);

        let second = collect(&proc_root, &config);
        assert!(second.chart.line.starts_with('M'), "{}", second.chart.line);
        remove_temp_proc(&proc_root);
    }

    #[test]
    fn frequency_reads_policy0_only() {
        let frequency = cpu_frequency(&fixture("proc"), &fixture("cpu/freq-policy0"))
            .expect("policy0 frequency");
        // 1234567 kHz = 1234.567 MHz, btop's sub-10-GHz branch.
        assert_eq!(frequency.text, "1.2 GHz");
        assert_eq!(frequency.mhz, 1235);

        // policy0 missing: btop never reads policy1 and falls back to cpuinfo.
        let frequency = cpu_frequency(&fixture("proc"), &fixture("cpu/freq-policy1"))
            .expect("cpuinfo fallback");
        assert_eq!(frequency.text, "4.0 GHz");
    }

    #[test]
    fn frequency_formatting_matches_btop() {
        assert_eq!(format_hz(3994.064), "4.0 GHz");
        assert_eq!(format_hz(12345.0), "12 GHz");
        assert_eq!(format_hz(800.0), "800 MHz");
        // btop truncates to three characters, so 9950 MHz becomes "10.".
        assert_eq!(format_hz(9950.0), "10. GHz");
    }

    #[test]
    fn thermal_zone_is_used_when_hwmon_has_no_package_sensor() {
        let sensors = sensors(&fixture("cpu/thermal")).expect("thermal sensor");
        assert_eq!(sensors.sensors[sensors.cpu].0, "thermal0/x86_pkg_temp");
        assert_eq!(sensors.temperature("Auto"), Some(41.0));
        assert_eq!(sensors.crit("Auto"), Some(95));
        // No Core or Tccd labels, so btop sets cpu_temp_only.
        assert!(sensors.cpu_temp_only);
        assert!(core_temperatures(&fixture("proc"), &sensors, 64, "").is_empty());
    }

    #[test]
    fn unavailable_sensors_and_battery_stay_none() {
        let snapshot = collect(&fixture("cpu/unavailable/proc"), &config());
        assert_eq!(snapshot.total_pct, None);
        assert_eq!(snapshot.load_avg, None);
        assert_eq!(snapshot.temperature_c, None);
        assert_eq!(snapshot.temperature_max_c, None);
        assert!(snapshot.core_temperatures_c.is_empty());
        assert_eq!(snapshot.battery_pct, None);
        assert_eq!(snapshot.battery_seconds, None);
        assert_eq!(snapshot.battery_status, None);
        assert_eq!(snapshot.cpu_hz, None);
        assert_eq!(snapshot.name, None);
        assert!(snapshot.fields.is_empty());
    }

    #[test]
    fn battery_capacity_is_the_fallback() {
        let reading = battery(&fixture("cpu/battery-capacity"), "Auto");
        // No energy pair: capacity is read, and AC offline makes it discharge.
        assert_eq!(reading.percent, Some(73.0));
        assert_eq!(reading.status.as_deref(), Some("discharging"));
        assert_eq!(reading.seconds, Some(5400));
    }

    #[test]
    fn battery_missing_is_unavailable() {
        let reading = battery(&fixture("cpu/no-power-supply"), "Auto");
        assert_eq!(reading.percent, None);
        assert_eq!(reading.seconds, None);
        assert_eq!(reading.status, None);
    }

    #[test]
    fn config_keys_change_btop_behaviour() {
        let mut config = config();
        let proc_root = fixture("cpu/config/proc");

        let defaults = collect(&proc_root, &config);
        assert_eq!(defaults.name.as_deref(), Some("Ryzen 9 7950X"));
        assert_eq!(defaults.cpu_hz.as_deref(), Some("4.0 GHz"));
        assert_eq!(defaults.temperature_c, Some(45.0));
        assert_eq!(defaults.temperature_max_c, Some(100.0));
        assert_eq!(defaults.core_temperatures_c, vec![Some(44.0); 2]);
        // Auto prefers the Battery; the UPS supplies 10%.
        assert_eq!(defaults.battery_pct, Some(74.0));
        assert_eq!(defaults.battery_seconds, Some(13_392));
        assert_eq!(defaults.battery_status.as_deref(), Some("discharging"));

        // show_cpu_freq gates the read; the percentages are independent.
        config.show_cpu_freq = false;
        let snapshot = collect(&proc_root, &config);
        assert_eq!(snapshot.cpu_hz, None);
        assert_eq!(snapshot.frequency_mhz, None);
        assert_eq!(snapshot.total_pct, Some(0.0));
        config.show_cpu_freq = true;

        // check_temp gates every temperature.
        config.check_temp = false;
        let snapshot = collect(&proc_root, &config);
        assert_eq!(snapshot.temperature_c, None);
        assert_eq!(snapshot.temperature_max_c, None);
        assert!(snapshot.core_temperatures_c.is_empty());
        config.check_temp = true;

        // show_coretemp keeps the package temperature.
        config.show_coretemp = false;
        let snapshot = collect(&proc_root, &config);
        assert_eq!(snapshot.temperature_c, Some(45.0));
        assert_eq!(snapshot.temperature_max_c, Some(100.0));
        assert!(snapshot.core_temperatures_c.is_empty());
        config.show_coretemp = true;

        // cpu_sensor names a discovered sensor; an unknown name keeps btop's.
        config.cpu_sensor = String::from("coretemp/Core 0");
        let snapshot = collect(&proc_root, &config);
        assert_eq!(snapshot.temperature_c, Some(44.0));
        assert_eq!(snapshot.temperature_max_c, Some(100.0));
        config.cpu_sensor = String::from("missing");
        assert_eq!(collect(&proc_root, &config).temperature_c, Some(45.0));
        config.cpu_sensor = String::from("Auto");

        // cpu_core_map remaps core 0 to sensor 1, which does not exist.
        config.cpu_core_map = String::from("0:1");
        assert_eq!(
            collect(&proc_root, &config).core_temperatures_c,
            vec![None, Some(44.0)]
        );
        config.cpu_core_map = String::new();

        // selected_battery overrides Auto and falls back when unknown.
        config.selected_battery = String::from("UPS0");
        assert_eq!(collect(&proc_root, &config).battery_pct, Some(10.0));
        config.selected_battery = String::from("missing");
        assert_eq!(collect(&proc_root, &config).battery_pct, Some(74.0));
        config.selected_battery = String::from("Auto");

        // show_battery gates every battery field.
        config.show_battery = false;
        let snapshot = collect(&proc_root, &config);
        assert_eq!(snapshot.battery_pct, None);
        assert_eq!(snapshot.battery_seconds, None);
        assert_eq!(snapshot.battery_status, None);
        config.show_battery = true;

        // custom_cpu_name replaces the model name.
        config.custom_cpu_name = String::from("Custom CPU");
        assert_eq!(
            collect(&proc_root, &config).name.as_deref(),
            Some("Custom CPU")
        );
    }

    fn in_container(cgroup: &Path) -> Config {
        let mut config = config();
        config.container = Some(Container {
            runtime: String::from("docker"),
            signal: String::from("1 /.dockerenv"),
            cgroup: Some(cgroup.to_path_buf()),
            own_netns: true,
        });
        config
    }

    #[test]
    fn container_percent_divides_by_elapsed_time_and_effective_cpus() {
        assert_eq!(container_percent(500_000.0, 250_000.0, 2.0), 100.0);
        assert_eq!(container_percent(125_000.0, 250_000.0, 2.0), 25.0);
        assert_eq!(container_percent(900_000.0, 250_000.0, 2.0), 100.0);
        assert_eq!(container_percent(0.0, 0.0, 2.0), 0.0);
    }

    /// The host tree carries loadavg, a cpuinfo frequency, hwmon sensors and
    /// a battery; inside a container none of them is read. This cgroup has no
    /// `cgroup.threads`, so the container's own load is unavailable too.
    #[test]
    fn container_collect_withholds_load_frequency_and_temperature() {
        let cgroup = temp_proc("container-withhold");
        std::fs::write(cgroup.join("cpu.stat"), "usage_usec 0\n").expect("cpu.stat");
        std::fs::write(cgroup.join("cpu.max"), "200000 100000\n").expect("cpu.max");
        std::fs::write(cgroup.join("cpuset.cpus.effective"), "0-3\n").expect("cpuset");
        let config = in_container(&cgroup);
        seed_container(
            &fixture("proc"),
            config.container.as_ref().expect("container"),
            config.update_ms,
        );
        // The cgroup used a lot more than E can in this window: clamped to 100.
        std::fs::write(
            cgroup.join("cpu.stat"),
            "usage_usec 1000000000000\nuser_usec 0\n",
        )
        .expect("cpu.stat");
        let snapshot = collect(&fixture("proc"), &config);
        assert_eq!(snapshot.total_pct, Some(100.0));
        assert_eq!(snapshot.effective_cpus, Some(2.0));
        assert_eq!(snapshot.load_avg, None);
        assert_eq!(snapshot.cores.len(), 4, "cpuset 0-3");
        assert_eq!(snapshot.cpu_hz, None);
        assert_eq!(snapshot.frequency_mhz, None);
        assert_eq!(snapshot.temperature_c, None);
        assert_eq!(snapshot.temperature_max_c, None);
        assert!(snapshot.core_temperatures_c.is_empty());
        assert_eq!(snapshot.battery_pct, None);
        assert_eq!(snapshot.name.as_deref(), Some("Ryzen Threadripper PRO"));
        // user_usec has no earlier reading, nice_usec none at all.
        assert_eq!(snapshot.fields["user"], None);
        assert_eq!(snapshot.fields["idle"], None);
        assert_eq!(snapshot.fields.len(), 10);

        let still = collect(&fixture("proc"), &config);
        assert_eq!(still.total_pct, Some(0.0));
        assert_eq!(still.fields["user"], Some(0.0));
        assert_eq!(still.fields["nice"], None);
        remove_temp_proc(&cgroup);
    }

    /// DEF-CPU-41: one core row per CPU in `cpuset.cpus.effective`, named by
    /// its CPU number; the host tree keeps all 64.
    #[test]
    fn container_cores_are_the_cpuset_cpus() {
        let names = |snapshot: CpuSnapshot| -> Vec<String> {
            snapshot.cores.into_iter().map(|core| core.name).collect()
        };
        let tree = fixture("container/proc");
        let mut container_config = config();
        container_config.container = container::detect(&tree);
        assert_eq!(
            names(collect(&tree, &container_config)),
            ["cpu0", "cpu1", "cpu2", "cpu3"]
        );

        let cgroup = temp_proc("container-cpuset-gaps");
        std::fs::write(cgroup.join("cpuset.cpus.effective"), "1,4-5\n").expect("cpuset");
        let snapshot = collect(&fixture("proc"), &in_container(&cgroup));
        assert!(snapshot.cores.iter().all(|core| core.usage_pct.is_some()));
        assert_eq!(names(snapshot), ["cpu1", "cpu4", "cpu5"]);
        remove_temp_proc(&cgroup);

        assert_eq!(collect(&fixture("proc"), &config()).cores.len(), 64);
    }

    /// DEF-CPU-44: kernel `calc_load` and `/proc/loadavg` digits, by hand, at
    /// the 30 s step (d-44-freq).
    #[test]
    fn container_load_uses_the_kernels_fixed_point_step() {
        // The kernel's rule, FIXED_1 / exp(step / period), at 30 s and at its 5 s.
        let factors = |step: f64| {
            [60.0, 300.0, 900.0]
                .map(|period: f64| (FIXED_1 as f64 * (-step / period).exp()).round() as u64)
        };
        assert_eq!(factors(30.0), EXP);
        assert_eq!(factors(5.0), [1884, 2014, 2037]);
        assert_eq!(LOAD_FREQ, Duration::from_millis(30_004));
        // From 0 with one task: (0 + 2048 * 806 + 2047) / 2048 = 806 -> 0.39.
        assert_eq!(calc_load(0, 1242, 2048), 806);
        assert_eq!(loadavg_value(806), 0.39);
        // From 1.00 with none: 2048 * e / 2048 = e -> 0.61, 0.90, 0.97.
        assert_eq!(calc_load(2048, 1242, 0), 1242);
        assert_eq!(calc_load(2048, 1853, 0), 1853);
        assert_eq!(calc_load(2048, 1981, 0), 1981);
        assert_eq!(loadavg_value(1242), 0.61);
        assert_eq!(loadavg_value(1853), 0.90);
        assert_eq!(loadavg_value(1981), 0.97);
        // A steady count holds: (4096 * 2048 + 2047) / 2048 = 4096 -> 2.00.
        assert_eq!(calc_load(4096, 1242, 4096), 4096);
        assert_eq!(loadavg_value(4096), 2.0);
        assert_eq!(loadavg_value(3072), 1.5);
    }

    /// The recorded container: tids 1 and 7 sleep (S), 8 runs (R) and 42 in
    /// the child cgroup `worker` waits on IO (D); pid 900 runs outside every
    /// container cgroup. Seeded with 2, load per core is 2 / E = 1.00.
    #[test]
    fn container_load_counts_r_and_d_tasks_of_the_cgroup_tree() {
        let tree = fixture("container/proc");
        let cgroup = fixture("container/sys/fs/cgroup");
        assert_eq!(active_tasks(&tree, &cgroup), Some(2));
        let mut config = config();
        config.container = container::detect(&tree);
        seed_container(
            &tree,
            config.container.as_ref().expect("container"),
            config.update_ms,
        );
        let snapshot = collect(&tree, &config);
        assert_eq!(snapshot.load_avg, Some([2.0, 2.0, 2.0]));
        assert_eq!(snapshot.load_per_core(), Some([1.0, 1.0, 1.0]));
        count_container_load(&tree, &cgroup);
        assert_eq!(collect(&tree, &config).load_avg, Some([2.0, 2.0, 2.0]));
        assert_eq!(active_tasks(&tree, &tree.join("missing")), None);
    }

    /// Two tasks seed 2.00; one LOAD_FREQ with none decays each average by
    /// its 30 s factor, 2 * e / 2048: 1.21, 1.81, 1.93. The counting thread is
    /// never counted.
    #[test]
    fn container_load_decays_and_skips_the_counting_thread() {
        let proc_root = temp_proc("container-load");
        let cgroup = proc_root.with_file_name("cgroup");
        std::fs::create_dir_all(&cgroup).expect("cgroup");
        std::fs::write(
            cgroup.join("cgroup.threads"),
            "8
42
",
        )
        .expect("threads");
        let stat = |tid: u32, state: &str| {
            std::fs::create_dir_all(proc_root.join(tid.to_string())).expect("tid");
            let text = format!("{tid} (tokio rt) {state} 1 1\n");
            std::fs::write(proc_root.join(format!("{tid}/stat")), text).expect("stat");
        };
        stat(8, "R");
        stat(42, "D");
        count_container_load(&proc_root, &cgroup);
        stat(8, "S");
        stat(42, "S");
        count_container_load(&proc_root, &cgroup);
        let avenrun = LAST.lock().expect("state")[&cgroup].avenrun;
        assert_eq!(avenrun, Some([2484, 3706, 3962]));
        assert_eq!(
            avenrun.map(|a| a.map(loadavg_value)),
            Some([1.21, 1.81, 1.93])
        );

        stat(8, "R");
        std::os::unix::fs::symlink("8/task/8", proc_root.join("thread-self")).expect("link");
        assert_eq!(active_tasks(&proc_root, &cgroup), Some(0));
        remove_temp_proc(&proc_root);
    }

    #[test]
    fn container_cores_busy_uses_effective_cpus() {
        let snapshot = CpuSnapshot {
            total_pct: Some(50.0),
            effective_cpus: Some(2.0),
            ..CpuSnapshot::default()
        };
        assert_eq!(snapshot.cores_busy(), Some(1.0));
        assert_eq!(snapshot.cores_label().as_deref(), Some("2"));
        let fraction = CpuSnapshot {
            total_pct: Some(50.0),
            effective_cpus: Some(1.5),
            ..CpuSnapshot::default()
        };
        assert_eq!(fraction.cores_busy(), Some(0.75));
        assert_eq!(fraction.cores_label().as_deref(), Some("1.5"));
    }
}

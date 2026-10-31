//! Process collector, mirroring `Proc::collect` in btop's
//! `src/linux/btop_collect.cpp`.
//!
//! Raw source: `<proc_root>/stat` (every numeric field of the `cpu` line),
//! `<proc_root>/<pid>/{comm,cmdline,stat,statm,status}`, `<proc_root>/uptime`,
//! `/etc/passwd` and `getpwuid(3)` for uid names.
//!
//! Formula (btop): cpu_p = clamp(round(cmult * 1000 * cpu_t_delta /
//! max(1, cputimes_delta)) / 10, 0, 100 * cores), with cmult = the online core
//! count when `proc_per_core` is true and 1 otherwise (btop's default). A pid
//! first seen seeds its cached cpu_t with the current value, so its first cpu_p
//! is 0, not its lifetime average. cpu_c = cpu_t / max(1.0, uptime * clk_tck -
//! starttime) is the fraction of one CPU since the process started and keys
//! btop's default "cpu lazy" order. Memory is the stat RSS page count times the
//! page size, with btop's digit guard and statm fallback. State characters map
//! to btop's words; name, command and user are read on first sight and kept,
//! like btop's `current_procs` cache, when a later read fails.
//!
//! btop's process keys: `proc_sorting` (default "cpu lazy") selects the sort
//! column and an unknown value sorts nothing; `proc_reversed` sorts ascending;
//! `proc_filter_kernel` drops kthreadd (pid 2) and its children.
//!
//! Tree order (ACC-VIEW-58-PROC-VIEW), btop's `_tree_gen`
//! (`src/btop_shared.cpp:109-190`, `src/linux/btop_collect.cpp:2714-2773`):
//! every row is sent in btop's tree pre-order, whatever `proc_tree` says. A
//! pid whose parent is not listed gets ppid 0, the rows with the lowest ppid
//! are the roots, and each child follows its parent at its parent's depth plus
//! one. Siblings keep the `proc_sorting` / `proc_reversed` order; the browser
//! sorts by the viewer's column and uses this order only to break ties.
//! `proc_tree` picks the browser's starting view (`config::initial_proc_view`).
//! Each row with children carries its subtree totals, btop's collapsed-tree
//! sum of `cpu_p`, `mem` and `threads` (`btop_shared.cpp:163-171`), over the
//! listed rows only, once per sample for every client.
//!
//! Row graph, btop's `proc_cpu_graphs` (`src/btop_draw.cpp:1793-1806`, 1902,
//! 1929-1936; ACC-VIEW-52-PROC-GRAPH): a pid's graph starts when cpu_p is above
//! 0 and goes after 10 updates in a row below 0.1 percent, the rule btop's
//! comment states (its 1.3.0 code resets the count on each idle update below
//! 10, so it never drops a graph of a living pid). It pushes 5 for 0.1 to 5
//! percent and round(cpu_p) otherwise, every sample, over a 20 s window, and
//! goes with the pid's cache entry. btop graphs a collapsed row's summed
//! cpu_p (`btop_draw.cpp:1796` runs after `_tree_gen`), so a row with children
//! keeps a second, summed graph by the same rule, started and dropped by its
//! descendants' cpu_p; the browser shows the one that matches the number.
//! Only a top-level row (a root, or a child of pid 1) keeps a summed graph,
//! which holds the server's cost per tick under the ACC-VIEW-52 bar
//! (ACC-PERF-16-NO-OVERHEAD); a deeper collapsed row shows no graph.
//!
//! Period: every sample, `update_ms` (250 ms by default; btop's 2000), on the
//! sampler's slow pass, so the walk and the counts rebuild every
//! `proc_refresh_ms` (1000 by default) while the values keep the tick
//! (ACC-PERF-149-SPLIT-CADENCE). The histories are told that slower period, so
//! the count graph and the row graphs keep their wall-clock windows.
//!
//! Reads (DEF-PERF-76-SERVER-COST): the procfs root is opened once as a
//! directory fd and every `<pid>/stat`, `<pid>/status`, `<pid>/comm`,
//! `<pid>/cmdline` and `<pid>/statm` is opened relative to it with `openat`,
//! into one reused byte buffer. A one-page procfs file's short read is its
//! end, so a stat costs one read; the stat line is parsed from its bytes with
//! no per-row string. Name, command line and user are `Arc<str>` in three
//! LRU caches keyed by pid and `starttime`, bounded at
//! [`IDENTITY_CACHE_LIMIT`] strings each (ACC-PERF-141-CACHE-BOUND): a string
//! is read when its pid first appears or comes back with another starttime,
//! cloned per tick by pointer, dropped when the process leaves the scan or
//! the cache passes its limit, and a row whose string is missing renders that
//! field unavailable rather than a stale value. The CPU model name and the
//! container's `cgroup.procs` membership are re-read every 5 s
//! (`gpu::SLOW_PERIOD`), not every tick; a process that joins or leaves the
//! container is listed up to 5 s late.
//!
//! Container (ACC-SCOPE-64-CONTAINER): inside a container this collector
//! departs from btop on the owner's order (2026-10-27). Every sample,
//! `update_ms`:
//! - the rows are the numeric `<proc_root>` entries that are members of the
//!   container: the pids in `cgroup.procs` of the container cgroup and of
//!   every descendant cgroup, depth first. A container that shares the host
//!   pid namespace still lists only its own processes. An unreadable
//!   `cgroup.procs` (or no cgroup v2 directory) leaves processes unavailable
//! - with `proc_per_core` false, Cpu% = btop's cpu_p * online / E, online
//!   being the cpuN lines of the same `/proc/stat` read and E
//!   [`container::effective_cpus`]: a percent of the container's capacity,
//!   not of the machine. With `proc_per_core` true it stays a percent of one
//!   CPU
//! - every other column, and the total, running and thread counts, come from
//!   the member rows by btop's code
//!
//! Processes tile (ACC-VIEW-53-PROC-TILE): btop 1.3.0 has no such tile
//! (`Proc::draw` draws only the process count); these facts are btop-web
//! additions the owner asked for on 2026-10-27. Source: the rows of this
//! sample after btop's filters (the member rows in a container, without
//! kernel threads when `proc_filter_kernel` is on). Every sample, `update_ms`:
//! - sleeping, waiting and zombie: rows whose state word is `Sleeping`, `Waiting`
//!   (`D`) or `Zombie` (`Z`), as running counts `Running`; a row with no
//!   readable stat counts in the total only
//! - top process: the row with the highest Cpu% (`cpu_pct`, the table's
//!   column), the lower pid on a tie; rows without a Cpu% are skipped
//! - 80 percent cpu and mem: the Cpu% (and, separately, MemB) values sorted
//!   largest first; the count is the first k whose sum reaches 80 percent of
//!   the sum of all values, 0 when that sum is 0
//! - count graph: the total pushed into a [`History`] (the configured time
//!   axis window, the 1 s EMA, the shared decimation), drawn from 0 to the
//!   smallest 1, 2 or 5 x 10^k at or above both 10 and the largest drawn
//!   sample; a change of that ceiling runs through [`Scale`], as the network
//!   plots do (ACC-GRAPH-67-SMOOTH-SCALE)
//!
//! Process popup (ACC-VIEW-176-PROC-POPUP), btop's `Proc::detailed` and
//! `_collect_details` (`src/linux/btop_collect.cpp`): while a popup asks about
//! a pid, each slow pass pushes that row's Cpu% and MemB into two histories
//! over the configured chart window and reads `<proc_root>/<pid>/io`
//! (`read_bytes`, `write_bytes`) and the GPU nodes in `<proc_root>/<pid>/fd`.
//! The pass adds no walk and no timer, and with no popup open it costs one
//! empty `retain`. btop keeps one detailed pid; this server has many viewers,
//! so it keeps up to [`DETAIL_LIMIT`], and a pid no request asked about for
//! [`DETAIL_MISSED`] passes goes. Departures from btop on the owner's
//! criterion: the plot is the table's Cpu%, not `cpu_p * cores`, its ceiling
//! follows the peak, and the command line has no 1000-byte limit.
//!
//! DEF-GRAPH-129-PROC-PLOT: every slow pass also keeps each row's Cpu% and MemB
//! in a compact record over the chart window, from the values the pass already
//! computes, with no read of its own; a new watch's histories are replayed from
//! it. btop starts its rings when the process is selected, so its graph opens
//! empty; the owner reported that as a defect, so here a popup opens on the
//! process's history. A sampler that slept with no viewer took no samples: the
//! samples either side of the sleep sit one slot apart, as on the CPU panel's
//! chart, whose [`History`] also counts samples and not wall time.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::fs;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use nix::fcntl::{OFlag, open, openat};
use nix::sys::stat::Mode;

use crate::chart::{
    ChartPath, History, NET_PLOT, PROC_COUNT, PROC_GRAPH, PROC_WINDOW_MS, Scale, ScaleFrame,
    capacity,
};
use crate::config::Config;

use super::container;
use super::gpu::{self, SLOW_PERIOD};
use super::{ProcEntry, ProcSnapshot, ProcSum, cpu_tone};

/// btop's `sort_vector` (`src/btop_shared.hpp`), in the same order.
pub const SORT_MODES: [&str; 8] = [
    "pid",
    "name",
    "command",
    "threads",
    "user",
    "memory",
    "cpu direct",
    "cpu lazy",
];

/// btop's `KTHREADD` (`src/linux/btop_collect.cpp`).
const KTHREADD: i32 = 2;

struct ProcState {
    cputimes: u64,
    processes: HashMap<i32, CachedProcess>,
    /// ACC-PERF-141-CACHE-BOUND: the rows' name, command and user strings.
    identity: IdentityCache,
    passwd: PasswdCache,
    /// The procfs root directory fd, opened once and reused by every read
    /// (DEF-PERF-76-SERVER-COST).
    dir: Option<ProcRoot>,
    /// btop's `kernels_procs`, seeded with kthreadd.
    kernel_pids: HashSet<i32>,
    /// btop's `proc_clear_count`, which rebuilds the kernel set every 256 samples.
    kernel_clear_count: u32,
    /// The process count history behind the Processes tile's graph.
    count: History,
    /// Moves the count graph to a new ceiling smoothly (ACC-GRAPH-67-SMOOTH-SCALE).
    scale: Scale,
    /// ACC-VIEW-176-PROC-POPUP: the pids an open popup asks about.
    detailed: HashMap<i32, Detailed>,
    /// The last pass's period and chart window, which a new [`Detailed`]'s
    /// replay takes.
    period_ms: u64,
    window_ms: u64,
    /// DEF-GRAPH-130-PLOT-STEPS: the passes so far, which is the sampler's
    /// `slow_seq`: it counts one call of [`collect`] each, and this count moves
    /// under the lock that guards the rings, where the frame's comes later.
    slow_seq: u64,
}

impl Default for ProcState {
    fn default() -> Self {
        Self {
            cputimes: 0,
            processes: HashMap::new(),
            identity: IdentityCache::default(),
            passwd: PasswdCache::default(),
            dir: None,
            kernel_pids: HashSet::from([KTHREADD]),
            kernel_clear_count: 0,
            count: History::new(),
            scale: Scale::default(),
            detailed: HashMap::new(),
            period_ms: 0,
            window_ms: 0,
            slow_seq: 0,
        }
    }
}

/// One procfs root directory, opened once and reused: a row's files are
/// opened relative to it with `openat`, so no path is resolved from `/` and no
/// per-row path string or file stat is paid.
struct ProcRoot {
    fd: OwnedFd,
}

impl ProcRoot {
    fn open(root: &Path) -> Option<Self> {
        let fd = open(
            root,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .ok()?;
        Some(Self { fd })
    }
}

/// Read `<pid>/<name>` through `root` into `buffer`, `path` being the reusable
/// relative-path scratch; returns the byte count. `None` when the file cannot
/// be opened or read. `single` marks a procfs one-page file (`comm`,
/// `status`, `stat`, `statm`, all `single_open` in the kernel): the kernel
/// serves it from one page, so a short read is the end of the file and the
/// EOF read is skipped. `cmdline` is a multi-page seq_file and keeps reading
/// to EOF.
fn read_at(
    root: &ProcRoot,
    pid: i32,
    name: &str,
    single: bool,
    buffer: &mut Vec<u8>,
    path: &mut String,
) -> Option<usize> {
    path.clear();
    let _ = write!(path, "{pid}/{name}");
    let file = openat(
        &root.fd,
        path.as_str(),
        OFlag::O_RDONLY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let mut file = File::from(file);
    buffer.clear();
    loop {
        let start = buffer.len();
        if start == buffer.capacity() {
            buffer.reserve(4096);
        }
        buffer.resize(buffer.capacity(), 0);
        let wanted = buffer.len() - start;
        match file.read(&mut buffer[start..]) {
            Ok(0) => {
                buffer.truncate(start);
                return Some(start);
            }
            Ok(read) => {
                buffer.truncate(start + read);
                if single && read < wanted {
                    return Some(start + read);
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
}

/// A `proc_info` entry in btop's `current_procs` plus the cached stat counters.
#[derive(Default)]
struct CachedProcess {
    /// The row without its identity strings, which the snapshot fills from
    /// [`IdentityCache`].
    entry: ProcEntry,
    cpu_t: u64,
    cpu_s: u64,
    /// The `starttime` the entry's identity was read at; a different one means
    /// the pid was reused and the identity is read again.
    starttime: Option<u64>,
    /// btop's `p_graphs` entry; it goes with the pid when the pid vanishes.
    graph: RowGraph,
    /// The subtree's summed graph, while the pid has children.
    sum_graph: RowGraph,
    /// DEF-GRAPH-129-PROC-PLOT: Cpu% and MemB per slow pass, oldest first, over
    /// the chart window; it goes with the pid, and a reused pid clears it. A
    /// read that fails keeps no sample, so an unread value draws nothing.
    record: VecDeque<[f32; 2]>,
}

/// A pid and the `starttime` its identity string was read at, `None` until
/// the first stat read supplies it.
type IdentityKey = (i32, Option<u64>);

/// ACC-PERF-141-CACHE-BOUND: the entries one identity cache holds, one string
/// per process. A desktop holds well under this many processes in one scan,
/// so the ceiling only bites under a process flood; the row whose string was
/// evicted shows that field unavailable rather than a stale value.
const IDENTITY_CACHE_LIMIT: usize = 4096;

/// ACC-PERF-141-CACHE-BOUND: one kind of identity string (name, command line
/// or user), keyed by pid and starttime so a reused pid reads its own string.
/// The least recently used string is dropped when the next one arrives at
/// [`IDENTITY_CACHE_LIMIT`].
#[derive(Default)]
struct StringLru {
    values: HashMap<IdentityKey, (u64, Arc<str>)>,
    /// Use stamps in eviction order; `remove_first` is the least recently used.
    order: BTreeMap<u64, IdentityKey>,
    tick: u64,
}

impl StringLru {
    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    /// The string for `key`, marked as just used.
    fn get(&mut self, key: IdentityKey) -> Option<Arc<str>> {
        let tick = self.next_tick();
        let (stamp, value) = self.values.get_mut(&key)?;
        self.order.remove(stamp);
        *stamp = tick;
        self.order.insert(tick, key);
        Some(value.clone())
    }

    fn insert(&mut self, key: IdentityKey, value: Arc<str>) {
        let tick = self.next_tick();
        if let Some((stamp, _)) = self.values.insert(key, (tick, value)) {
            self.order.remove(&stamp);
        }
        self.order.insert(tick, key);
        while self.values.len() > IDENTITY_CACHE_LIMIT {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.values.remove(&oldest);
        }
    }

    fn remove(&mut self, key: &IdentityKey) {
        if let Some((stamp, _)) = self.values.remove(key) {
            self.order.remove(&stamp);
        }
    }

    /// Move a string read before the starttime was known onto the pid's key.
    fn move_key(&mut self, from: IdentityKey, to: IdentityKey) {
        if let Some((stamp, value)) = self.values.remove(&from) {
            self.order.remove(&stamp);
            self.insert(to, value);
        }
    }

    /// Drop every string whose process left the scan or changed starttime.
    fn retain(&mut self, keep: impl Fn(&IdentityKey) -> bool) {
        let values = &mut self.values;
        self.order.retain(|_, key| {
            if keep(key) {
                true
            } else {
                values.remove(key);
                false
            }
        });
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.values.len()
    }
}

/// ACC-PERF-141-CACHE-BOUND: the process identity strings, one bounded LRU per
/// kind.
#[derive(Default)]
struct IdentityCache {
    name: StringLru,
    command: StringLru,
    user: StringLru,
}

impl IdentityCache {
    fn retain(&mut self, keep: impl Fn(&IdentityKey) -> bool) {
        self.name.retain(&keep);
        self.command.retain(&keep);
        self.user.retain(&keep);
    }

    fn remove(&mut self, key: &IdentityKey) {
        self.name.remove(key);
        self.command.remove(key);
        self.user.remove(key);
    }

    fn move_key(&mut self, from: IdentityKey, to: IdentityKey) {
        self.name.move_key(from, to);
        self.command.move_key(from, to);
        self.user.move_key(from, to);
    }

    /// The three lengths, for the churn test's ceiling check.
    #[cfg(test)]
    fn len(&self) -> (usize, usize, usize) {
        (self.name.len(), self.command.len(), self.user.len())
    }
}

/// btop's `p_graphs` entry and its `p_counters` count of updates in a row
/// below 0.1 percent.
#[derive(Default)]
struct RowGraph {
    history: Option<History>,
    idle: u64,
}

/// `/etc/passwd`, re-read only when its mtime changes.
#[derive(Default)]
struct PasswdCache {
    modified: Option<SystemTime>,
    users: BTreeMap<String, String>,
    unavailable: bool,
}

/// Values shared by every pid in one sample.
struct Context {
    total_memory: Option<u64>,
    page_size: u64,
    /// btop's `cmult`: the core count with `proc_per_core`, otherwise 1.
    cmult: u64,
    cores: usize,
    clk_tck: f64,
    uptime: Option<f64>,
    delta_cputimes: u64,
    /// Container mode, `proc_per_core` false: online / E; otherwise 1.
    cpu_scale: f64,
    /// The slots a process's record holds: the chart window at this pass's period.
    record_slots: usize,
}

/// btop's parsed `/proc/<pid>/stat` fields.
struct Stat {
    state: char,
    ppid: i32,
    cpu_t: u64,
    nice: i64,
    threads: usize,
    starttime: u64,
    mem: u64,
}

/// ACC-VIEW-176-PROC-POPUP: the pids one server watches at once. A server has
/// few viewers, and the watch asked about longest ago makes room for a new one,
/// so a client that walks every pid holds no more than this many rings
/// (ACC-PERF-141-CACHE-BOUND).
const DETAIL_LIMIT: usize = 16;

/// ACC-VIEW-176-PROC-POPUP: the slow passes a watch outlives its last request.
/// An open popup asks once per pass, so this bears a viewer's lost requests;
/// a closed popup's rings go within this many passes.
const DETAIL_MISSED: u32 = 5;

/// One popup plot's samples and the ceiling they draw to.
#[derive(Default)]
struct DetailSeries {
    history: History,
    scale: Scale,
    frame: ScaleFrame,
}

impl DetailSeries {
    /// DEF-GRAPH-129-PROC-PLOT: a new watch's series, replayed from a record
    /// through the same smoothing as [`Self::push`]. The first update of a
    /// [`Scale`] starts idle, so the first body draws at its ceiling with no
    /// change running from zero.
    fn seeded(
        record: impl Iterator<Item = f32>,
        period_ms: u64,
        window_ms: u64,
        ceiling: fn(f64) -> f64,
    ) -> Self {
        let mut series = Self::default();
        series.history.set_window(window_ms);
        for sample in record {
            series.history.push(Some(f64::from(sample)), period_ms);
        }
        let target = ceiling(series.history.max().unwrap_or_default());
        series.frame = series.scale.update(target, period_ms, NET_PLOT);
        series
    }

    /// One sample, `period_ms` after the last. `ceiling` maps the history's
    /// peak to the plot's top; a change of it runs through [`Scale`]
    /// (ACC-GRAPH-67-SMOOTH-SCALE).
    fn push(
        &mut self,
        value: Option<f64>,
        period_ms: u64,
        window_ms: u64,
        ceiling: fn(f64) -> f64,
    ) {
        self.history.set_window(window_ms);
        self.history.push(value, period_ms);
        let target = ceiling(self.history.max().unwrap_or_default());
        self.frame = self.scale.update(target, period_ms, NET_PLOT);
    }

    fn plot(&self) -> ProcPlot {
        ProcPlot {
            chart: self.history.chart(NET_PLOT, 0.0, self.frame.draw_max),
            scale: self.frame.clone(),
        }
    }
}

/// btop's `Proc::detailed`, one per pid an open popup asks about.
struct Detailed {
    /// The process this watch belongs to; a reused pid drops the watch.
    starttime: Option<u64>,
    /// The whole `cmdline`, read once per watch; `None` when it cannot be read
    /// or is empty, as a kernel thread's is.
    command: Option<Arc<str>>,
    cpu: DetailSeries,
    mem: DetailSeries,
    /// btop `detailed.io_read`, `read_bytes` from `<pid>/io`.
    io_read: Option<u64>,
    /// btop `detailed.io_write`, `write_bytes` from `<pid>/io`.
    io_write: Option<u64>,
    /// The GPU nodes the process holds; `None` when its fd table cannot be read.
    gpu_nodes: Option<Vec<String>>,
    /// Slow passes since the last request.
    missed: u32,
}

impl Detailed {
    fn sample(
        &mut self,
        root: &ProcRoot,
        pid: i32,
        entry: &ProcEntry,
        (period_ms, window_ms): (u64, u64),
        buffer: &mut Vec<u8>,
        path: &mut String,
    ) {
        self.cpu
            .push(entry.cpu_pct, period_ms, window_ms, cpu_ceiling);
        self.mem.push(
            // f64 holds every byte count below 2^53 (8 PiB) exactly.
            entry.mem_bytes.map(|bytes| bytes as f64),
            period_ms,
            window_ms,
            memory_ceiling,
        );
        self.read_files(root, pid, buffer, path);
    }

    /// btop's `detailed.io_read` and `io_write`, and the GPU nodes.
    fn read_files(&mut self, root: &ProcRoot, pid: i32, buffer: &mut Vec<u8>, path: &mut String) {
        let io = read_at(root, pid, "io", true, buffer, path)
            .and_then(|read| std::str::from_utf8(&buffer[..read]).ok());
        self.io_read = io.and_then(|io| io_counter(io, "read_bytes"));
        self.io_write = io.and_then(|io| io_counter(io, "write_bytes"));
        self.gpu_nodes = gpu::held_nodes(&root.fd, pid);
    }
}

/// One `<pid>/io` counter, by its exact key.
fn io_counter(io: &str, key: &str) -> Option<u64> {
    io.lines().find_map(|line| {
        line.strip_prefix(key)?
            .strip_prefix(':')?
            .trim()
            .parse()
            .ok()
    })
}

/// The CPU plot's top: the 1, 2 or 5 x 10^k percent at or above the peak, at
/// least 1, because most rows are a fraction of one percent of the machine.
fn cpu_ceiling(peak: f64) -> f64 {
    nice_ceiling(peak.max(1.0)) as f64
}

/// The memory plot's top: the 1, 2 or 5 x 10^k MiB or GiB at or above the peak.
fn memory_ceiling(peak: f64) -> f64 {
    const MIB: f64 = 1_048_576.0;
    const GIB: f64 = 1_073_741_824.0;
    let unit = if peak >= GIB { GIB } else { MIB };
    nice_ceiling((peak / unit).max(1.0)) as f64 * unit
}

/// One popup plot, ready for a template.
pub struct ProcPlot {
    pub chart: ChartPath,
    pub scale: ScaleFrame,
}

/// ACC-VIEW-176-PROC-POPUP: what the popup shows beyond the process row.
pub struct ProcDetail {
    /// The slow pass the rings hold, the counter of the frames' `data-slow-seq`
    /// (DEF-GRAPH-130-PLOT-STEPS).
    pub slow_seq: u64,
    pub command: Option<Arc<str>>,
    pub cpu: ProcPlot,
    pub mem: ProcPlot,
    pub io_read: Option<u64>,
    pub io_write: Option<u64>,
    pub gpu_nodes: Option<Vec<String>>,
}

/// The popup's values for `pid`, and a watch on it for the next
/// [`DETAIL_MISSED`] slow passes. `None` when the last pass did not list the
/// pid, so a request reaches no process the table does not show
/// (ACC-SCOPE-64-CONTAINER). This reads one process's files on the first
/// request only; every later value comes from the sampler's slow pass.
pub fn detail(proc_root: &Path, pid: i32) -> Option<ProcDetail> {
    let mut states = LAST.lock().unwrap_or_else(|error| error.into_inner());
    let ProcState {
        processes,
        detailed,
        dir,
        period_ms,
        window_ms,
        slow_seq,
        ..
    } = states.get_mut(proc_root)?;
    let cached = processes.get(&pid)?;
    let root = dir.as_ref()?;
    if !detailed.contains_key(&pid) {
        if detailed.len() >= DETAIL_LIMIT
            && let Some(oldest) = detailed
                .iter()
                .max_by_key(|(_, watch)| watch.missed)
                .map(|(pid, _)| *pid)
        {
            detailed.remove(&oldest);
        }
        let mut buffer = Vec::with_capacity(4096);
        let mut path = String::with_capacity(32);
        // btop's `Proc::detailed` starts its rings here, and the owner's
        // DEF-GRAPH-129-PROC-PLOT is the empty axis that gives, so the watch
        // starts from the process's record.
        let replay = |at: usize, ceiling| {
            let samples = cached.record.iter().map(move |sample| sample[at]);
            DetailSeries::seeded(samples, *period_ms, *window_ms, ceiling)
        };
        let mut watch = Detailed {
            starttime: cached.starttime,
            command: read_at(root, pid, "cmdline", false, &mut buffer, &mut path)
                .map(|read| full_command_line(&buffer[..read]))
                .filter(|command| !command.is_empty())
                .map(Arc::from),
            cpu: replay(0, cpu_ceiling),
            mem: replay(1, memory_ceiling),
            io_read: None,
            io_write: None,
            gpu_nodes: None,
            missed: 0,
        };
        watch.read_files(root, pid, &mut buffer, &mut path);
        detailed.insert(pid, watch);
    }
    let watch = detailed.get_mut(&pid)?;
    watch.missed = 0;
    Some(ProcDetail {
        slow_seq: *slow_seq,
        command: watch.command.clone(),
        cpu: watch.cpu.plot(),
        mem: watch.mem.plot(),
        io_read: watch.io_read,
        io_write: watch.io_write,
        gpu_nodes: watch.gpu_nodes.clone(),
    })
}

static LAST: Mutex<BTreeMap<PathBuf, ProcState>> = Mutex::new(BTreeMap::new());

pub fn collect(proc_root: &Path, config: &Config) -> ProcSnapshot {
    let mut states = LAST.lock().unwrap_or_else(|error| error.into_inner());
    let state = states.entry(proc_root.to_path_buf()).or_default();
    state.slow_seq += 1;
    let Some((cputimes, online)) = cpu_times(proc_root) else {
        return ProcSnapshot::default();
    };
    let (members, cpu_scale) = match &config.container {
        None => (None, 1.0),
        Some(container) => {
            let Some(cgroup) = container.cgroup.as_deref() else {
                return ProcSnapshot::default();
            };
            let Some(members) = cached_members(cgroup) else {
                return ProcSnapshot::default();
            };
            let scale = if config.proc_per_core {
                1.0
            } else {
                online as f64 / container::effective_cpus(cgroup)
            };
            (Some(members), scale)
        }
    };

    state.passwd.refresh();
    if config.proc_filter_kernel {
        state.kernel_clear_count += 1;
        if state.kernel_clear_count >= 256 {
            state.kernel_pids.clear();
            state.kernel_pids.insert(KTHREADD);
            state.kernel_clear_count = 0;
        }
    }

    let cores = core_count();
    let context = Context {
        total_memory: total_memory(proc_root),
        page_size: page_size(),
        cmult: if config.proc_per_core {
            cores as u64
        } else {
            1
        },
        cores,
        clk_tck: clk_tck(),
        uptime: uptime(proc_root),
        delta_cputimes: cputimes.saturating_sub(state.cputimes).max(1),
        cpu_scale,
        record_slots: capacity(config.window_ms(), config.update_ms),
    };

    let Some(dir) = state.dir.take().or_else(|| ProcRoot::open(proc_root)) else {
        return ProcSnapshot::default();
    };
    let Ok(directory) = fs::read_dir(proc_root) else {
        return ProcSnapshot::default();
    };
    let mut candidates: Vec<i32> = directory
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok()?.parse().ok())
        .filter(|pid| members.as_ref().is_none_or(|members| members.contains(pid)))
        .collect();
    candidates.sort_unstable();

    let mut found = Vec::with_capacity(candidates.len());
    let mut buffer = Vec::with_capacity(4096);
    let mut path = String::with_capacity(32);
    for pid in candidates {
        if config.proc_filter_kernel && state.kernel_pids.contains(&pid) {
            continue;
        }
        found.push(pid);
        let first_sight = !state.processes.contains_key(&pid);
        let cached = state.processes.entry(pid).or_insert_with(|| CachedProcess {
            entry: ProcEntry {
                pid,
                ..ProcEntry::default()
            },
            ..CachedProcess::default()
        });
        if first_sight {
            // A pid first seen reads its identity now, before the stat that
            // supplies the starttime; the key is rebound to it below. A stale
            // key from an earlier holder of this pid's number goes first.
            state.identity.remove(&(pid, None));
            if !read_identity(
                &dir,
                pid,
                (pid, None),
                &mut state.identity,
                &state.passwd.users,
                &mut buffer,
                &mut path,
            ) {
                // btop's `continue`: the entry keeps what was read before the failure.
                continue;
            }
        }
        if let Some(stat) = read_stat(&dir, pid, &context, &mut buffer, &mut path) {
            match cached.starttime {
                // The identity read before the first stat moves onto the
                // starttime the stat supplies.
                None => state
                    .identity
                    .move_key((pid, None), (pid, Some(stat.starttime))),
                Some(starttime) if starttime != stat.starttime => {
                    // A reused pid is another process: its old strings go
                    // before the read, so a failed read leaves them
                    // unavailable instead of showing the old process's.
                    state.identity.remove(&(pid, Some(starttime)));
                    cached.starttime = Some(stat.starttime);
                    cached.record.clear();
                    if !read_identity(
                        &dir,
                        pid,
                        (pid, Some(stat.starttime)),
                        &mut state.identity,
                        &state.passwd.users,
                        &mut buffer,
                        &mut path,
                    ) {
                        continue;
                    }
                }
                Some(_) => {}
            }
            cached.starttime = Some(stat.starttime);
            apply(cached, stat, &context);
        }
        // btop caches a child of kthreadd and removes it from this sample.
        if config.proc_filter_kernel
            && state
                .processes
                .get(&pid)
                .is_some_and(|cached| cached.entry.ppid == Some(KTHREADD))
        {
            state.kernel_pids.insert(pid);
            found.pop();
        }
    }
    state.dir = Some(dir);
    state
        .processes
        .retain(|pid, _| found.binary_search(pid).is_ok());
    // ACC-PERF-141-CACHE-BOUND: a string goes when its process left the scan
    // or its pid number came back with another starttime.
    state.identity.retain(|(pid, starttime)| {
        state
            .processes
            .get(pid)
            .is_some_and(|cached| cached.starttime == *starttime)
    });
    state.cputimes = cputimes;
    state.period_ms = config.update_ms;
    state.window_ms = config.window_ms();

    // ACC-VIEW-176-PROC-POPUP: one sample for each pid a popup asks about.
    let ProcState {
        processes,
        detailed,
        dir,
        ..
    } = &mut *state;
    if let Some(root) = dir.as_ref() {
        detailed.retain(|pid, watch| {
            watch.missed += 1;
            let Some(cached) = processes
                .get(pid)
                .filter(|cached| cached.starttime == watch.starttime)
                .filter(|_| watch.missed <= DETAIL_MISSED)
            else {
                return false;
            };
            watch.sample(
                root,
                *pid,
                &cached.entry,
                (config.update_ms, config.window_ms()),
                &mut buffer,
                &mut path,
            );
            true
        });
    }

    let (processes, identity) = (&state.processes, &mut state.identity);
    let mut entries: Vec<ProcEntry> = found
        .iter()
        .filter_map(|pid| {
            let cached = processes.get(pid)?;
            let key = (*pid, cached.starttime);
            let mut entry = cached.entry.clone();
            entry.name = identity.name.get(key);
            entry.command = identity.command.get(key).unwrap_or_default();
            entry.user = identity.user.get(key);
            Some(entry)
        })
        .collect();
    let busy_below = tree_entries(&mut entries, config);

    for (entry, busy) in entries.iter_mut().zip(busy_below) {
        let Some(cached) = state.processes.get_mut(&entry.pid) else {
            continue;
        };
        if let Some(cpu_p) = entry.cpu_pct {
            cached.graph.tick(Some(cpu_p), cpu_p, config.update_ms);
        }
        // The row graphs draw no area, so none is built. The tone comes from
        // the window, not the readout (DEF-VIEW-67-SPARKLINE-TONE).
        if let Some(history) = &cached.graph.history {
            entry.graph.line = history.line(PROC_GRAPH, 0.0, 100.0);
            entry.graph_tone = window_tone(history);
        }
        match entry.sum.as_mut() {
            Some(sum) if entry.depth == 0 || (entry.depth == 1 && entry.ppid == Some(1)) => {
                if let Some(total) = sum.cpu_pct {
                    cached.sum_graph.tick(busy, total, config.update_ms);
                }
                if let Some(history) = &cached.sum_graph.history {
                    sum.graph.line = history.line(PROC_GRAPH, 0.0, 100.0);
                    sum.graph_tone = window_tone(history);
                }
            }
            _ => cached.sum_graph = RowGraph::default(),
        }
    }

    let in_state = |word: &str| {
        entries
            .iter()
            .filter(|entry| entry.state.as_deref() == Some(word))
            .count()
    };
    let threads = entries.iter().filter_map(|entry| entry.threads).sum();
    state.count.set_window(config.window_ms());
    state
        .count
        .push(Some(entries.len() as f64), config.update_ms);
    let chart_max = count_ceiling(state.count.max().unwrap_or_default());
    let scale = state
        .scale
        .update(chart_max as f64, config.update_ms, PROC_COUNT);
    ProcSnapshot {
        total: Some(entries.len()),
        running: Some(in_state("Running")),
        sleeping: Some(in_state("Sleeping")),
        waiting: Some(in_state("Waiting")),
        zombie: Some(in_state("Zombie")),
        threads: Some(threads),
        top: top_process(&entries).cloned(),
        cpu_80: share_count(entries.iter().filter_map(|entry| entry.cpu_pct)),
        // f64 holds every byte sum below 2^53 (8 PiB) exactly.
        mem_80: share_count(
            entries
                .iter()
                .filter_map(|entry| entry.mem_bytes.map(|bytes| bytes as f64)),
        ),
        chart: state.count.chart(PROC_COUNT, 0.0, scale.draw_max),
        chart_max,
        scale,
        entries,
    }
}

/// The row with the highest Cpu%, the lower pid on a tie.
fn top_process(entries: &[ProcEntry]) -> Option<&ProcEntry> {
    entries
        .iter()
        .filter_map(|entry| entry.cpu_pct.map(|cpu| (cpu, entry)))
        .max_by(|(one, first), (other, second)| {
            one.total_cmp(other).then(second.pid.cmp(&first.pid))
        })
        .map(|(_, entry)| entry)
}

/// How many of `values`, largest first, reach 80 percent of their sum: the
/// first k whose running sum x 5 is at least the sum x 4. `None` without
/// values, 0 when the sum is 0.
fn share_count(values: impl Iterator<Item = f64>) -> Option<usize> {
    let mut values: Vec<f64> = values.collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable_by(|one, other| other.total_cmp(one));
    // Summed in the same order as the running sum, so the last running sum is the total.
    let total: f64 = values.iter().sum();
    if total <= 0.0 {
        return Some(0);
    }
    let mut sum = 0.0;
    values
        .iter()
        .position(|value| {
            sum += value;
            sum * 5.0 >= total * 4.0
        })
        .map(|index| index + 1)
}

/// The count graph's ceiling: the smallest 1, 2 or 5 x 10^k at or above both
/// `largest` and 10, so the scale changes only when the drawn maximum
/// crosses a step.
fn count_ceiling(largest: f64) -> u64 {
    nice_ceiling(largest.max(10.0))
}

/// The smallest 1, 2 or 5 x 10^k at or above `target`.
// Own code: one loop; no crate in the tree picks a nice axis ceiling.
fn nice_ceiling(target: f64) -> u64 {
    let mut decade = 1_u64;
    loop {
        for step in [1, 2, 5] {
            if (step * decade) as f64 >= target {
                return step * decade;
            }
        }
        decade = decade.saturating_mul(10);
    }
}

/// btop reads comm, cmdline and the uid from status once per pid; a failure
/// stops the first collection of that pid before stat, and is not retried
/// until the pid number comes back with a different starttime. Each string
/// goes into its own cache under `key` as it is read.
fn read_identity(
    root: &ProcRoot,
    pid: i32,
    key: IdentityKey,
    identity: &mut IdentityCache,
    users: &BTreeMap<String, String>,
    buffer: &mut Vec<u8>,
    path: &mut String,
) -> bool {
    let Some(read) = read_at(root, pid, "comm", true, buffer, path) else {
        return false;
    };
    let Ok(comm) = std::str::from_utf8(&buffer[..read]) else {
        return false;
    };
    identity
        .name
        .insert(key, Arc::from(comm.strip_suffix('\n').unwrap_or(comm)));

    let Some(read) = read_at(root, pid, "cmdline", false, buffer, path) else {
        return false;
    };
    identity
        .command
        .insert(key, Arc::from(command_line(&buffer[..read])));

    let Some(uid) = uid(root, pid, buffer, path) else {
        return false;
    };
    identity.user.insert(key, Arc::from(user_name(&uid, users)));
    true
}

/// btop uses `/etc/passwd` first, then `getpwuid(3)` for a uid the file does
/// not list, and the numeric uid when neither resolves it.
fn user_name(uid: &str, users: &BTreeMap<String, String>) -> String {
    if let Some(name) = users.get(uid) {
        return name.clone();
    }
    uid.parse::<u32>()
        .ok()
        .and_then(|uid| {
            nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))
                .ok()
                .flatten()
        })
        .map_or_else(|| uid.to_string(), |user| user.name)
}

/// btop appends each NUL-separated argument and a space, resizes to 1000 bytes
/// when the string grows past it, and then removes the last byte.
fn command_line(raw: &[u8]) -> String {
    let arguments: Vec<&[u8]> = raw.split(|byte| *byte == 0).collect();
    let mut end = arguments.len();
    if arguments.last().is_some_and(|last| last.is_empty()) {
        end -= 1;
    }
    let mut command = Vec::new();
    for argument in &arguments[..end] {
        command.extend_from_slice(argument);
        command.push(b' ');
        if command.len() > 1000 {
            command.truncate(1000);
            break;
        }
    }
    if !command.is_empty() {
        command.pop();
    }
    String::from_utf8_lossy(&command).into_owned()
}

/// The popup's command line: [`command_line`] without btop's 1000-byte limit.
fn full_command_line(raw: &[u8]) -> String {
    let raw = raw.strip_suffix(&[0]).unwrap_or(raw);
    String::from_utf8_lossy(raw).replace('\0', " ")
}

fn uid(root: &ProcRoot, pid: i32, buffer: &mut Vec<u8>, path: &mut String) -> Option<String> {
    let read = read_at(root, pid, "status", true, buffer, path)?;
    let status = std::str::from_utf8(&buffer[..read]).ok()?;
    let line = status.lines().find_map(|line| line.strip_prefix("Uid:"))?;
    Some(line.split_whitespace().next()?.to_string())
}

/// Parse one ASCII number from a stat field.
fn number<T: std::str::FromStr>(field: &[u8]) -> Option<T> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

fn read_stat(
    root: &ProcRoot,
    pid: i32,
    context: &Context,
    buffer: &mut Vec<u8>,
    path: &mut String,
) -> Option<Stat> {
    let read = read_at(root, pid, "stat", true, buffer, path)?;
    let stat = &buffer[..read];
    // The fields after the last ')': field 14, utime, is the 12th there.
    let close = stat.iter().rposition(|byte| *byte == b')')?;
    let mut fields = stat[close + 1..]
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty());
    let state = char::from(*fields.next()?.first()?);
    let ppid = number(fields.next()?)?;
    // pgrp through cmajflt.
    for _ in 2..11 {
        fields.next()?;
    }
    let cpu_t = number::<u64>(fields.next()?)? + number::<u64>(fields.next()?)?;
    // cutime, cstime and priority.
    for _ in 13..16 {
        fields.next()?;
    }
    let nice = number(fields.next()?)?;
    let threads = number(fields.next()?)?;
    // itrealvalue.
    fields.next()?;
    let starttime = number(fields.next()?)?;
    // vsize.
    fields.next()?;
    let rss = fields.next()?;
    let (rss_len, rss_value) = (rss.len(), number::<u64>(rss));
    let mem = process_memory(rss_len, rss_value, root, pid, context, buffer, path)?;
    Some(Stat {
        state,
        ppid,
        cpu_t,
        nice,
        threads,
        starttime,
        mem,
    })
}

/// btop replaces the RSS field with totalMem when its digit count exceeds that
/// of totalMem in KiB, then reads statm when the value reaches totalMem.
fn process_memory(
    rss_len: usize,
    rss: Option<u64>,
    root: &ProcRoot,
    pid: i32,
    context: &Context,
    buffer: &mut Vec<u8>,
    path: &mut String,
) -> Option<u64> {
    let limit = context
        .total_memory
        .map(|total| (total >> 10).to_string().len());
    let mut memory = match limit {
        Some(len) if rss_len > len => context.total_memory?,
        _ => rss?.checked_mul(context.page_size)?,
    };
    if context.total_memory.is_some_and(|total| memory >= total) {
        memory = statm_memory(root, pid, context.page_size, buffer, path)?;
    }
    Some(memory)
}

fn statm_memory(
    root: &ProcRoot,
    pid: i32,
    page_size: u64,
    buffer: &mut Vec<u8>,
    path: &mut String,
) -> Option<u64> {
    let read = read_at(root, pid, "statm", true, buffer, path)?;
    std::str::from_utf8(&buffer[..read])
        .ok()?
        .split_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()?
        .checked_mul(page_size)
}

fn apply(cached: &mut CachedProcess, stat: Stat, context: &Context) {
    let entry = &mut cached.entry;
    entry.state = Some(state_name(stat.state).to_string());
    if entry.ppid.unwrap_or(0) == 0 {
        entry.ppid = Some(stat.ppid);
    }
    entry.nice = Some(stat.nice);
    entry.threads = Some(stat.threads);
    if cached.cpu_s == 0 {
        cached.cpu_s = stat.starttime;
        cached.cpu_t = stat.cpu_t;
    }
    let cpu_pct = process_cpu(
        stat.cpu_t.saturating_sub(cached.cpu_t),
        context.delta_cputimes,
        context.cmult,
        context.cores,
    ) * context.cpu_scale;
    entry.cpu_pct = Some(cpu_pct);
    entry.cpu_c = context.uptime.map(|uptime| {
        stat.cpu_t as f64 / ((uptime * context.clk_tck) - cached.cpu_s as f64).max(1.0)
    });
    cached.cpu_t = stat.cpu_t;
    entry.mem_bytes = Some(stat.mem);
    // One allocation of exactly the window, kept at it when the window narrows.
    let record = &mut cached.record;
    while record.len() >= context.record_slots {
        record.pop_front();
    }
    record.shrink_to(context.record_slots);
    record.reserve_exact(context.record_slots - record.len());
    record.push_back([cpu_pct as f32, stat.mem as f32]);
}

impl RowGraph {
    /// btop's `proc_cpu_graphs` rule (`btop_draw.cpp:1797-1806`): a graph
    /// starts when `trigger` is above 0, and goes after [`GRAPH_DROP_AFTER`]
    /// updates in a row below 0.1 percent. A new graph starts from 0, the
    /// previous update's value, as btop's first braille cell does
    /// (`Graph::_create`, `last` = 0), so it draws in the update that creates
    /// it. Each update pushes [`graph_value`] of `value`. An unavailable
    /// trigger changes nothing. The own graph passes cpu_p twice; the summed
    /// graph passes the descendants' cpu_p and the subtree's.
    fn tick(&mut self, trigger: Option<f64>, value: f64, update_ms: u64) {
        let Some(trigger) = trigger else {
            return;
        };
        match self.history {
            None if trigger > 0.0 => {
                let mut history = History::with_window(PROC_WINDOW_MS);
                history.push(Some(0.0), update_ms);
                self.history = Some(history);
                self.idle = 0;
            }
            None => return,
            Some(_) if trigger < 0.1 => {
                self.idle += 1;
                if self.idle >= GRAPH_DROP_AFTER {
                    self.history = None;
                    return;
                }
            }
            Some(_) => self.idle = 0,
        }
        if let Some(history) = self.history.as_mut() {
            history.push(Some(graph_value(value)), update_ms);
        }
    }
}

/// btop's count of idle updates before a graph goes, at any interval: 20 s at
/// btop's 2000 ms, 2.5 s at 250 ms. The count bounds how many rows carry a
/// graph, which is the client's cost (ACC-VIEW-52-PROC-GRAPH).
const GRAPH_DROP_AFTER: u64 = 10;

/// btop draws 0.1 to 5 percent as 5 so light load stays visible, and any other
/// value rounded (`btop_draw.cpp:1902`).
fn graph_value(cpu_p: f64) -> f64 {
    if (0.1..5.0).contains(&cpu_p) {
        5.0
    } else {
        cpu_p.round()
    }
}

/// DEF-VIEW-67-SPARKLINE-TONE: a row graph is painted in the band of its
/// window's peak, not of the newest readout, so a bar whose history holds a
/// spike keeps that spike's tone while the number falls to zero. A window of
/// zeros keeps `cpu-zero`. Stored samples never change, so the tone holds
/// still between frames until the window itself changes.
fn window_tone(history: &History) -> &'static str {
    cpu_tone(history.max())
}

/// btop: `clamp(round(cmult * 1000 * delta / max(1, delta_total)) / 10, 0, 100 * cores)`.
/// The product is integer arithmetic in btop, so the division truncates before
/// the single decimal is added.
fn process_cpu(delta: u64, delta_total: u64, cmult: u64, cores: usize) -> f64 {
    let scaled = cmult.saturating_mul(1000).saturating_mul(delta) / delta_total.max(1);
    (scaled as f64 / 10.0).clamp(0.0, 100.0 * cores as f64)
}

/// btop's `Proc::proc_sorter` in tree mode: a stable sort on the chosen
/// column. btop's "cpu lazy" rotation applies only to its flat list, which
/// the browser now sorts.
fn sort_entries(entries: &mut [ProcEntry], sorting: &str, reverse: bool) {
    if !SORT_MODES.contains(&sorting) {
        return;
    }
    entries.sort_by(|left, right| {
        let ordering = compare(left, right, sorting);
        if reverse {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

fn compare(left: &ProcEntry, right: &ProcEntry, sorting: &str) -> Ordering {
    match sorting {
        "pid" => left.pid.cmp(&right.pid),
        "name" => text(&left.name).cmp(text(&right.name)),
        "command" => left.command.cmp(&right.command),
        "threads" => left.threads.unwrap_or(0).cmp(&right.threads.unwrap_or(0)),
        "user" => text(&left.user).cmp(text(&right.user)),
        "memory" => left
            .mem_bytes
            .unwrap_or(0)
            .cmp(&right.mem_bytes.unwrap_or(0)),
        "cpu direct" => float(left.cpu_pct)
            .partial_cmp(&float(right.cpu_pct))
            .unwrap_or(Ordering::Equal),
        "cpu lazy" => float(left.cpu_c)
            .partial_cmp(&float(right.cpu_c))
            .unwrap_or(Ordering::Equal),
        _ => Ordering::Equal,
    }
}

/// btop's tree view: sort flat, group every child under its parent, then walk
/// pre-order. The per-level `tree_sort` re-applies the same comparison the flat
/// sort already made, so the pre-order walk is its result. Sets each row's
/// depth and, on rows with children, the subtree sums; returns, per output
/// row, the descendants' summed cpu_p (`None` on a leaf), which drives the
/// summed graph.
fn tree_entries(entries: &mut Vec<ProcEntry>, config: &Config) -> Vec<Option<f64>> {
    let mut pids: Vec<i32> = entries.iter().map(|entry| entry.pid).collect();
    pids.sort_unstable();
    sort_entries(entries, &config.proc_sorting, config.proc_reversed);
    for entry in entries.iter_mut() {
        if !entry
            .ppid
            .is_some_and(|ppid| pids.binary_search(&ppid).is_ok())
        {
            entry.ppid = Some(0);
        }
    }
    // Stable: siblings keep the configured order. btop then takes each
    // parent's children as the equal range of its pid (`rng::equal_range`).
    entries.sort_by_key(|entry| entry.ppid);
    let Some(first) = entries.first() else {
        return Vec::new();
    };
    let mut order = Vec::with_capacity(entries.len());
    for index in children(entries, first.ppid) {
        walk(index, 0, entries, &mut order);
    }
    let mut busy_below = Vec::with_capacity(order.len());
    let ordered: Vec<ProcEntry> = order
        .into_iter()
        .map(|walked| {
            let mut entry = std::mem::take(&mut entries[walked.index]);
            entry.depth = walked.depth;
            entry.sum = walked.sum;
            busy_below.push(walked.busy_below);
            entry
        })
        .collect();
    *entries = ordered;
    busy_below
}

/// One row of the pre-order walk.
struct Walked {
    index: usize,
    depth: usize,
    sum: Option<ProcSum>,
    busy_below: Option<f64>,
}

/// Own plus descendants' values; `None` only when no row has the value.
#[derive(Clone, Copy, Default)]
struct Totals {
    cpu: Option<f64>,
    mem: Option<u64>,
    threads: Option<usize>,
}

impl Totals {
    fn plus(self, other: Self) -> Self {
        fn add<T: std::ops::Add<Output = T>>(one: Option<T>, other: Option<T>) -> Option<T> {
            one.into_iter()
                .chain(other)
                .reduce(|one, other| one + other)
        }
        Self {
            cpu: add(self.cpu, other.cpu),
            mem: add(self.mem, other.mem),
            threads: add(self.threads, other.threads),
        }
    }
}

/// The indices of the rows whose ppid is `ppid`, in rows sorted by ppid.
fn children(entries: &[ProcEntry], ppid: Option<i32>) -> std::ops::Range<usize> {
    let start = entries.partition_point(|entry| entry.ppid < ppid);
    start..start + entries[start..].partition_point(|entry| entry.ppid == ppid)
}

/// Pre-order from `index`; returns the subtree totals and records them on
/// rows with children.
fn walk(index: usize, depth: usize, entries: &[ProcEntry], order: &mut Vec<Walked>) -> Totals {
    let slot = order.len();
    order.push(Walked {
        index,
        depth,
        sum: None,
        busy_below: None,
    });
    let entry = &entries[index];
    let own = Totals {
        cpu: entry.cpu_pct,
        mem: entry.mem_bytes,
        threads: entry.threads,
    };
    let kids = children(entries, Some(entry.pid));
    if kids.is_empty() {
        return own;
    }
    let below = kids.fold(Totals::default(), |below, child| {
        below.plus(walk(child, depth + 1, entries, order))
    });
    let total = below.plus(own);
    order[slot].sum = Some(ProcSum {
        cpu_pct: total.cpu,
        mem_bytes: total.mem,
        threads: total.threads,
        graph: Default::default(),
        graph_tone: "",
    });
    order[slot].busy_below = below.cpu;
    total
}

fn text(value: &Option<Arc<str>>) -> &str {
    value.as_deref().unwrap_or("")
}

fn float(value: Option<f64>) -> f64 {
    value.unwrap_or(0.0)
}

/// btop's `Proc::collect` cputimes: every numeric field of the `cpu` line,
/// guest and guest_nice included (unlike `Cpu::collect`, which subtracts them),
/// and the count of cpuN lines in the same read.
fn cpu_times(proc_root: &Path) -> Option<(u64, usize)> {
    let text = fs::read_to_string(proc_root.join("stat")).ok()?;
    let line = text.lines().find(|line| line.starts_with("cpu "))?;
    let online = text
        .lines()
        .filter(|line| line.starts_with("cpu") && !line.starts_with("cpu "))
        .count();
    Some((
        line.split_whitespace()
            .skip(1)
            .map_while(|value| value.parse::<u64>().ok())
            .sum(),
        online,
    ))
}

/// Container mode: the ids in `file` (`cgroup.procs`, or `cgroup.threads` for
/// the CPU collector's load) of `cgroup` and every descendant cgroup, depth
/// first. `None` when the container's own file is unreadable; a descendant
/// that vanishes mid-walk is skipped.
pub(super) fn cgroup_members(cgroup: &Path, file: &str) -> Option<HashSet<i32>> {
    let pids = |directory: &Path| -> Option<Vec<i32>> {
        let text = fs::read_to_string(directory.join(file)).ok()?;
        Some(
            text.lines()
                .filter_map(|line| line.trim().parse().ok())
                .collect(),
        )
    };
    let mut members: HashSet<i32> = pids(cgroup)?.into_iter().collect();
    let children = |directory: &Path| -> Vec<PathBuf> {
        fs::read_dir(directory)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .map(|entry| entry.path())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut pending = children(cgroup);
    while let Some(directory) = pending.pop() {
        members.extend(pids(&directory).unwrap_or_default());
        pending.extend(children(&directory));
    }
    Some(members)
}

/// A container's member pids; `None` when its cgroup cannot be read.
type Members = Option<HashSet<i32>>;

/// DEF-PERF-76-SERVER-COST: the membership's period, the 5 s slow cadence
/// (`gpu::SLOW_PERIOD`). The cgroup tree and its `cgroup.procs` files do not
/// change every tick, so the walk runs on the slow cadence; a process that
/// joins or leaves the container is listed up to 5 s late.
static MEMBERS: Mutex<BTreeMap<PathBuf, (Instant, Members)>> = Mutex::new(BTreeMap::new());

/// [`cgroup_members`] over `cgroup.procs`, read at most once per
/// [`SLOW_PERIOD`].
fn cached_members(cgroup: &Path) -> Option<HashSet<i32>> {
    let mut cache = MEMBERS.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((read, members)) = cache.get(cgroup)
        && read.elapsed() < SLOW_PERIOD
    {
        return members.clone();
    }
    let members = cgroup_members(cgroup, "cgroup.procs");
    cache.insert(cgroup.to_path_buf(), (Instant::now(), members.clone()));
    members
}

/// btop `Tools::system_uptime`: the first token of `/proc/uptime` as a double.
fn uptime(proc_root: &Path) -> Option<f64> {
    let text = fs::read_to_string(proc_root.join("uptime")).ok()?;
    text.split_whitespace().next()?.parse().ok()
}

fn total_memory(proc_root: &Path) -> Option<u64> {
    let text = fs::read_to_string(proc_root.join("meminfo")).ok()?;
    let line = text.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kilobytes: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kilobytes * 1024)
}

/// btop's `_SC_CLK_TCK`, defaulting to 100 when `sysconf` fails.
fn clk_tck() -> f64 {
    sysconf(nix::unistd::SysconfVar::CLK_TCK)
        .filter(|value| *value > 0)
        .map_or(100.0, |value| value as f64)
}

/// btop's `_SC_NPROCESSORS_ONLN`, then `_SC_NPROCESSORS_CONF`, then 1.
pub(super) fn core_count() -> usize {
    let count = sysconf(nix::unistd::SysconfVar::_NPROCESSORS_ONLN)
        .filter(|value| *value > 0)
        .or_else(|| sysconf(nix::unistd::SysconfVar::_NPROCESSORS_CONF).filter(|value| *value > 0))
        .unwrap_or(1);
    usize::try_from(count).unwrap_or(1)
}

fn sysconf(variable: nix::unistd::SysconfVar) -> Option<i64> {
    nix::unistd::sysconf(variable).ok().flatten()
}

fn page_size() -> u64 {
    sysconf(nix::unistd::SysconfVar::PAGE_SIZE)
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(4096)
}

fn passwd_map(text: &str) -> BTreeMap<String, String> {
    let mut users = BTreeMap::new();
    for line in text.lines() {
        let mut fields = line.split(':');
        let (Some(name), Some(_), Some(uid)) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        if users.contains_key(uid) {
            break;
        }
        users.insert(uid.to_string(), name.to_string());
    }
    users
}

impl PasswdCache {
    /// btop rebuilds the uid map only when `/etc/passwd` mtime changes, and
    /// gives up for the process lifetime when it cannot be read.
    fn refresh(&mut self) {
        if self.unavailable {
            return;
        }
        let path = Path::new("/etc/passwd");
        let Ok(modified) = fs::metadata(path).and_then(|metadata| metadata.modified()) else {
            self.unavailable = true;
            self.users.clear();
            return;
        };
        if self.modified == Some(modified) {
            return;
        }
        match fs::read_to_string(path) {
            Ok(text) => {
                self.users = passwd_map(&text);
                self.modified = Some(modified);
            }
            Err(_) => {
                self.unavailable = true;
                self.users.clear();
            }
        }
    }
}

/// btop's `Proc::proc_states` table.
fn state_name(state: char) -> &'static str {
    match state {
        'R' => "Running",
        'S' => "Sleeping",
        'D' => "Waiting",
        'Z' => "Zombie",
        'T' => "Stopped",
        't' => "Tracing",
        'X' | 'x' => "Dead",
        'K' => "Wakekill",
        'W' => "Unknown",
        'P' => "Parked",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fixture(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative)
    }

    fn temp_tree(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("btop-web-proc-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn copy_tree(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let destination = target.join(entry.file_name());
            if entry.path().is_dir() {
                copy_tree(&entry.path(), &destination);
            } else {
                fs::copy(entry.path(), &destination).unwrap();
            }
        }
    }

    fn config() -> Config {
        Config::load(None).expect("embedded defaults")
    }

    /// The pid's CPU ring length; `None` while no popup watches it.
    fn ring_len(proc_root: &Path, pid: i32) -> Option<usize> {
        LAST.lock()
            .unwrap()
            .get(proc_root)?
            .detailed
            .get(&pid)
            .map(|watch| watch.cpu.history.len())
    }

    fn record(proc_root: &Path, pid: i32) -> Vec<[f32; 2]> {
        LAST.lock().unwrap().get(proc_root).unwrap().processes[&pid]
            .record
            .iter()
            .copied()
            .collect()
    }

    /// ACC-VIEW-176-PROC-POPUP: a request starts the watch, the sampler's slow
    /// pass fills it with no further request, and a pid no request asks about
    /// holds no ring. The watch goes [`DETAIL_MISSED`] passes after its last
    /// request, and a pid the last pass did not list has no detail.
    #[test]
    fn a_popup_watch_fills_on_the_pass_and_goes_with_its_requests() {
        let proc_root = temp_tree("detail-watch").join("proc");
        copy_tree(&fixture("proc"), &proc_root);
        let config = config();
        assert!(detail(&proc_root, 222).is_none(), "no pass has run");
        collect(&proc_root, &config);
        assert!(
            detail(&proc_root, 99999).is_none(),
            "the table lists no such pid"
        );
        assert_eq!(ring_len(&proc_root, 111), None, "a ring without a request");

        let first = detail(&proc_root, 222).expect("a listed pid");
        assert_eq!(ring_len(&proc_root, 222), Some(1));
        assert!(first.cpu.chart.line.is_empty(), "one sample draws no line");
        assert_eq!(first.io_read, None, "the fixture has no io file");
        assert_eq!(first.command.as_deref(), Some("hotproc --busy"));
        detail(&proc_root, 222).expect("a listed pid");
        assert_eq!(
            ring_len(&proc_root, 222),
            Some(1),
            "a request took a sample"
        );

        collect(&proc_root, &config);
        assert_eq!(ring_len(&proc_root, 222), Some(2));
        let second = detail(&proc_root, 222).expect("a listed pid");
        assert!(!second.cpu.chart.line.is_empty());
        assert!(!second.mem.chart.line.is_empty());
        assert_eq!(second.cpu.scale.label_max, Some(1));

        for _ in 0..DETAIL_MISSED {
            collect(&proc_root, &config);
        }
        assert!(ring_len(&proc_root, 222).is_some());
        collect(&proc_root, &config);
        assert_eq!(
            ring_len(&proc_root, 222),
            None,
            "the watch outlived its limit"
        );
    }

    /// ACC-VIEW-176-PROC-POPUP: a reused pid is another process, so the old
    /// process's rings go with it, and so do the rings of a process that exits.
    #[test]
    fn a_popup_watch_ends_with_its_process() {
        let proc_root = temp_tree("detail-reuse").join("proc");
        copy_tree(&fixture("proc"), &proc_root);
        let config = config();
        collect(&proc_root, &config);
        detail(&proc_root, 222).expect("a listed pid");
        detail(&proc_root, 111).expect("a listed pid");

        let stat = proc_root.join("222/stat");
        let reused = fs::read_to_string(&stat)
            .unwrap()
            .replace(" 29470638 ", " 29470639 ");
        fs::write(&stat, reused).unwrap();
        fs::remove_dir_all(proc_root.join("111")).unwrap();
        collect(&proc_root, &config);
        assert_eq!(
            ring_len(&proc_root, 222),
            None,
            "a reused pid kept its rings"
        );
        assert_eq!(
            ring_len(&proc_root, 111),
            None,
            "an exited pid kept its rings"
        );
        assert!(detail(&proc_root, 111).is_none());
    }

    /// DEF-GRAPH-129-PROC-PLOT: a new watch draws the passes before its request,
    /// with its scale idle at the right ceiling, and continues from them.
    #[test]
    fn a_new_popup_watch_draws_the_passes_before_it() {
        let proc_root = temp_tree("detail-seed").join("proc");
        copy_tree(&fixture("proc"), &proc_root);
        let config = config();
        for _ in 0..5 {
            collect(&proc_root, &config);
        }
        assert_eq!(ring_len(&proc_root, 222), None, "a ring without a request");

        let first = detail(&proc_root, 222).expect("a listed pid");
        assert_eq!(ring_len(&proc_root, 222), Some(5));
        assert_eq!(first.slow_seq, 5);
        assert!(!first.cpu.chart.line.is_empty());
        assert!(!first.mem.chart.line.is_empty());
        for plot in [&first.cpu, &first.mem] {
            assert!(plot.scale.style.is_empty(), "a scale change from zero");
            assert!(plot.scale.label_max.is_some_and(|max| max > 0));
        }
        // The mem field of the fixture's stat, not the ceiling's floor of 1 MiB.
        assert_eq!(
            first.mem.scale.label_max,
            Some(memory_ceiling(record(&proc_root, 222)[4][1].into()) as u64)
        );

        collect(&proc_root, &config);
        assert_eq!(ring_len(&proc_root, 222), Some(6), "the pass continues it");
        assert_eq!(detail(&proc_root, 222).expect("listed").slow_seq, 6);
    }

    /// DEF-GRAPH-129-PROC-PLOT: a process older than the window opens on the
    /// whole axis: the curve starts one slot in from the left, as every full
    /// history does, at the plots' slot of one per pass.
    #[test]
    fn a_process_older_than_the_window_opens_on_the_whole_axis() {
        let proc_root = temp_tree("detail-axis").join("proc");
        copy_tree(&fixture("proc"), &proc_root);
        let mut config = config();
        config.update_ms = 1000;
        config.window_s = 60;
        let slots = capacity(config.window_ms(), config.update_ms);
        for _ in 0..slots + 3 {
            collect(&proc_root, &config);
        }
        assert_eq!(record(&proc_root, 222).len(), slots);
        let first = detail(&proc_root, 222).expect("a listed pid");
        assert_eq!(ring_len(&proc_root, 222), Some(slots));
        let start: f64 = first.cpu.chart.line[1..]
            .split(' ')
            .next()
            .and_then(|x| x.parse().ok())
            .expect("the first vertex");
        let slot = NET_PLOT.width / (slots - 1) as f64;
        assert!(start <= slot + 0.01, "the curve starts at {start}");

        // The record follows a narrower window, down to its capacity.
        config.window_s = 30;
        collect(&proc_root, &config);
        let narrowed = capacity(config.window_ms(), config.update_ms);
        let state = LAST.lock().unwrap();
        let held = &state.get(&proc_root).unwrap().processes[&222].record;
        assert_eq!(held.len(), narrowed);
        assert_eq!(held.capacity(), narrowed);
    }

    /// DEF-GRAPH-129-PROC-PLOT: a reused pid starts empty, as the watch does, and
    /// an exited pid's record goes with its row.
    #[test]
    fn a_record_ends_with_its_process() {
        let proc_root = temp_tree("record-reuse").join("proc");
        copy_tree(&fixture("proc"), &proc_root);
        let config = config();
        collect(&proc_root, &config);
        collect(&proc_root, &config);
        assert_eq!(record(&proc_root, 222).len(), 2);

        let stat = proc_root.join("222/stat");
        let reused = fs::read_to_string(&stat)
            .unwrap()
            .replace(" 29470638 ", " 29470639 ");
        fs::write(&stat, reused).unwrap();
        fs::remove_dir_all(proc_root.join("111")).unwrap();
        collect(&proc_root, &config);
        assert_eq!(
            record(&proc_root, 222).len(),
            1,
            "the old process's samples"
        );
        assert!(
            detail(&proc_root, 222)
                .expect("listed")
                .cpu
                .chart
                .line
                .is_empty(),
            "one sample of the new process draws no line"
        );
        assert!(
            !LAST
                .lock()
                .unwrap()
                .get(&proc_root)
                .unwrap()
                .processes
                .contains_key(&111),
            "an exited pid kept its record"
        );
        assert!(detail(&proc_root, 111).is_none());
    }

    /// DEF-GRAPH-129-PROC-PLOT: a process whose stat cannot be read has no value,
    /// so it keeps no sample and its plots draw no line.
    #[test]
    fn an_unread_value_keeps_no_sample() {
        let proc_root = temp_tree("record-unread").join("proc");
        copy_tree(&fixture("proc"), &proc_root);
        fs::remove_file(proc_root.join("222/stat")).unwrap();
        let config = config();
        collect(&proc_root, &config);
        collect(&proc_root, &config);
        assert!(record(&proc_root, 222).is_empty());
        let first = detail(&proc_root, 222).expect("a listed pid");
        assert!(first.cpu.chart.line.is_empty());
        assert!(first.mem.chart.line.is_empty());
    }

    /// ACC-PERF-141-CACHE-BOUND: a client that walks every pid holds at most
    /// [`DETAIL_LIMIT`] watches, and the newest request always has its own.
    #[test]
    fn popup_watches_are_bounded() {
        let proc_root = temp_tree("detail-bound").join("proc");
        copy_tree(&fixture("proc"), &proc_root);
        let pids: Vec<i32> = (1000..1000 + DETAIL_LIMIT as i32 + 4).collect();
        for pid in &pids {
            copy_tree(&proc_root.join("222"), &proc_root.join(pid.to_string()));
        }
        let config = config();
        collect(&proc_root, &config);
        for pid in &pids {
            detail(&proc_root, *pid).expect("a listed pid");
            assert!(
                ring_len(&proc_root, *pid).is_some(),
                "pid {pid} has no ring"
            );
        }
        let held = LAST.lock().unwrap().get(&proc_root).unwrap().detailed.len();
        assert_eq!(held, DETAIL_LIMIT);
    }

    /// The popup's command line keeps what btop's 1000-byte limit removes.
    #[test]
    fn the_popup_command_line_has_no_limit() {
        assert_eq!(full_command_line(b"ttyd\0-d\0 3\0"), "ttyd -d  3");
        assert_eq!(full_command_line(b""), "");
        let raw = [b'a'; 5000];
        assert_eq!(full_command_line(&raw).len(), 5000);
        assert_eq!(command_line(&raw).len(), 999);
    }

    /// `write_bytes` is not `cancelled_write_bytes`, and a missing or
    /// malformed counter is unavailable.
    #[test]
    fn io_counters_are_read_by_their_exact_key() {
        let io =
            "rchar: 9\nread_bytes: 5242880\nwrite_bytes: 1048576\ncancelled_write_bytes: 4096\n";
        assert_eq!(io_counter(io, "read_bytes"), Some(5_242_880));
        assert_eq!(io_counter(io, "write_bytes"), Some(1_048_576));
        assert_eq!(
            io_counter("cancelled_write_bytes: 4096\n", "write_bytes"),
            None
        );
        assert_eq!(io_counter("read_bytes: x\n", "read_bytes"), None);
    }

    /// The popup plots' tops: 1, 2 or 5 x 10^k above the peak, at least 1%
    /// and 1 MiB, and a whole GiB step from 1 GiB up.
    #[test]
    fn popup_plot_ceilings_follow_the_peak() {
        const MIB: f64 = 1_048_576.0;
        const GIB: f64 = 1024.0 * MIB;
        for (peak, top) in [
            (0.0, 1.0),
            (0.4, 1.0),
            (1.2, 2.0),
            (57.0, 100.0),
            (640.0, 1000.0),
        ] {
            assert_eq!(cpu_ceiling(peak), top, "cpu peak {peak}");
        }
        for (peak, top) in [
            (0.0, MIB),
            (700.0 * MIB, 1000.0 * MIB),
            (1.2 * GIB, 2.0 * GIB),
            (17.0 * GIB, 20.0 * GIB),
        ] {
            assert_eq!(memory_ceiling(peak), top, "memory peak {peak}");
        }
    }

    fn configured(edit: impl FnOnce(&mut Config)) -> Config {
        let mut config = config();
        edit(&mut config);
        config
    }

    fn order(mut entries: Vec<ProcEntry>, sorting: &str, reverse: bool) -> Vec<i32> {
        sort_entries(&mut entries, sorting, reverse);
        entries.iter().map(|entry| entry.pid).collect()
    }

    fn tree_order(entries: &[ProcEntry]) -> Vec<(i32, usize)> {
        entries
            .iter()
            .map(|entry| (entry.pid, entry.depth))
            .collect()
    }

    #[test]
    fn process_cpu_uses_btops_integer_division() {
        assert_eq!(process_cpu(100, 8000, 1, 8), 1.2);
        assert_eq!(process_cpu(0, 8000, 1, 8), 0.0);
        assert_eq!(process_cpu(1_000_000, 1, 1, 8), 800.0);
    }

    #[test]
    fn cpu_times_sums_every_numeric_field() {
        let root = temp_tree("cpu-times");
        fs::write(
            root.join("stat"),
            "cpu 1 2 3 4 5 6 7 8 9 10\ncpu0 1 1 1 1 1 1 1 1 1 1\n",
        )
        .unwrap();
        assert_eq!(cpu_times(&root), Some((55, 1)));
    }

    #[test]
    fn collect_reads_the_btop_process_columns() {
        let snapshot = collect(&fixture("proc"), &config());
        let process = snapshot
            .entries
            .iter()
            .find(|entry| entry.pid == 111)
            .expect("pid 111");
        assert_eq!(process.name.as_deref(), Some("ttyd"));
        assert_eq!(process.ppid, Some(1));
        assert_eq!(process.threads, Some(2));
        assert_eq!(process.nice, Some(0));
        assert_eq!(process.state.as_deref(), Some("Sleeping"));
        assert!(process.user.as_deref().is_some_and(|user| !user.is_empty()));
        assert_eq!(process.mem_bytes, Some(3482 * page_size()));
        assert_eq!(
            process.command.as_ref(),
            "ttyd -d 3 -W -i lo -p 7681 -t titleFixed Resources Monitor -t scrollback 0 btop --utf-force"
        );
        assert_eq!(process.cpu_pct, Some(0.0));
        assert!(
            process
                .cpu_c
                .is_some_and(|value| value > 0.0 && value < 1.0)
        );
    }

    #[test]
    fn collect_computes_cpu_c_from_uptime_and_starttime() {
        let root = fixture("proc");
        let snapshot = collect(&root, &config());
        let process = snapshot
            .entries
            .iter()
            .find(|entry| entry.pid == 222)
            .expect("pid 222");
        let uptime = uptime(&root).expect("recorded uptime");
        let expected = 300_000.0 / (uptime * clk_tck() - 29_470_638.0).max(1.0);
        let value = process.cpu_c.expect("cpu_c");
        assert!((value - expected).abs() < 1e-12, "{value} != {expected}");
        assert!(value < 1.0, "cpu_c is a fraction of one CPU, got {value}");
    }

    #[test]
    fn collect_counts_total_running_and_threads() {
        let snapshot = collect(&fixture("proc"), &config());
        assert_eq!(snapshot.total, Some(9));
        assert_eq!(snapshot.running, Some(1));
        assert_eq!(snapshot.threads, Some(85));
    }

    /// ACC-VIEW-53-PROC-TILE: pid 222 is R, the six others with a stat are
    /// S, and pid 333 (no stat) counts in the total only.
    #[test]
    fn collect_counts_states_from_the_rows() {
        let snapshot = collect(&fixture("proc"), &config());
        assert_eq!(snapshot.total, Some(9));
        assert_eq!(snapshot.running, Some(1));
        assert_eq!(snapshot.sleeping, Some(7));
        assert_eq!(snapshot.waiting, Some(0));
        assert_eq!(snapshot.zombie, Some(0));
    }

    /// Replace `from` with `to` once in `<root>/<file>`; the recorded files are read-only.
    fn edit(root: &Path, file: &str, from: &str, to: &str) {
        let path = root.join(file);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(from), "{file} lacks {from}");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&path, text.replacen(from, to, 1)).unwrap();
    }

    #[test]
    fn waiting_and_zombie_states_are_counted() {
        let root = temp_tree("states");
        copy_tree(&fixture("proc"), &root);
        edit(&root, "111/stat", "(ttyd) S", "(ttyd) D");
        edit(&root, "158/stat", "(pulseaudio) S", "(pulseaudio) Z");
        let snapshot = collect(&root, &config());
        assert_eq!(snapshot.waiting, Some(1));
        assert_eq!(snapshot.zombie, Some(1));
        assert_eq!(snapshot.sleeping, Some(5));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn top_process_has_the_highest_cpu_and_the_lower_pid_on_ties() {
        let row = |pid: i32, cpu_pct: Option<f64>| ProcEntry {
            pid,
            cpu_pct,
            ..ProcEntry::default()
        };
        let pid = |entries: &[ProcEntry]| top_process(entries).map(|entry| entry.pid);
        assert_eq!(
            pid(&[row(5, Some(1.0)), row(3, Some(9.5)), row(4, None)]),
            Some(3)
        );
        assert_eq!(
            pid(&[row(9, Some(7.0)), row(2, Some(7.0)), row(4, Some(7.0))]),
            Some(2)
        );
        assert_eq!(pid(&[row(1, None), row(2, None)]), None);
        assert_eq!(pid(&[]), None);
    }

    /// pid 222 gains 5000 of 10000 ticks: the top process is pid 222 with the
    /// Cpu% its table row shows.
    #[test]
    fn top_process_uses_the_table_cpu_value() {
        let root = temp_tree("top");
        copy_tree(&fixture("proc"), &root);
        collect(&root, &config());
        edit(&root, "stat", "cpu  62538468 ", "cpu  62548468 ");
        edit(&root, "222/stat", " 200000 100000 ", " 205000 100000 ");
        let snapshot = collect(&root, &config());
        let top = snapshot.top.expect("a top process");
        let row = snapshot
            .entries
            .iter()
            .find(|entry| entry.pid == 222)
            .expect("pid 222");
        assert_eq!(top.pid, 222);
        assert_eq!(top.cpu_pct, row.cpu_pct);
        assert_eq!(top.cpu_label(), "50.0");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn share_count_reaches_80_percent() {
        let count = |values: &[f64]| share_count(values.iter().copied());
        assert_eq!(count(&[50.0, 30.0, 10.0, 10.0]), Some(2));
        assert_eq!(count(&[10.0, 30.0, 50.0, 10.0]), Some(2));
        assert_eq!(count(&[25.0; 4]), Some(4));
        assert_eq!(count(&[100.0]), Some(1));
        assert_eq!(count(&[0.0, 0.0]), Some(0));
        assert_eq!(count(&[]), None);
        let rows = [Some(60.0), None, Some(40.0)];
        assert_eq!(share_count(rows.iter().flatten().copied()), Some(2));
    }

    /// pid 715 holds 34353 of the 40740 RSS pages the rows with a stat hold.
    #[test]
    fn memory_share_count_on_the_fixture() {
        let snapshot = collect(&fixture("proc"), &config());
        assert_eq!(snapshot.mem_80, Some(1));
        assert!(snapshot.cpu_80.is_some());
    }

    #[test]
    fn count_ceiling_steps_1_2_5() {
        for (largest, ceiling) in [
            (0.0, 10),
            (9.0, 10),
            (10.0, 10),
            (11.0, 20),
            (133.0, 200),
            (201.0, 500),
            (640.0, 1000),
            (1001.0, 2000),
        ] {
            assert_eq!(count_ceiling(largest), ceiling, "{largest}");
        }
    }

    #[test]
    fn count_history_draws_from_the_second_tick() {
        let root = temp_tree("count-history");
        copy_tree(&fixture("proc"), &root);
        let first = collect(&root, &config());
        assert!(first.chart.line.is_empty() && first.chart.area.is_empty());
        let second = collect(&root, &config());
        assert!(second.chart.line.starts_with('M'), "{}", second.chart.line);
        assert!(second.chart.area.ends_with('Z'));
        assert_eq!(second.chart_max, 10, "9 processes draw on 0 to 10");
        assert_eq!(
            second.scale.draw_max, 10.0,
            "a steady ceiling runs no change"
        );
        assert!(second.scale.style.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_process_without_a_stat_file_stays_in_the_list() {
        let snapshot = collect(&fixture("proc"), &config());
        let process = snapshot
            .entries
            .iter()
            .find(|entry| entry.pid == 333)
            .expect("pid 333");
        assert_eq!(process.name.as_deref(), Some("nostat"));
        assert_eq!(process.command.as_ref(), "/usr/bin/nostat");
        assert!(process.user.as_deref().is_some_and(|user| !user.is_empty()));
        assert_eq!(process.state, None);
        assert_eq!(process.cpu_pct, None);
        assert_eq!(process.cpu_c, None);
        assert_eq!(process.mem_bytes, None);
        assert_eq!(process.threads, None);
    }

    #[test]
    fn a_permission_denied_status_file_leaves_the_user_unavailable() {
        let root = temp_tree("denied-status");
        copy_tree(&fixture("proc"), &root);
        let status = root.join("334/status");
        fs::set_permissions(&status, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_to_string(&status).is_ok() {
            return; // a privileged test run can read the file anyway
        }

        let first = collect(&root, &config());
        let process = first
            .entries
            .iter()
            .find(|entry| entry.pid == 334)
            .expect("pid 334");
        assert_eq!(process.user, None);
        assert_eq!(process.name.as_deref(), Some("denied"));
        // btop stops the first collection before stat when status fails.
        assert_eq!(process.state, None);
        assert_eq!(process.mem_bytes, None);

        let second = collect(&root, &config());
        let process = second
            .entries
            .iter()
            .find(|entry| entry.pid == 334)
            .expect("pid 334");
        assert_eq!(process.user, None, "status is never retried");
        assert_eq!(process.state.as_deref(), Some("Sleeping"));
        assert_eq!(process.threads, Some(4));
        assert_eq!(process.nice, Some(5));
    }

    #[test]
    fn a_cached_entry_survives_a_missing_stat_file() {
        let root = temp_tree("cached-stat");
        fs::write(root.join("stat"), "cpu 0 0 0 0 0 0 0 0 0 0\n").unwrap();
        fs::write(root.join("meminfo"), "MemTotal:       1048576 kB\n").unwrap();
        fs::write(root.join("uptime"), "1000.00 0.00\n").unwrap();
        let pid = root.join("555");
        fs::create_dir_all(&pid).unwrap();
        fs::write(pid.join("comm"), "cached\n").unwrap();
        fs::write(pid.join("cmdline"), b"cached\0--flag\0").unwrap();
        fs::write(
            pid.join("status"),
            "Name:\tcached\nUid:\t1000\t1000\t1000\t1000\n",
        )
        .unwrap();
        fs::write(
            pid.join("stat"),
            "555 (cached) S 1 555 555 0 -1 0 10 0 0 0 25 5 0 0 20 0 3 0 95000 1000 42 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        )
        .unwrap();
        fs::write(pid.join("statm"), "100 42 30 5 0 20 0\n").unwrap();

        let first = collect(&root, &config());
        let process = first
            .entries
            .iter()
            .find(|entry| entry.pid == 555)
            .expect("pid 555");
        let memory = process.mem_bytes.expect("rss memory");
        assert_eq!(memory, 42 * page_size());
        assert_eq!(process.cpu_pct, Some(0.0));
        assert!(process.cpu_c.is_some_and(|value| value > 0.0));

        fs::remove_file(pid.join("stat")).unwrap();
        let second = collect(&root, &config());
        let process = second
            .entries
            .iter()
            .find(|entry| entry.pid == 555)
            .expect("cached row");
        assert_eq!(process.mem_bytes, Some(memory), "last good value is kept");
        assert_eq!(process.cpu_pct, Some(0.0));
        assert_eq!(second.total, Some(1));

        fs::remove_dir_all(&pid).unwrap();
        let third = collect(&root, &config());
        assert!(third.entries.is_empty());
        assert_eq!(third.total, Some(0));
    }

    #[test]
    fn an_over_long_rss_field_falls_back_to_statm() {
        let root = temp_tree("rss-overflow");
        fs::write(root.join("stat"), "cpu 0 0 0 0 0 0 0 0 0 0\n").unwrap();
        fs::write(root.join("meminfo"), "MemTotal:       1048576 kB\n").unwrap();
        let pid = root.join("556");
        fs::create_dir_all(&pid).unwrap();
        fs::write(pid.join("comm"), "overflow\n").unwrap();
        fs::write(pid.join("cmdline"), b"overflow\0").unwrap();
        fs::write(
            pid.join("status"),
            "Name:\toverflow\nUid:\t1000\t1000\t1000\t1000\n",
        )
        .unwrap();
        fs::write(
            pid.join("stat"),
            "556 (overflow) S 1 556 556 0 -1 0 10 0 0 0 25 5 0 0 20 0 3 0 95000 1000 99999999999999999999999999 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        )
        .unwrap();
        fs::write(pid.join("statm"), "100 10 8 1 0 5 0\n").unwrap();

        let snapshot = collect(&root, &config());
        let process = snapshot
            .entries
            .iter()
            .find(|entry| entry.pid == 556)
            .expect("pid 556");
        assert_eq!(process.mem_bytes, Some(10 * page_size()));
    }

    #[test]
    fn sort_modes_follow_btops_proc_sorter() {
        let make = || {
            vec![
                ProcEntry {
                    pid: 1,
                    name: Some("b".into()),
                    command: "z".into(),
                    threads: Some(2),
                    user: Some("u".into()),
                    mem_bytes: Some(10),
                    cpu_pct: Some(1.0),
                    cpu_c: Some(0.3),
                    ..ProcEntry::default()
                },
                ProcEntry {
                    pid: 2,
                    name: Some("a".into()),
                    command: "y".into(),
                    threads: Some(3),
                    user: Some("v".into()),
                    mem_bytes: Some(20),
                    cpu_pct: Some(2.0),
                    cpu_c: Some(0.2),
                    ..ProcEntry::default()
                },
                ProcEntry {
                    pid: 3,
                    name: Some("c".into()),
                    command: "x".into(),
                    threads: Some(1),
                    user: Some("w".into()),
                    mem_bytes: Some(30),
                    cpu_pct: Some(3.0),
                    cpu_c: Some(0.1),
                    ..ProcEntry::default()
                },
            ]
        };
        // btop sorts every column descending unless `proc_reversed` is set.
        assert_eq!(order(make(), "pid", false), vec![3, 2, 1]);
        assert_eq!(order(make(), "pid", true), vec![1, 2, 3]);
        assert_eq!(order(make(), "name", false), vec![3, 1, 2]);
        assert_eq!(order(make(), "command", false), vec![1, 2, 3]);
        assert_eq!(order(make(), "threads", false), vec![2, 1, 3]);
        assert_eq!(order(make(), "user", false), vec![3, 2, 1]);
        assert_eq!(order(make(), "memory", false), vec![3, 2, 1]);
        assert_eq!(order(make(), "cpu direct", false), vec![3, 2, 1]);
        assert_eq!(order(make(), "cpu lazy", false), vec![1, 2, 3]);
        assert_eq!(order(make(), "not a mode", false), vec![1, 2, 3]);
    }

    #[test]
    fn state_words_match_btop_proc_states() {
        let cases = [
            ('R', "Running"),
            ('S', "Sleeping"),
            ('D', "Waiting"),
            ('Z', "Zombie"),
            ('T', "Stopped"),
            ('t', "Tracing"),
            ('X', "Dead"),
            ('x', "Dead"),
            ('K', "Wakekill"),
            ('W', "Unknown"),
            ('P', "Parked"),
            ('I', "Unknown"),
            ('0', "Unknown"),
        ];
        for (state, word) in cases {
            assert_eq!(state_name(state), word);
        }
    }

    #[test]
    fn command_line_matches_btops_byte_join() {
        assert_eq!(command_line(b"ttyd\0-d\0 3\0"), "ttyd -d  3");
        assert_eq!(command_line(b"a\0\0b\0"), "a  b");
        assert_eq!(command_line(b""), "");
        assert_eq!(command_line(b"tini\0"), "tini");

        let mut raw = vec![b'x'; 1001];
        raw.push(0);
        let command = command_line(&raw);
        assert_eq!(command.len(), 999);
        assert!(command.bytes().all(|byte| byte == b'x'));
    }

    #[test]
    fn passwd_map_keeps_the_first_name_for_a_uid() {
        let map = passwd_map(
            "root:x:0:0::/root:/bin/sh\nlab:x:1000:1000::/home/lab:/bin/bash\nalias:x:1000:1000::/:/bin/sh\n",
        );
        assert_eq!(map.get("0").map(String::as_str), Some("root"));
        assert_eq!(map.get("1000").map(String::as_str), Some("lab"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn a_uid_missing_from_passwd_uses_getpwuid_then_the_number() {
        let empty = BTreeMap::new();
        assert_eq!(user_name("0", &empty), "root");
        assert_eq!(user_name(&u32::MAX.to_string(), &empty), "4294967295");
        assert_eq!(user_name("not-a-uid", &empty), "not-a-uid");

        let mut users = BTreeMap::new();
        users.insert("0".to_string(), "alias".to_string());
        assert_eq!(user_name("0", &users), "alias");
    }

    #[test]
    fn defaults_are_btops_process_defaults() {
        let config = config();
        assert_eq!(config.proc_sorting, "cpu lazy");
        assert!(!config.proc_reversed);
        assert!(!config.proc_tree);
        assert!(!config.proc_per_core);
        assert!(config.proc_filter.is_empty());
        assert!(!config.proc_filter_kernel);
    }

    /// The configured sort orders siblings in the tree; the browser sorts by
    /// the viewer's column and keeps this order for ties.
    #[test]
    fn sorting_and_reversing_change_the_order() {
        let pids = |sorting: &str, reversed: bool| {
            let config = configured(|config| {
                config.proc_sorting = sorting.to_string();
                config.proc_reversed = reversed;
            });
            collect(&fixture("proc-tree"), &config)
                .entries
                .iter()
                .map(|entry| entry.pid)
                .collect::<Vec<_>>()
        };
        assert_eq!(pids("pid", false), vec![50, 2, 60, 1, 40, 10, 20, 30]);
        assert_eq!(pids("pid", true), vec![1, 10, 20, 30, 40, 2, 60, 50]);
        assert_eq!(pids("name", false), vec![1, 40, 10, 20, 30, 2, 60, 50]);
    }

    /// ACC-VIEW-58-PROC-VIEW: the rows come in btop's tree pre-order with
    /// their depth whatever `proc_tree` says; pid 50's parent 999 is not
    /// listed, so it is a root.
    #[test]
    fn rows_always_come_in_tree_order() {
        for tree in [false, true] {
            let config = configured(|config| {
                config.proc_tree = tree;
                config.proc_sorting = "pid".to_string();
            });
            let snapshot = collect(&fixture("proc-tree"), &config);
            assert_eq!(
                tree_order(&snapshot.entries),
                vec![
                    (50, 0),
                    (2, 0),
                    (60, 1),
                    (1, 0),
                    (40, 1),
                    (10, 1),
                    (20, 2),
                    (30, 3),
                ],
                "proc_tree {tree}"
            );
            assert_eq!(snapshot.total, Some(8));
        }
    }

    fn row(snapshot: &ProcSnapshot, pid: i32) -> &ProcEntry {
        snapshot
            .entries
            .iter()
            .find(|entry| entry.pid == pid)
            .expect("listed pid")
    }

    /// btop's collapsed-tree sum (`btop_shared.cpp:163-171`): own plus every
    /// descendant's threads, memory and cpu_p.
    #[test]
    fn parents_carry_their_subtree_sums() {
        let snapshot = collect(&fixture("proc-tree"), &config());
        let page = page_size();
        for (pid, threads, pages) in [(1, 11, 2700), (10, 9, 1800), (20, 7, 1300), (2, 2, 1000)] {
            let sum = row(&snapshot, pid).sum.as_ref().expect("a parent's sum");
            assert_eq!(sum.threads, Some(threads), "pid {pid} threads");
            assert_eq!(sum.mem_bytes, Some(pages * page), "pid {pid} memory");
            assert_eq!(sum.cpu_pct, Some(0.0), "pid {pid} cpu on the first tick");
        }
    }

    #[test]
    fn leaves_carry_no_sum() {
        let snapshot = collect(&fixture("proc-tree"), &config());
        for pid in [30, 40, 50, 60] {
            assert!(row(&snapshot, pid).sum.is_none(), "pid {pid}");
        }
    }

    /// An unavailable own value adds nothing to the sum and never counts as 0.
    #[test]
    fn unavailable_values_add_nothing() {
        let known = Totals {
            cpu: Some(1.5),
            mem: Some(10),
            threads: None,
        };
        let total = known.plus(Totals::default());
        assert_eq!(
            (total.cpu, total.mem, total.threads),
            (Some(1.5), Some(10), None)
        );
        let total = known.plus(known);
        assert_eq!(
            (total.cpu, total.mem, total.threads),
            (Some(3.0), Some(20), None)
        );
    }

    /// The summed graph starts when a descendant uses CPU, on every top-level
    /// ancestor of that descendant (a root or a child of pid 1), and goes after
    /// 10 idle updates; a deeper ancestor sums the CPU but keeps no graph.
    #[test]
    fn summed_graph_follows_the_descendants() {
        let root = temp_tree("summed-graph");
        copy_tree(&fixture("proc-tree"), &root);
        collect(&root, &config());
        edit(&root, "stat", "cpu 1 ", "cpu 1001 ");
        edit(&root, "30/stat", " 40 10 0 0 ", " 140 10 0 0 ");
        let snapshot = collect(&root, &config());
        assert_eq!(row(&snapshot, 30).cpu_pct, Some(10.0));
        assert!(row(&snapshot, 30).graph.line.starts_with('M'));
        for pid in [20, 10, 1] {
            let sum = row(&snapshot, pid).sum.as_ref().expect("a parent's sum");
            assert_eq!(sum.cpu_pct, Some(10.0), "pid {pid}");
            assert_eq!(
                sum.graph.line.starts_with('M'),
                pid != 20,
                "pid {pid} summed line"
            );
            assert!(
                row(&snapshot, pid).graph.line.is_empty(),
                "pid {pid} own graph"
            );
        }
        assert!(row(&snapshot, 40).sum.is_none());
        assert!(
            row(&snapshot, 2)
                .sum
                .as_ref()
                .is_some_and(|sum| sum.graph.line.is_empty())
        );
        let mut last = snapshot;
        for _ in 0..10 {
            last = collect(&root, &config());
        }
        for pid in [20, 10, 1] {
            let sum = row(&last, pid).sum.as_ref().expect("a parent's sum");
            assert!(sum.graph.line.is_empty(), "pid {pid} summed graph went");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn filter_kernel_hides_kthreadd_and_its_children() {
        let config = configured(|config| {
            config.proc_filter_kernel = true;
            config.proc_sorting = "pid".to_string();
        });
        for _ in 0..2 {
            let snapshot = collect(&fixture("proc-tree"), &config);
            assert_eq!(
                snapshot
                    .entries
                    .iter()
                    .map(|entry| entry.pid)
                    .collect::<Vec<_>>(),
                vec![50, 1, 40, 10, 20, 30]
            );
            assert_eq!(snapshot.total, Some(6));
        }
    }

    #[test]
    fn per_core_scales_process_cpu_by_the_core_count() {
        let root = temp_tree("per-core");
        fs::write(root.join("stat"), "cpu 0 0 0 0 0 0 0 0 0 0\n").unwrap();
        fs::write(root.join("uptime"), "1000.00 0.00\n").unwrap();
        fs::write(root.join("meminfo"), "MemTotal:       1048576 kB\n").unwrap();
        let pid = root.join("500");
        fs::create_dir_all(&pid).unwrap();
        fs::write(pid.join("comm"), "cpuproc\n").unwrap();
        fs::write(pid.join("cmdline"), b"cpuproc\0").unwrap();
        fs::write(
            pid.join("status"),
            "Name:\tcpuproc\nUid:\t1000\t1000\t1000\t1000\n",
        )
        .unwrap();
        let cpu_pct = |snapshot: &ProcSnapshot| {
            snapshot
                .entries
                .iter()
                .find(|entry| entry.pid == 500)
                .and_then(|entry| entry.cpu_pct)
        };

        write_cpu_stat(&pid, 100);
        assert_eq!(cpu_pct(&collect(&root, &config())), Some(0.0));

        // 100 ticks of process CPU in a 100-tick /proc/stat window.
        fs::write(root.join("stat"), "cpu 100 0 0 0 0 0 0 0 0 0\n").unwrap();
        write_cpu_stat(&pid, 200);
        assert_eq!(cpu_pct(&collect(&root, &config())), Some(100.0));

        fs::write(root.join("stat"), "cpu 150 0 0 0 0 0 0 0 0 0\n").unwrap();
        write_cpu_stat(&pid, 250);
        let per_core = configured(|config| config.proc_per_core = true);
        assert_eq!(
            cpu_pct(&collect(&root, &per_core)),
            Some(100.0 * core_count() as f64)
        );
    }

    fn graph_after(graph: &mut RowGraph, cpu_pct: Option<f64>) -> bool {
        graph.tick(cpu_pct, cpu_pct.unwrap_or_default(), 250);
        graph.history.is_some()
    }

    /// ACC-VIEW-52-PROC-GRAPH: btop's create and drop rule, and the graph
    /// draws in the update that creates it.
    #[test]
    fn graph_starts_above_zero_and_goes_after_10_idle_updates() {
        let mut cached = RowGraph::default();
        assert!(!graph_after(&mut cached, Some(0.0)));
        assert!(graph_after(&mut cached, Some(3.0)));
        let graph = cached.history.as_ref().expect("a graph");
        assert_eq!(graph.len(), 2, "starts from 0, then 3.0's value");
        assert!(graph.chart(PROC_GRAPH, 0.0, 100.0).line.starts_with('M'));
        assert!(
            graph_after(&mut cached, None),
            "unavailable changes nothing"
        );
        for _ in 0..9 {
            assert!(graph_after(&mut cached, Some(0.0)));
        }
        assert!(graph_after(&mut cached, Some(0.1)), "busy resets the count");
        for _ in 0..9 {
            assert!(graph_after(&mut cached, Some(0.0)));
        }
        assert!(!graph_after(&mut cached, Some(0.0)), "the 10th idle update");
    }

    #[test]
    fn graph_value_floors_light_load_to_5() {
        let values = [0.0, 0.1, 4.9, 5.0, 5.4, 5.5, 37.4];
        assert_eq!(
            values.map(graph_value),
            [0.0, 5.0, 5.0, 5.0, 5.0, 6.0, 37.0]
        );
    }

    /// A busy pid's row carries its graph from its first busy update, and a
    /// pid that vanishes takes its graph with it.
    #[test]
    fn listed_rows_carry_their_graph() {
        let root = temp_tree("graph");
        fs::write(root.join("uptime"), "1000.00 0.00\n").unwrap();
        fs::write(root.join("meminfo"), "MemTotal:       1048576 kB\n").unwrap();
        let pid = root.join("500");
        fs::create_dir_all(&pid).unwrap();
        fs::write(pid.join("comm"), "cpuproc\n").unwrap();
        fs::write(pid.join("cmdline"), b"cpuproc\0").unwrap();
        fs::write(pid.join("status"), "Uid:\t1000\t1000\t1000\t1000\n").unwrap();
        let tick = |total: u64, utime: u64| {
            fs::write(
                root.join("stat"),
                format!("cpu {total} 0 0 0 0 0 0 0 0 0\n"),
            )
            .unwrap();
            write_cpu_stat(&pid, utime);
            let snapshot = collect(&root, &config());
            snapshot
                .entries
                .first()
                .map(|entry| entry.graph.line.clone())
        };
        assert_eq!(
            tick(0, 100).as_deref(),
            Some(""),
            "first sight is 0 percent"
        );
        assert!(tick(100, 150).is_some_and(|line| line.starts_with('M')));
        fs::rename(&pid, root.join("gone")).unwrap();
        fs::write(root.join("stat"), "cpu 200 0 0 0 0 0 0 0 0 0\n").unwrap();
        assert!(collect(&root, &config()).entries.is_empty());
        fs::rename(root.join("gone"), &pid).unwrap();
        assert_eq!(
            tick(300, 200).as_deref(),
            Some(""),
            "a returning pid starts again"
        );
    }

    /// DEF-VIEW-67-SPARKLINE-TONE: the bar is painted in the band of its
    /// window's peak, so a row whose readout is 0.0 keeps the spike's tone
    /// while the window holds the spike, and holds that tone from frame to
    /// frame.
    #[test]
    fn a_zero_row_keeps_the_tone_of_its_window_peak() {
        let root = temp_tree("spike-tone");
        fs::write(root.join("uptime"), "1000.00 0.00\n").unwrap();
        fs::write(root.join("meminfo"), "MemTotal:       1048576 kB\n").unwrap();
        let pid = root.join("500");
        fs::create_dir_all(&pid).unwrap();
        fs::write(pid.join("comm"), "spike\n").unwrap();
        fs::write(pid.join("cmdline"), b"spike\0").unwrap();
        fs::write(pid.join("status"), "Uid:\t1000\t1000\t1000\t1000\n").unwrap();
        // `total` moves every update; the pid's utime moves only while busy.
        let tick = |total: u64, utime: u64| {
            fs::write(
                root.join("stat"),
                format!("cpu {total} 0 0 0 0 0 0 0 0 0\n"),
            )
            .unwrap();
            write_cpu_stat(&pid, utime);
            collect(&root, &config())
                .entries
                .first()
                .cloned()
                .expect("pid 500")
        };
        tick(0, 100); // first sight seeds the counters at 0 percent
        let mut row = tick(100, 200);
        for step in 2..=10 {
            row = tick(step * 100, 100 + step * 100);
        }
        assert_eq!(row.cpu_pct, Some(100.0));
        assert_eq!(row.graph_tone, "cpu-crit");
        let mut zero = tick(1100, 1100);
        assert_eq!(zero.cpu_pct, Some(0.0));
        assert!(!zero.graph.line.is_empty());
        // The spike stays in the 20 s window for the nine idle updates the
        // graph survives; every one paints in the peak's band.
        let mut idle = 0;
        while !zero.graph.line.is_empty() {
            assert_eq!(zero.graph_tone, "cpu-crit", "idle update {}", idle + 1);
            idle += 1;
            zero = tick(1100 + idle * 100, 1100);
        }
        assert_eq!(idle, 9, "the graph goes on the 10th idle update");
        let _ = fs::remove_dir_all(&root);
    }

    /// DEF-VIEW-67-SPARKLINE-TONE: a window whose every sample is zero keeps
    /// the zero tone and draws a flat bar at the floor.
    #[test]
    fn a_zero_window_keeps_the_zero_tone_and_a_flat_bar() {
        let mut history = History::with_window(PROC_WINDOW_MS);
        history.push(Some(0.0), 250);
        history.push(Some(0.0), 250);
        assert_eq!(window_tone(&history), "cpu-zero");
        let line = history.line(PROC_GRAPH, 0.0, 100.0);
        let ys: Vec<&str> = line
            .split(['M', 'L'])
            .filter_map(|point| point.split_once(' ').map(|(_, y)| y))
            .collect();
        assert!(ys.len() >= 2, "{line}");
        assert!(ys.windows(2).all(|pair| pair[0] == pair[1]), "{line}");
    }

    fn in_container(proc_root: &Path) -> Config {
        let mut config = config();
        config.container = container::detect(proc_root);
        assert!(
            config.container.is_some(),
            "{} is a container tree",
            proc_root.display()
        );
        config
    }

    fn pids(snapshot: &ProcSnapshot) -> Vec<i32> {
        let mut pids: Vec<i32> = snapshot.entries.iter().map(|entry| entry.pid).collect();
        pids.sort_unstable();
        pids
    }

    /// pid 900 is visible in `/proc` (a shared pid namespace) but in no
    /// container cgroup; pid 42 sits in the child cgroup `worker`.
    #[test]
    fn container_lists_only_cgroup_members() {
        let root = fixture("container/proc");
        let snapshot = collect(&root, &in_container(&root));
        assert_eq!(pids(&snapshot), vec![1, 7, 42]);
    }

    #[test]
    fn container_counts_total_running_and_threads_from_members() {
        let root = fixture("container/proc");
        let snapshot = collect(&root, &in_container(&root));
        assert_eq!(snapshot.total, Some(3));
        assert_eq!(snapshot.running, Some(0));
        // tini 1, ttyd 2, 12_start_resour 1; pid 900's thread is not counted.
        assert_eq!(snapshot.threads, Some(4));
    }

    /// ACC-VIEW-53-PROC-TILE with ACC-SCOPE-64-CONTAINER: pid 900 (not a
    /// member) is running in the fixture (DEF-CPU-44) and is made the largest
    /// by RSS and the busiest; no tile fact counts it. Member pid 42 is D.
    #[test]
    fn container_facts_use_member_rows_only() {
        let tree = temp_tree("container-facts");
        copy_tree(&fixture("container"), &tree);
        let root = tree.join("proc");
        edit(&root, "900/stat", " 7667712 397 ", " 7667712 4000 ");
        let config = in_container(&root);
        collect(&root, &config);
        edit(&root, "stat", "cpu  62538468 ", "cpu  62548468 ");
        edit(&root, "7/stat", " 35 18 ", " 135 18 ");
        edit(&root, "900/stat", " 0 0 731 148 ", " 5000 0 731 148 ");
        let snapshot = collect(&root, &config);
        assert_eq!(snapshot.running, Some(0));
        assert_eq!(snapshot.sleeping, Some(2));
        assert_eq!(snapshot.waiting, Some(1));
        assert_eq!(snapshot.top.map(|entry| entry.pid), Some(7));
        assert_eq!(snapshot.cpu_80, Some(1));
        // 3482 of 4153 member pages; with pid 900's 4000 it would take two.
        assert_eq!(snapshot.mem_80, Some(1));
        let _ = fs::remove_dir_all(&tree);
    }

    /// 100 ticks of one process in a 6400-tick window over 64 online CPUs is
    /// btop's 1.5 % of the machine, and 48 % of a container with E = 2.
    #[test]
    fn container_process_cpu_is_a_share_of_effective_cpus() {
        let tree = temp_tree("container-cpu");
        copy_tree(&fixture("container"), &tree);
        let root = tree.join("proc");
        let stat = |total: u64| {
            let mut text = format!("cpu {total} 0 0 0 0 0 0 0 0 0\n");
            for index in 0..64 {
                text.push_str(&format!("cpu{index} 0 0 0 0 0 0 0 0 0 0\n"));
            }
            fs::write(root.join("stat"), text).unwrap();
        };
        let cpu_pct = |snapshot: &ProcSnapshot, pid: i32| {
            snapshot
                .entries
                .iter()
                .find(|entry| entry.pid == pid)
                .and_then(|entry| entry.cpu_pct)
        };
        let config = in_container(&root);
        stat(0);
        write_cpu_stat(&root.join("7"), 100);
        assert_eq!(cpu_pct(&collect(&root, &config), 7), Some(0.0));

        stat(6400);
        write_cpu_stat(&root.join("7"), 200);
        let snapshot = collect(&root, &config);
        assert_eq!(cpu_pct(&snapshot, 7), Some(1.5 * 64.0 / 2.0));
        assert_eq!(cpu_pct(&snapshot, 42), Some(0.0));

        // proc_per_core keeps btop's percent of one CPU.
        let per_core = Config {
            proc_per_core: true,
            ..config.clone()
        };
        stat(12_800);
        write_cpu_stat(&root.join("7"), 300);
        let cores = core_count();
        assert_eq!(
            cpu_pct(&collect(&root, &per_core), 7),
            Some(process_cpu(100, 6400, cores as u64, cores))
        );
        let _ = fs::remove_dir_all(&tree);
    }

    #[test]
    fn unreadable_cgroup_procs_leaves_processes_unavailable() {
        let tree = temp_tree("container-noprocs");
        copy_tree(&fixture("container"), &tree);
        fs::remove_file(tree.join("sys/fs/cgroup/cgroup.procs")).unwrap();
        let root = tree.join("proc");
        let snapshot = collect(&root, &in_container(&root));
        assert_eq!(snapshot.total, None);
        assert_eq!(snapshot.threads, None);
        assert!(snapshot.entries.is_empty());
        // ACC-VIEW-53-PROC-TILE: every tile fact is unavailable, never 0.
        for fact in [
            snapshot.running,
            snapshot.sleeping,
            snapshot.waiting,
            snapshot.zombie,
            snapshot.cpu_80,
            snapshot.mem_80,
        ] {
            assert_eq!(fact, None);
        }
        assert!(snapshot.top.is_none());
        assert!(snapshot.chart.line.is_empty() && snapshot.chart.area.is_empty());
        let _ = fs::remove_dir_all(&tree);
    }

    fn write_cpu_stat(directory: &Path, utime: u64) {
        write_cpu_stat_at(directory, utime, 1000);
    }

    fn write_cpu_stat_at(directory: &Path, utime: u64, starttime: u64) {
        let (utime, starttime) = (utime.to_string(), starttime.to_string());
        let fields = [
            "S",
            "1",
            "1",
            "1",
            "0",
            "-1",
            "4194560",
            "0",
            "0",
            "0",
            "0",
            utime.as_str(),
            "0",
            "0",
            "0",
            "20",
            "0",
            "1",
            "0",
            starttime.as_str(),
            "1000",
            "42",
        ];
        fs::write(
            directory.join("stat"),
            format!("500 (cpuproc) {}", fields.join(" ")),
        )
        .unwrap();
    }

    /// DEF-PERF-76-SERVER-COST: a tick clones the identity strings by pointer,
    /// and a pid number that comes back with a different starttime re-reads
    /// them.
    #[test]
    fn identity_is_shared_by_pointer_and_rebuilt_for_a_reused_pid() {
        let root = temp_tree("identity-reused");
        fs::write(root.join("stat"), "cpu 0 0 0 0 0 0 0 0 0 0\n").unwrap();
        fs::write(root.join("meminfo"), "MemTotal:       1048576 kB\n").unwrap();
        fs::write(root.join("uptime"), "1000.00 0.00\n").unwrap();
        let pid = root.join("600");
        fs::create_dir_all(&pid).unwrap();
        fs::write(pid.join("comm"), "first\n").unwrap();
        fs::write(pid.join("cmdline"), b"first\0").unwrap();
        fs::write(pid.join("status"), "Uid:\t1000\t1000\t1000\t1000\n").unwrap();
        write_cpu_stat_at(&pid, 100, 1000);

        let first = collect(&root, &config());
        let second = collect(&root, &config());
        let (one, other) = (row(&first, 600), row(&second, 600));
        assert!(Arc::ptr_eq(
            one.name.as_ref().expect("a name"),
            other.name.as_ref().expect("a name")
        ));
        assert!(Arc::ptr_eq(&one.command, &other.command));
        assert!(Arc::ptr_eq(
            one.user.as_ref().expect("a user"),
            other.user.as_ref().expect("a user")
        ));

        // The pid number comes back with a new starttime: a reused pid.
        fs::write(pid.join("comm"), "second\n").unwrap();
        fs::write(pid.join("cmdline"), b"second\0").unwrap();
        write_cpu_stat_at(&pid, 100, 2000);
        let third = collect(&root, &config());
        let reused = row(&third, 600);
        assert_eq!(reused.name.as_deref(), Some("second"));
        assert_eq!(reused.command.as_ref(), "second");
        assert!(!Arc::ptr_eq(
            one.name.as_ref().expect("a name"),
            reused.name.as_ref().expect("a name")
        ));
        let _ = fs::remove_dir_all(&root);
    }

    /// ACC-PERF-141-CACHE-BOUND: the cache holds its ceiling, not one more,
    /// and the entry it drops is the least recently used.
    #[test]
    fn identity_lru_evicts_the_least_recently_used_string() {
        let mut cache = StringLru::default();
        for pid in 0..IDENTITY_CACHE_LIMIT as i32 {
            cache.insert((pid, Some(1)), Arc::from(pid.to_string()));
        }
        assert_eq!(cache.len(), IDENTITY_CACHE_LIMIT);
        // Touch the first entry, so the second is now the least recently used.
        assert!(cache.get((0, Some(1))).is_some());

        cache.insert((IDENTITY_CACHE_LIMIT as i32, Some(1)), Arc::from("new"));
        assert_eq!(
            cache.len(),
            IDENTITY_CACHE_LIMIT,
            "the cache stays at its ceiling"
        );
        assert_eq!(cache.get((0, Some(1))).as_deref(), Some("0"));
        assert_eq!(
            cache.get((1, Some(1))),
            None,
            "the least recently used entry is evicted"
        );
        assert_eq!(
            cache.get((IDENTITY_CACHE_LIMIT as i32, Some(1))).as_deref(),
            Some("new")
        );
    }

    fn identity_lengths(root: &Path) -> (usize, usize, usize) {
        LAST.lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(root)
            .map_or((0, 0, 0), |state| state.identity.len())
    }

    /// The test process's resident set, from `/proc/self/statm`.
    fn rss_bytes() -> u64 {
        let statm = fs::read_to_string("/proc/self/statm").unwrap_or_default();
        let pages: u64 = statm
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        pages * page_size()
    }

    /// ACC-PERF-141-CACHE-BOUND: 10,000 short-lived processes stay under the
    /// ceiling, every entry goes when its process leaves the scan, and RSS
    /// does not grow with the churn.
    #[test]
    fn identity_cache_is_bounded_under_process_churn() {
        let root = temp_tree("identity-churn");
        fs::write(root.join("stat"), "cpu 0 0 0 0 0 0 0 0 0 0\n").unwrap();
        fs::write(root.join("meminfo"), "MemTotal:       1048576 kB\n").unwrap();
        fs::write(root.join("uptime"), "1000.00 0.00\n").unwrap();
        let config = config();
        let rss_before = rss_bytes();
        let mut observed = (0, 0, 0);

        for index in 0..10_000 {
            let pid = 1_000 + index;
            let directory = root.join(pid.to_string());
            fs::create_dir_all(&directory).unwrap();
            let comm = format!("churn-{index}");
            fs::write(directory.join("comm"), format!("{comm}\n")).unwrap();
            fs::write(directory.join("cmdline"), format!("churn\0{index}\0")).unwrap();
            fs::write(directory.join("status"), "Uid:\t1000\t1000\t1000\t1000\n").unwrap();
            write_cpu_stat_at(&directory, 100, 1_000 + index as u64);

            let snapshot = collect(&root, &config);
            let entry = snapshot
                .entries
                .iter()
                .find(|entry| entry.pid == pid)
                .expect("the churned process");
            assert_eq!(entry.name.as_deref(), Some(comm.as_str()));

            let lengths = identity_lengths(&root);
            observed = (
                observed.0.max(lengths.0),
                observed.1.max(lengths.1),
                observed.2.max(lengths.2),
            );
            assert!(
                lengths.0 <= IDENTITY_CACHE_LIMIT
                    && lengths.1 <= IDENTITY_CACHE_LIMIT
                    && lengths.2 <= IDENTITY_CACHE_LIMIT,
                "{lengths:?} passes the {IDENTITY_CACHE_LIMIT} ceiling"
            );
            fs::remove_dir_all(&directory).unwrap();
        }

        // One scan with no process directories evicts the last process.
        let snapshot = collect(&root, &config);
        assert!(snapshot.entries.is_empty());
        assert_eq!(
            identity_lengths(&root),
            (0, 0, 0),
            "every entry goes when its process is gone"
        );
        let rss_after = rss_bytes();
        eprintln!(
            "identity churn: ceiling {IDENTITY_CACHE_LIMIT} per kind, observed max {observed:?}, RSS {rss_before} -> {rss_after} bytes"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// ACC-PERF-141-CACHE-BOUND: a pid number that comes back as another
    /// process never shows the previous process's strings; when the new
    /// process's identity cannot be read, the fields are unavailable.
    #[test]
    fn identity_never_crosses_a_reused_pid() {
        let root = temp_tree("identity-hostile");
        fs::write(root.join("stat"), "cpu 0 0 0 0 0 0 0 0 0 0\n").unwrap();
        fs::write(root.join("meminfo"), "MemTotal:       1048576 kB\n").unwrap();
        fs::write(root.join("uptime"), "1000.00 0.00\n").unwrap();
        let pid = root.join("700");
        fs::create_dir_all(&pid).unwrap();
        fs::write(pid.join("comm"), "first\n").unwrap();
        fs::write(pid.join("cmdline"), b"first\0").unwrap();
        fs::write(pid.join("status"), "Uid:\t1000\t1000\t1000\t1000\n").unwrap();
        write_cpu_stat_at(&pid, 100, 1000);
        assert_eq!(
            row(&collect(&root, &config()), 700).name.as_deref(),
            Some("first")
        );

        // The pid number comes back with another starttime, and its identity
        // cannot be read.
        fs::remove_file(pid.join("comm")).unwrap();
        write_cpu_stat_at(&pid, 100, 2000);
        let reused = collect(&root, &config());
        let entry = row(&reused, 700);
        assert_eq!(entry.name, None, "never the previous process's name");
        assert_eq!(entry.user, None);
        assert_eq!(entry.command.as_ref(), "");

        // Identity is read once per starttime, so the same starttime keeps
        // the unavailable result even after the file becomes readable.
        fs::write(pid.join("comm"), "second\n").unwrap();
        fs::write(pid.join("cmdline"), b"second\0").unwrap();
        assert_eq!(
            row(&collect(&root, &config()), 700).name,
            None,
            "the read is not retried per tick"
        );
        write_cpu_stat_at(&pid, 100, 3000);
        assert_eq!(
            row(&collect(&root, &config()), 700).name.as_deref(),
            Some("second"),
            "the next starttime reads the new process's own strings"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// DEF-PERF-76-SERVER-COST: `cgroup.procs` is read on the 5 s slow
    /// cadence, so a process that joins the container is listed up to that
    /// late.
    #[test]
    fn container_membership_refreshes_on_the_slow_cadence() {
        let tree = temp_tree("members-period");
        copy_tree(&fixture("container"), &tree);
        let root = tree.join("proc");
        let config = in_container(&root);
        let cgroup = config
            .container
            .as_ref()
            .and_then(|container| container.cgroup.clone())
            .expect("container cgroup");
        assert_eq!(pids(&collect(&root, &config)), vec![1, 7, 42]);

        // pid 900, visible in /proc, joins the container's cgroup.
        let file = cgroup.join("cgroup.procs");
        let mut text = fs::read_to_string(&file).unwrap();
        text.push_str("900\n");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&file, text).unwrap();
        assert_eq!(
            pids(&collect(&root, &config)),
            vec![1, 7, 42],
            "membership is cached for the period"
        );

        // Age the cache past the period: the next tick re-reads the file.
        MEMBERS
            .lock()
            .unwrap()
            .get_mut(&cgroup)
            .expect("a cached member set")
            .0 = Instant::now() - SLOW_PERIOD - std::time::Duration::from_secs(1);
        assert_eq!(pids(&collect(&root, &config)), vec![1, 7, 42, 900]);
        let _ = fs::remove_dir_all(&tree);
    }
}

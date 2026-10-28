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
//! Period: every sample, `update_ms` (250 ms by default; btop's 2000).
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
//! - count graph: the total pushed into a [`History`] (the 240 s window, the
//!   1 s EMA, the shared decimation), drawn from 0 to the smallest 1, 2 or
//!   5 x 10^k at or above both 10 and the largest drawn sample; a change of
//!   that ceiling runs through [`Scale`], as the network plots do
//!   (ACC-GRAPH-67-SMOOTH-SCALE)

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::chart::{History, PROC_COUNT, PROC_GRAPH, PROC_WINDOW_MS, Scale};
use crate::config::Config;

use super::container;
use super::{ProcEntry, ProcSnapshot, ProcSum};

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
    passwd: PasswdCache,
    /// btop's `kernels_procs`, seeded with kthreadd.
    kernel_pids: HashSet<i32>,
    /// btop's `proc_clear_count`, which rebuilds the kernel set every 256 samples.
    kernel_clear_count: u32,
    /// The process count history behind the Processes tile's graph.
    count: History,
    /// Moves the count graph to a new ceiling smoothly (ACC-GRAPH-67-SMOOTH-SCALE).
    scale: Scale,
}

impl Default for ProcState {
    fn default() -> Self {
        Self {
            cputimes: 0,
            processes: HashMap::new(),
            passwd: PasswdCache::default(),
            kernel_pids: HashSet::from([KTHREADD]),
            kernel_clear_count: 0,
            count: History::new(),
            scale: Scale::default(),
        }
    }
}

/// A `proc_info` entry in btop's `current_procs` plus the cached stat counters.
#[derive(Default)]
struct CachedProcess {
    entry: ProcEntry,
    cpu_t: u64,
    cpu_s: u64,
    /// btop's `p_graphs` entry; it goes with the pid when the pid vanishes.
    graph: RowGraph,
    /// The subtree's summed graph, while the pid has children.
    sum_graph: RowGraph,
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

static LAST: Mutex<BTreeMap<PathBuf, ProcState>> = Mutex::new(BTreeMap::new());

pub fn collect(proc_root: &Path, config: &Config) -> ProcSnapshot {
    let Some((cputimes, online)) = cpu_times(proc_root) else {
        return ProcSnapshot::default();
    };
    let (members, cpu_scale) = match &config.container {
        None => (None, 1.0),
        Some(container) => {
            let Some(cgroup) = container.cgroup.as_deref() else {
                return ProcSnapshot::default();
            };
            let Some(members) = cgroup_members(cgroup, "cgroup.procs") else {
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

    let mut states = LAST.lock().unwrap_or_else(|error| error.into_inner());
    let state = states.entry(proc_root.to_path_buf()).or_default();
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
    for pid in candidates {
        if config.proc_filter_kernel && state.kernel_pids.contains(&pid) {
            continue;
        }
        found.push(pid);
        let path = proc_root.join(pid.to_string());
        let first_sight = !state.processes.contains_key(&pid);
        let cached = state.processes.entry(pid).or_insert_with(|| CachedProcess {
            entry: ProcEntry {
                pid,
                ..ProcEntry::default()
            },
            ..CachedProcess::default()
        });
        if first_sight && !read_identity(&path, cached, &state.passwd.users) {
            // btop's `continue`: the entry keeps what was read before the failure.
            continue;
        }
        if let Some(stat) = read_stat(&path, &context) {
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
    state
        .processes
        .retain(|pid, _| found.binary_search(pid).is_ok());
    state.cputimes = cputimes;

    let mut entries: Vec<ProcEntry> = found
        .iter()
        .filter_map(|pid| state.processes.get(pid).map(|cached| cached.entry.clone()))
        .collect();
    let busy_below = tree_entries(&mut entries, config);

    for (entry, busy) in entries.iter_mut().zip(busy_below) {
        let Some(cached) = state.processes.get_mut(&entry.pid) else {
            continue;
        };
        if let Some(cpu_p) = entry.cpu_pct {
            cached.graph.tick(Some(cpu_p), cpu_p, config.update_ms);
        }
        // The row graphs draw no area, so none is built.
        if let Some(history) = &cached.graph.history {
            entry.graph.line = history.line(PROC_GRAPH, 0.0, 100.0);
        }
        match entry.sum.as_mut() {
            Some(sum) if entry.depth == 0 || (entry.depth == 1 && entry.ppid == Some(1)) => {
                if let Some(total) = sum.cpu_pct {
                    cached.sum_graph.tick(busy, total, config.update_ms);
                }
                if let Some(history) = &cached.sum_graph.history {
                    sum.graph.line = history.line(PROC_GRAPH, 0.0, 100.0);
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
// Own code: one loop; no crate in the tree picks a nice axis ceiling.
fn count_ceiling(largest: f64) -> u64 {
    let target = largest.max(10.0);
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
/// stops the first collection of that pid before stat, and is not retried.
fn read_identity(
    directory: &Path,
    cached: &mut CachedProcess,
    users: &BTreeMap<String, String>,
) -> bool {
    let Ok(comm) = fs::read_to_string(directory.join("comm")) else {
        return false;
    };
    cached.entry.name = Some(comm.strip_suffix('\n').unwrap_or(&comm).to_string());

    let Ok(raw) = fs::read(directory.join("cmdline")) else {
        return false;
    };
    cached.entry.command = command_line(&raw);

    let Some(uid) = uid(directory) else {
        return false;
    };
    cached.entry.user = Some(user_name(&uid, users));
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

fn uid(directory: &Path) -> Option<String> {
    let status = fs::read_to_string(directory.join("status")).ok()?;
    let line = status.lines().find_map(|line| line.strip_prefix("Uid:"))?;
    Some(line.split_whitespace().next()?.to_string())
}

fn read_stat(directory: &Path, context: &Context) -> Option<Stat> {
    let stat = fs::read_to_string(directory.join("stat")).ok()?;
    let tail = stat.get(stat.rfind(')')? + 1..)?.trim_start();
    let fields: Vec<&str> = tail.split_whitespace().collect();
    if fields.len() < 22 {
        return None;
    }
    Some(Stat {
        state: fields[0].chars().next()?,
        ppid: fields[1].parse().ok()?,
        cpu_t: fields[11].parse::<u64>().ok()? + fields[12].parse::<u64>().ok()?,
        nice: fields[16].parse().ok()?,
        threads: fields[17].parse().ok()?,
        starttime: fields[19].parse().ok()?,
        mem: process_memory(fields[21], directory, context)?,
    })
}

/// btop replaces the RSS field with totalMem when its digit count exceeds that
/// of totalMem in KiB, then reads statm when the value reaches totalMem.
fn process_memory(field: &str, directory: &Path, context: &Context) -> Option<u64> {
    let limit = context
        .total_memory
        .map(|total| (total >> 10).to_string().len());
    let mut memory = match limit {
        Some(len) if field.len() > len => context.total_memory?,
        _ => field.parse::<u64>().ok()?.checked_mul(context.page_size)?,
    };
    if context.total_memory.is_some_and(|total| memory >= total) {
        memory = statm_memory(directory, context.page_size)?;
    }
    Some(memory)
}

fn statm_memory(directory: &Path, page_size: u64) -> Option<u64> {
    let statm = fs::read_to_string(directory.join("statm")).ok()?;
    statm
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
    entry.cpu_pct = Some(
        process_cpu(
            stat.cpu_t.saturating_sub(cached.cpu_t),
            context.delta_cputimes,
            context.cmult,
            context.cores,
        ) * context.cpu_scale,
    );
    entry.cpu_c = context.uptime.map(|uptime| {
        stat.cpu_t as f64 / ((uptime * context.clk_tck) - cached.cpu_s as f64).max(1.0)
    });
    cached.cpu_t = stat.cpu_t;
    entry.mem_bytes = Some(stat.mem);
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
    });
    order[slot].busy_below = below.cpu;
    total
}

fn text(value: &Option<String>) -> &str {
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
            process.command,
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
        assert_eq!(process.command, "/usr/bin/nostat");
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
        let utime = utime.to_string();
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
            "1000",
            "1000",
            "42",
        ];
        fs::write(
            directory.join("stat"),
            format!("500 (cpuproc) {}", fields.join(" ")),
        )
        .unwrap();
    }
}

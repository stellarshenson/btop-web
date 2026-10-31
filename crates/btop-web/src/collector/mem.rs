//! Memory collector, mirroring `Mem::collect` and `Mem::get_totalMem` in
//! btop's `src/linux/btop_collect.cpp`.
//!
//! Raw source: `<proc_root>/meminfo` (MemTotal, MemFree, MemAvailable, Cached,
//! SwapTotal, SwapFree, all in kB), the optional
//! `<proc_root>/spl/kstat/zfs/arcstats` (`size`, `c_min`),
//! `<proc_root>/filesystems`, the mount table (`<etc>/mtab` when it exists,
//! otherwise `<proc_root>/self/mounts`), `<etc>/fstab` with `use_fstab`, and
//! `statvfs(3)` on every mountpoint under `<root>`. Disk IO comes from
//! `<sys_root>/block/<dev>/stat`, or the ZFS kstat files under
//! `<proc_root>/spl/kstat/zfs`.
//!
//! `<root>`, `<etc>` and `<sys_root>` are derived from the `proc_root` parent,
//! so a recorded tree pairs `<tree>/proc` with `<tree>`, `<tree>/etc` and
//! `<tree>/sys`, while live `/proc` maps to `/`, `/etc` and `/sys`.
//!
//! Formula (btop): available = MemAvailable, or MemFree + Cached when the
//! kernel lacks MemAvailable; with ZFS ARC, available += arc size above
//! c_min. used = MemTotal - available, but MemFree when available exceeds
//! MemTotal. Percentages are `round(stats * 100 / total)`.
//! Memory bar (DEF-VIEW-50-MEM-TERMS, owner 2026-10-28), in place of btop's
//! cached row: used | cache | free = MemTotal, free = MemFree and cache the
//! remainder, MemAvailable - MemFree, the memory the kernel can reclaim; so
//! available = free + cache. Btop's Cached is not a segment: it leaves
//! Buffers and reclaimable slab out of the sum. [`split`] clamps each part to
//! what is left of the total, so MemAvailable < MemFree gives free =
//! MemAvailable and cache 0.
//! Disk total = f_blocks * f_frsize; free = f_bavail * f_frsize (f_bfree with
//! `disk_free_priv`); used = total - free; used percent =
//! round(used * 100 / total); free percent = 100 - used percent. Block IO is
//! `max(0, sectors delta) * 512`, and activity =
//! clamp(round(io_ticks delta / uptime delta / 10), 0, 100).
//!
//! Period: every sample, `update_ms` (250 ms by default; btop's 2000). Deltas, the fstab
//! cache and the statvfs ignore list persist across samples in
//! [`MemCollector`].
//!
//! Container (ACC-SCOPE-64-CONTAINER): inside a container this collector
//! departs from btop on the owner's order (2026-10-27) and reads the
//! container's cgroup v2 files. Every sample, `update_ms`, in bytes:
//! - used U = max(0, `memory.current` - `inactive_file`), docker's working
//!   set (DEF-VIEW-96-MEM-WRONG); cache = `inactive_file`, this container's
//!   reclaimable file cache. `active_file` and `slab_reclaimable` are part of
//!   the container's use and are not subtracted
//! - T = MemTotal when `memory.max` is `max`, missing or at least MemTotal:
//!   total, free and the other rows are the host's (DEF-VIEW-101-MEM-BREAKDOWN).
//!   free = MemFree; available = MemAvailable. other reclaimable =
//!   MemAvailable - MemFree - (`inactive_file` + `active_file` +
//!   `slab_reclaimable`): MemAvailable counts all three of this container's as
//!   reclaimable, though docker's working set keeps the last two in used.
//!   other used is the remainder, T - the other four rows, so the bar is
//!   used | cache | other used | other reclaimable | free = T exactly
//! - when `memory.max` is below MemTotal the limit binds: T = `memory.max`,
//!   free = T - U - cache, this container's room under its limit, and both
//!   other rows are dropped; the bar is used | cache | free = T; available =
//!   free + cache
//! - percentages round(x * 100 / T). The ZFS ARC is not charged to a cgroup
//!   and is not added
//! - swap used = `memory.swap.current`; swap total = min(`memory.swap.max`,
//!   SwapTotal), SwapTotal when `max`; free = total - used. A missing
//!   `memory.swap.current` leaves swap unavailable
//! - disks keep the mount table and `statvfs`, which answer for the
//!   container's mount namespace; IO activity, read and write are withheld
//!   (`io.stat` has no busy time), so no block stat or uptime is read

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use nix::sys::statvfs::statvfs;

use super::{DiskUsage, MemSnapshot, sys_root};
use crate::chart::{History, MEM_HISTORY, smooth};
use crate::config::Config;

/// State btop keeps in `Mem::` statics between samples.
#[derive(Debug, Default)]
pub struct MemCollector {
    fstab: Vec<String>,
    fstab_mtime: Option<SystemTime>,
    ignore_list: Vec<String>,
    disks: BTreeMap<String, DiskState>,
    old_uptime: f64,
    /// btop's used-memory graph history, one sample per tick.
    used_history: History,
    /// The 1 s EMA of each Memory bar segment's share, in bar order
    /// (ACC-GRAPH-47-GAUGE-SMOOTH, ACC-VIEW-190-MEM-RECLAIM).
    segment_ema: Vec<f64>,
}

#[derive(Debug, Default)]
struct DiskState {
    name: String,
    dev: String,
    fstype: String,
    stat: Option<PathBuf>,
    old_io: [i64; 3],
    io_read: Option<u64>,
    io_write: Option<u64>,
    io_activity: Option<f64>,
    total: Option<u64>,
    free: Option<u64>,
    used: Option<u64>,
    used_percent: Option<f64>,
    free_percent: Option<f64>,
}

impl MemCollector {
    pub fn collect(&mut self, proc_root: &Path, config: &Config) -> MemSnapshot {
        let swap_enabled = config.show_swap || config.swap_disk;
        let Ok(text) = std::fs::read_to_string(proc_root.join("meminfo")) else {
            return MemSnapshot {
                swap_enabled,
                ..MemSnapshot::default()
            };
        };
        let stats = parse_meminfo(&text);
        let Some(total) = stats.get("MemTotal").copied() else {
            return MemSnapshot {
                swap_enabled,
                ..MemSnapshot::default()
            };
        };
        if let Some(container) = &config.container {
            let snapshot = container_memory(container.cgroup.as_deref(), &stats, total, config);
            return self.finish(proc_root, config, snapshot);
        }

        let free = stats.get("MemFree").copied();
        let cached = stats.get("Cached").copied();
        let mut available = stats
            .get("MemAvailable")
            .copied()
            .or_else(|| match (free, cached) {
                (Some(free), Some(cached)) => Some(free + cached),
                _ => None,
            });

        // btop's zfs_arc_cached: only the ARC above c_min is available to
        // applications.
        if config.zfs_arc_cached
            && let Some((size, minimum)) = zfs_arc(proc_root)
            && let Some(value) = available.as_mut()
        {
            *value += size.saturating_sub(minimum);
        }

        let used = match (available, free) {
            (Some(available), _) if available <= total => Some(total - available),
            (_, Some(free)) => Some(total.saturating_sub(free)),
            _ => None,
        };
        let (free, cache) = match (used, free) {
            (Some(used), Some(free)) => {
                let ([_, free], cache) = split(total, [used, free]);
                (Some(free), Some(cache))
            }
            _ => (free, None),
        };

        // btop stops reading meminfo after Cached when both swap keys are off.
        let (swap_total, swap_free) = if config.show_swap || config.swap_disk {
            (
                stats.get("SwapTotal").copied(),
                stats.get("SwapFree").copied(),
            )
        } else {
            (None, None)
        };
        let swap_used = match (swap_total, swap_free) {
            (Some(total), Some(free)) if total > 0 => Some(total - free),
            (Some(0), _) => Some(0),
            _ => None,
        };
        let has_swap = config.show_swap && swap_total.is_some_and(|total| total > 0);
        let swap_percent = swap_total
            .filter(|_| has_swap)
            .and_then(|total| swap_used.map(|used| percent(used, total)));
        let swap_free_percent = swap_total
            .filter(|_| has_swap)
            .and_then(|total| swap_free.map(|free| percent(free, total)));

        let snapshot = MemSnapshot {
            ram_total: Some(total),
            ram_used: used,
            ram_free: free,
            ram_available: available,
            ram_cache: cache,
            ram_percent: used.map(|used| percent(used, total)),
            ram_free_percent: free.map(|free| percent(free, total)),
            ram_available_percent: available.map(|available| percent(available, total)),
            ram_cache_percent: cache.map(|cache| percent(cache, total)),
            swap_total,
            swap_used,
            swap_free,
            swap_percent,
            swap_free_percent,
            has_swap,
            swap_enabled,
            disks: Vec::new(),
            ..MemSnapshot::default()
        };
        self.finish(proc_root, config, snapshot)
    }

    /// The disks and the used history, shared by the host and container paths.
    fn finish(
        &mut self,
        proc_root: &Path,
        config: &Config,
        mut snapshot: MemSnapshot,
    ) -> MemSnapshot {
        if config.show_disks {
            snapshot.disks = self.collect_disks(proc_root, config, &snapshot);
        }
        self.used_history.set_window(config.window_ms());
        self.used_history
            .push(snapshot.ram_percent, config.update_ms);
        snapshot.gauge_pct = snapshot.ram_percent.and(self.used_history.ema());
        // ACC-VIEW-192-GAUGE-CONTAINER: the arc draws used and cache, the
        // container's own in a container. A part turning unknown empties the
        // list, so the average starts over.
        let segments: Vec<_> = snapshot
            .ram_segments()
            .into_iter()
            .filter(|(name, _)| matches!(*name, "used" | "cache"))
            .collect();
        if self.segment_ema.len() != segments.len() {
            self.segment_ema.clear();
        }
        self.segment_ema = segments
            .iter()
            .enumerate()
            .map(|(at, (_, share))| {
                smooth(self.segment_ema.get(at).copied(), *share, config.update_ms)
            })
            .collect();
        snapshot.gauge_segments = segments
            .iter()
            .zip(&self.segment_ema)
            .map(|((name, _), share)| (*name, *share))
            .collect();
        snapshot.used_chart = self.used_history.chart(MEM_HISTORY, 0.0, 100.0);
        snapshot
    }

    /// The disk block of `Mem::collect`, from the mount table through the IO
    /// deltas.
    fn collect_disks(
        &mut self,
        proc_root: &Path,
        config: &Config,
        mem: &MemSnapshot,
    ) -> Vec<DiskUsage> {
        //? Get list of "real" filesystems from /proc/filesystems
        let fstypes = if config.only_physical && !config.use_fstab {
            let Ok(filesystems) = std::fs::read_to_string(proc_root.join("filesystems")) else {
                return Vec::new();
            };
            let mut types = vec!["zfs".to_string(), "wslfs".to_string(), "drvfs".to_string()];
            for line in filesystems.lines() {
                let Some(kind) = line.split_whitespace().next() else {
                    continue;
                };
                if !matches!(kind, "nodev" | "squashfs" | "nullfs") {
                    types.push(kind.to_string());
                }
            }
            types
        } else {
            Vec::new()
        };

        let etc = etc_root(proc_root);
        if config.use_fstab && !self.load_fstab(&etc.join("fstab")) {
            return Vec::new();
        }

        //? Get mounts from /etc/mtab or /proc/self/mounts
        let mtab = etc.join("mtab");
        let mounts_path = if mtab.exists() {
            mtab
        } else {
            proc_root.join("self/mounts")
        };
        let Ok(mounts) = std::fs::read_to_string(mounts_path) else {
            return Vec::new();
        };

        let filter = Filter::parse(&config.disks_filter);
        let mut found: Vec<String> = Vec::new();
        for line in mounts.lines() {
            let mut fields = line.split_whitespace();
            let (Some(device), Some(mountpoint), Some(fstype)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let mount = mountpoint.to_string();
            if self.ignore_list.contains(&mount) || found.contains(&mount) {
                continue;
            }
            if filter.as_ref().is_some_and(|filter| !filter.allows(&mount)) {
                continue;
            }

            //? Skip ZFS datasets if zfs_hide_datasets is enabled
            let dataset_name_start = if fstype == "zfs" {
                device.find('/')
            } else {
                None
            };
            if fstype == "zfs" && dataset_name_start.is_some() && config.zfs_hide_datasets {
                continue;
            }

            let included = (!config.use_fstab && !config.only_physical)
                || (config.use_fstab && self.fstab.iter().any(|fstab_mount| fstab_mount == &mount))
                || (!config.use_fstab
                    && config.only_physical
                    && fstypes.iter().any(|kind| kind == fstype));
            if !included {
                continue;
            }

            found.push(mount.clone());
            if let Entry::Vacant(slot) = self.disks.entry(mount) {
                let dev = std::fs::canonicalize(device).map_or_else(
                    |_| device.to_string(),
                    |path| path.to_string_lossy().into_owned(),
                );
                let name = disk_name(slot.key());
                let stat = block_stat_path(proc_root, &dev, fstype, dataset_name_start, config);
                slot.insert(DiskState {
                    name,
                    dev,
                    fstype: fstype.to_string(),
                    stat,
                    ..DiskState::default()
                });
            }
        }

        if config.swap_disk && mem.has_swap {
            found.push("swap".to_string());
        }
        self.disks.retain(|mount, _| found.contains(mount));

        //? Get disk/partition stats with statvfs
        let disk_free_priv = config.disk_free_priv;
        let root = tree_root(proc_root);
        let mut failed: Vec<String> = Vec::new();
        for mount in self.disks.keys().cloned().collect::<Vec<_>>() {
            match statvfs(&root.join(mount.trim_start_matches('/'))) {
                Ok(stats) => {
                    let block_size = stats.fragment_size();
                    let total = stats.blocks().saturating_mul(block_size);
                    let blocks = if disk_free_priv {
                        stats.blocks_free()
                    } else {
                        stats.blocks_available()
                    };
                    let free = blocks.saturating_mul(block_size);
                    let used = total.saturating_sub(free);
                    if let Some(disk) = self.disks.get_mut(&mount) {
                        disk.total = Some(total);
                        disk.free = Some(free);
                        disk.used = Some(used);
                        disk.used_percent = (total > 0).then(|| percent(used, total));
                        disk.free_percent = disk.used_percent.map(|percent| 100.0 - percent);
                    }
                }
                Err(_) => failed.push(mount),
            }
        }
        // btop's ignore_list is permanent for the process lifetime.
        for mount in failed {
            self.ignore_list.push(mount.clone());
            self.disks.remove(&mount);
        }

        //? Setup disk order: root first, then swap, then mount-table order
        let mut order: Vec<String> = Vec::new();
        if self.disks.contains_key("/") {
            order.push("/".to_string());
        }
        if config.swap_disk && mem.has_swap {
            order.push("swap".to_string());
        }
        for mount in &found {
            if mount != "/" && mount != "swap" && self.disks.contains_key(mount) {
                order.push(mount.clone());
            }
        }

        //? Get disks IO, withheld in a container
        if config.container.is_none() {
            self.disk_io(proc_root, config);
        }

        let mut disks = Vec::with_capacity(order.len());
        for mount in order {
            if mount == "swap" {
                disks.push(DiskUsage {
                    name: "swap".to_string(),
                    mount: "swap".to_string(),
                    fstype: Some("swap".to_string()),
                    total: mem.swap_total,
                    used: mem.swap_used,
                    free: mem.swap_free,
                    used_percent: mem.swap_percent,
                    free_percent: mem.swap_free_percent,
                    ..DiskUsage::default()
                });
                continue;
            }
            let Some(disk) = self.disks.get(&mount) else {
                continue;
            };
            disks.push(DiskUsage {
                name: disk.name.clone(),
                mount,
                dev: Some(disk.dev.clone()),
                fstype: Some(disk.fstype.clone()),
                used: disk.used,
                total: disk.total,
                free: disk.free,
                used_percent: disk.used_percent,
                free_percent: disk.free_percent,
                io_read: disk.io_read,
                io_write: disk.io_write,
                io_activity: disk.io_activity,
            });
        }
        disks
    }

    /// The IO block of `Mem::collect`: sector and io_ticks deltas per disk.
    fn disk_io(&mut self, proc_root: &Path, config: &Config) {
        let uptime = uptime_secs(proc_root);
        let elapsed = uptime - self.old_uptime;
        for disk in self.disks.values_mut() {
            let Some(stat) = disk.stat.clone() else {
                continue;
            };
            if disk.fstype == "zfs" {
                if config.zfs_hide_datasets {
                    zfs_pool_io(&stat, disk);
                } else {
                    zfs_objset_io(&stat, disk);
                }
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&stat) else {
                continue;
            };
            let Some((sectors_read, sectors_write, io_ticks)) = block_stat(&text) else {
                continue;
            };
            if disk.io_read.is_none() {
                disk.io_read = Some(0);
                disk.io_write = Some(0);
                disk.io_activity = Some(0.0);
            } else {
                disk.io_read = Some((sectors_read - disk.old_io[0]).max(0) as u64 * 512);
                disk.io_write = Some((sectors_write - disk.old_io[1]).max(0) as u64 * 512);
                let activity = ((io_ticks - disk.old_io[2]) as f64 / elapsed / 10.0)
                    .round()
                    .clamp(0.0, 100.0);
                // Two reads of one uptime (a recorded tree) give 0/0: unavailable.
                disk.io_activity = Some(activity).filter(|activity| activity.is_finite());
            }
            disk.old_io = [sectors_read, sectors_write, io_ticks];
        }
        self.old_uptime = uptime;
    }

    /// btop re-reads `/etc/fstab` only when its mtime changes. A missing or
    /// unreadable fstab skips the disk section for that sample.
    fn load_fstab(&mut self, path: &Path) -> bool {
        let mtime = std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok();
        let Some(mtime) = mtime else {
            return false;
        };
        if self.fstab_mtime == Some(mtime) {
            return true;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        self.fstab_mtime = Some(mtime);
        self.fstab.clear();
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let Some(device) = fields.next() else {
                continue;
            };
            if device.starts_with('#') {
                continue;
            }
            let Some(mount) = fields.next() else {
                continue;
            };
            if mount != "none" && mount != "swap" {
                self.fstab.push(mount.to_string());
            }
        }
        true
    }
}

/// Container mode: memory and swap from the cgroup (see the module doc). No
/// cgroup v2 directory leaves every value unavailable.
fn container_memory(
    cgroup: Option<&Path>,
    meminfo: &BTreeMap<String, u64>,
    mem_total: u64,
    config: &Config,
) -> MemSnapshot {
    let swap_enabled = config.show_swap || config.swap_disk;
    let mut snapshot = MemSnapshot {
        swap_enabled,
        ..MemSnapshot::default()
    };
    let Some(cgroup) = cgroup else {
        return snapshot;
    };
    // `max` and a missing file both read as no number.
    let read = |name: &str| -> Option<u64> {
        std::fs::read_to_string(cgroup.join(name))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    let memory_stat: BTreeMap<String, u64> = std::fs::read_to_string(cgroup.join("memory.stat"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(' ')?;
            Some((key.to_string(), value.trim().parse().ok()?))
        })
        .collect();
    // A limit at or above MemTotal binds nothing, so only a lower memory.max
    // replaces the bar's total.
    let limit = read("memory.max").filter(|limit| *limit < mem_total);
    snapshot.ram_limit = limit;
    let bar = read("memory.current")
        .zip(memory_stat.get("inactive_file").copied())
        .and_then(|(current, inactive_file)| {
            let used = current.saturating_sub(inactive_file);
            if let Some(limit) = limit {
                let ([used, cache], free) = split(limit, [used, inactive_file]);
                return Some((limit, used, cache, free, None, free + cache));
            }
            let mem_free = meminfo.get("MemFree").copied()?;
            let mem_available = meminfo.get("MemAvailable").copied()?;
            let stat = |name: &str| memory_stat.get(name).copied().unwrap_or(0);
            let other_cache = mem_available
                .saturating_sub(mem_free)
                .saturating_sub(inactive_file + stat("active_file") + stat("slab_reclaimable"));
            let ([used, cache, free, other_cache], other) =
                split(mem_total, [used, inactive_file, mem_free, other_cache]);
            let others = Some((other, other_cache));
            Some((mem_total, used, cache, free, others, mem_available))
        });
    if let Some((total, used, cache, free, others, available)) = bar {
        snapshot.ram_total = Some(total);
        snapshot.ram_used = Some(used);
        snapshot.ram_cache = Some(cache);
        snapshot.ram_other = others.map(|(other, _)| other);
        snapshot.ram_other_cache = others.map(|(_, other_cache)| other_cache);
        snapshot.ram_free = Some(free);
        snapshot.ram_available = Some(available);
        snapshot.ram_percent = Some(percent(used, total));
        snapshot.ram_cache_percent = Some(percent(cache, total));
        snapshot.ram_other_percent = snapshot.ram_other.map(|other| percent(other, total));
        snapshot.ram_other_cache_percent = snapshot
            .ram_other_cache
            .map(|other_cache| percent(other_cache, total));
        snapshot.ram_free_percent = Some(percent(free, total));
        snapshot.ram_available_percent = Some(percent(available, total));
    }
    if swap_enabled
        && let (Some(used), Some(swap_total)) = (
            read("memory.swap.current"),
            meminfo.get("SwapTotal").copied(),
        )
    {
        let total = read("memory.swap.max").map_or(swap_total, |limit| limit.min(swap_total));
        let free = total.saturating_sub(used);
        snapshot.has_swap = config.show_swap && total > 0;
        snapshot.swap_total = Some(total);
        snapshot.swap_used = Some(used);
        snapshot.swap_free = Some(free);
        if snapshot.has_swap {
            snapshot.swap_percent = Some(percent(used, total));
            snapshot.swap_free_percent = Some(percent(free, total));
        }
    }
    snapshot
}

/// The Memory bar's parts in order, each clamped to what the earlier ones
/// left of `total`, and the remainder: the parts plus it sum to `total`, also
/// when reads taken at different instants overshoot.
fn split<const N: usize>(total: u64, parts: [u64; N]) -> ([u64; N], u64) {
    let mut left = total;
    let parts = parts.map(|part| {
        let part = part.min(left);
        left -= part;
        part
    });
    (parts, left)
}

/// btop: `round(stats * 100 / total)`.
fn percent(value: u64, total: u64) -> f64 {
    (value as f64 * 100.0 / total as f64).round()
}

fn parse_meminfo(text: &str) -> BTreeMap<String, u64> {
    let mut stats = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let Some(value) = value.split_whitespace().next() else {
            continue;
        };
        if let Ok(kilobytes) = value.parse::<u64>() {
            stats.insert(key.trim().to_string(), kilobytes * 1024);
        }
    }
    stats
}

fn zfs_arc(proc_root: &Path) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(proc_root.join("spl/kstat/zfs/arcstats")).ok()?;
    let mut size = None;
    let mut minimum = 0;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else { continue };
        let _kind = fields.next();
        let Some(value) = fields.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        match name {
            "size" => size = Some(value),
            "c_min" => minimum = value,
            _ => {}
        }
    }
    size.map(|size| (size, minimum))
}

/// `/proc` maps to `/`; a recorded `<tree>/proc` to `<tree>`.
fn tree_root(proc_root: &Path) -> &Path {
    match proc_root.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("/"),
    }
}

/// `/proc` maps to `/etc`; a recorded `<tree>/proc` to `<tree>/etc`.
fn etc_root(proc_root: &Path) -> PathBuf {
    tree_root(proc_root).join("etc")
}

fn uptime_secs(proc_root: &Path) -> f64 {
    std::fs::read_to_string(proc_root.join("uptime"))
        .ok()
        .and_then(|text| {
            text.split_whitespace()
                .next()
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(0.0)
}

/// btop's `fs::path(mountpoint).filename()`, with its fallbacks: `root` for
/// `/`, and the full mountpoint when the filename is empty.
fn disk_name(mount: &str) -> String {
    if mount == "/" {
        return "root".to_string();
    }
    if mount.ends_with('/') {
        // C++ std::filesystem::path("/mnt/").filename() is empty.
        return mount.to_string();
    }
    Path::new(mount)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| mount.to_string())
}

/// btop's `disks_filter`: whitespace-separated mountpoints, `exclude=` first
/// to invert the match.
struct Filter {
    exclude: bool,
    mounts: Vec<String>,
}

impl Filter {
    fn parse(value: &str) -> Option<Self> {
        let mut mounts: Vec<String> = value.split_whitespace().map(str::to_string).collect();
        let exclude = mounts
            .first()
            .is_some_and(|first| first.starts_with("exclude="));
        if exclude {
            let first = mounts.remove(0);
            let rest = &first["exclude=".len()..];
            if !rest.is_empty() {
                mounts.insert(0, rest.to_string());
            }
        }
        (!mounts.is_empty() || exclude).then_some(Self { exclude, mounts })
    }

    fn allows(&self, mount: &str) -> bool {
        self.mounts.iter().any(|candidate| candidate == mount) != self.exclude
    }
}

/// btop shrinks the device name until `/sys/block/<name>/stat` exists, then
/// checks the partition subdirectory.
fn block_stat_path(
    proc_root: &Path,
    device: &str,
    fstype: &str,
    dataset_name_start: Option<usize>,
    config: &Config,
) -> Option<PathBuf> {
    let sys_block = sys_root(proc_root).join("block");
    let devname = Path::new(device)
        .file_name()?
        .to_string_lossy()
        .into_owned();
    let mut name = devname.clone();
    let mut trimmed = 0;
    while name.len() >= 2 {
        let stat = sys_block.join(&name).join("stat");
        if stat.is_file() {
            if trimmed > 0 {
                let partition = sys_block.join(&name).join(&devname).join("stat");
                if partition.is_file() {
                    return Some(partition);
                }
            }
            return Some(stat);
        }
        if fstype == "zfs" {
            return zfs_stat_file(
                proc_root,
                device,
                dataset_name_start,
                config.zfs_hide_datasets,
            );
        }
        name.pop();
        trimmed += 1;
    }
    None
}

fn zfs_stat_file(
    proc_root: &Path,
    device: &str,
    dataset_name_start: Option<usize>,
    hide_datasets: bool,
) -> Option<PathBuf> {
    let root = proc_root.join("spl/kstat/zfs");
    if hide_datasets {
        let pool = root.join(device);
        return pool.is_dir().then_some(pool);
    }
    let pool = match dataset_name_start {
        Some(start) => &device[..start],
        None => device,
    };
    let entries = std::fs::read_dir(root.join(pool)).ok()?;
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("objset") {
            continue;
        }
        if objset_device(&entry.path()).as_deref() == Some(device) {
            return Some(entry.path());
        }
    }
    None
}

/// The type-7 name field of an objset kstat file: btop skips two lines, then
/// scans for `7` and reads the next token.
fn objset_device(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines().skip(2) {
        if let Some(position) = line.find('7') {
            return line[position + 1..]
                .split_whitespace()
                .next()
                .map(str::to_string);
        }
    }
    None
}

/// The four type-4 values of an objset kstat file, in btop's read order:
/// io_ticks, bytes written, io_ticks again, bytes read.
fn objset_values(path: &Path) -> Option<[i64; 4]> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut values = Vec::with_capacity(4);
    for line in text.lines().skip(3) {
        let mut fields = line.split_whitespace();
        let _name = fields.next();
        if fields.next() != Some("4") {
            continue;
        }
        let Some(value) = fields.next().and_then(|value| value.parse::<i64>().ok()) else {
            continue;
        };
        values.push(value);
        if values.len() == 4 {
            return Some([values[0], values[1], values[2], values[3]]);
        }
    }
    None
}

fn zfs_objset_io(path: &Path, disk: &mut DiskState) {
    let Some(values) = objset_values(path) else {
        return;
    };
    let ticks = values[0] + values[2];
    let write = values[1];
    let read = values[3];
    if disk.io_read.is_none() {
        disk.io_read = Some(0);
        disk.io_write = Some(0);
        disk.io_activity = Some(0.0);
    } else {
        disk.io_read = Some((read - disk.old_io[0]).max(0) as u64);
        disk.io_write = Some((write - disk.old_io[1]).max(0) as u64);
        disk.io_activity = Some((ticks - disk.old_io[2]).max(0) as f64);
    }
    disk.old_io = [read, write, ticks];
}

fn zfs_pool_io(dir: &Path, disk: &mut DiskState) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let (mut read, mut write, mut ticks, mut objects) = (0_i64, 0_i64, 0_i64, 0_i64);
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("objset") {
            continue;
        }
        let Some(values) = objset_values(&entry.path()) else {
            continue;
        };
        ticks += values[0] + values[2];
        write += values[1];
        read += values[3];
        objects += 1;
    }
    if objects == 0 {
        return;
    }
    if disk.io_read.is_none() {
        disk.io_read = Some(0);
        disk.io_write = Some(0);
        disk.io_activity = Some(0.0);
    } else {
        disk.io_read = Some((read - disk.old_io[0]).max(0) as u64);
        disk.io_write = Some((write - disk.old_io[1]).max(0) as u64);
        disk.io_activity = Some((ticks - disk.old_io[2]).max(0) as f64);
    }
    disk.old_io = [read, write, ticks];
}

/// Fields 3, 7 and 10 of `/sys/block/<dev>/stat`: sectors read, sectors
/// written and milliseconds spent doing IO.
fn block_stat(text: &str) -> Option<(i64, i64, i64)> {
    let values: Vec<i64> = text
        .split_whitespace()
        .filter_map(|value| value.parse().ok())
        .take(10)
        .collect();
    if values.len() < 10 {
        return None;
    }
    Some((values[2], values[6], values[9]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative)
    }

    fn fixture_config() -> Config {
        let mut config = Config::load(None).expect("embedded defaults");
        config.proc_root = fixture("proc");
        config
    }

    fn empty_config(proc_root: &Path) -> Config {
        let mut config = Config::load(None).expect("embedded defaults");
        config.proc_root = proc_root.to_path_buf();
        config
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
        std::fs::write(path, text).expect("write fixture");
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("btop-web-mem-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proc/self")).expect("create tree");
        root
    }

    fn mounts(disks: &[DiskUsage]) -> Vec<&str> {
        disks.iter().map(|disk| disk.mount.as_str()).collect()
    }

    #[test]
    fn percent_matches_btops_rounding() {
        assert_eq!(percent(1, 3), 33.0);
        assert_eq!(percent(2, 3), 67.0);
    }

    #[test]
    fn escaped_mountpoint_is_passed_to_statvfs_raw() {
        let root = temp_root("mount-escape");
        write(
            &root.join("proc/meminfo"),
            "MemTotal:       1000 kB\nMemFree:         500 kB\nCached:          100 kB\nSwapTotal:         0 kB\nSwapFree:          0 kB\n",
        );
        // btop's stream extraction keeps the token as written, so only a literal
        // directory name containing the four characters resolves.
        std::fs::create_dir_all(root.join("disk\\040name")).expect("create mountpoint");
        write(
            &root.join("proc/self/mounts"),
            "/dev/sdd /disk\\040name ext4 rw 0 0\n",
        );

        let mut config = empty_config(&root.join("proc"));
        config.use_fstab = false;
        config.only_physical = false;
        let mut collector = MemCollector::default();
        let snapshot = collector.collect(&root.join("proc"), &config);

        assert_eq!(mounts(&snapshot.disks), ["/disk\\040name"]);
        let _ = std::fs::remove_dir_all(root);
    }

    /// ACC-GRAPH-47-GAUGE-SMOOTH: the arc draws the used history's EMA; the
    /// readout keeps btop's raw percentage.
    #[test]
    fn gauge_pct_is_the_smoothed_used_percent() {
        let config = fixture_config();
        let mut collector = MemCollector::default();
        collector.used_history.push(Some(0.0), config.update_ms);
        let snapshot = collector.collect(&fixture("proc"), &config);
        let raw = snapshot.ram_percent.expect("fixture used percent");
        let alpha = 1.0 - (-(config.update_ms as f64) / 1000.0).exp();
        let arc = snapshot.gauge_pct.expect("smoothed arc");
        assert!((arc - alpha * raw).abs() < 1e-9, "{arc} vs {raw}");
        let unseeded = MemCollector::default().collect(&fixture("proc"), &config);
        assert_eq!(unseeded.ram_percent, Some(raw));
        assert_eq!(unseeded.gauge_pct, Some(raw));
    }

    #[test]
    fn memory_percentages_and_swap_fields_follow_btop() {
        let mut collector = MemCollector::default();
        let snapshot = collector.collect(&fixture("proc"), &fixture_config());

        assert_eq!(snapshot.ram_total, Some(527_649_328 * 1024));
        assert_eq!(snapshot.ram_free_percent, Some(33.0));
        assert_eq!(snapshot.ram_available_percent, Some(93.0));
        let total = 527_649_328u64 * 1024;
        let available = 490_578_592u64 * 1024 + 1_048_576 - 262_144;
        assert_eq!(
            snapshot.ram_percent,
            Some(percent(total - available, total))
        );
        // DEF-VIEW-50-MEM-TERMS: used | cache | free = MemTotal, cache =
        // MemAvailable - MemFree, no other segment on a host.
        let free = 172_308_644u64 * 1024;
        assert_eq!(snapshot.ram_free, Some(free));
        assert_eq!(snapshot.ram_cache, Some(available - free));
        assert_eq!(snapshot.ram_other, None);
        assert_eq!(bar_sum(&snapshot), total);
        assert_eq!(snapshot.ram_cache_percent, Some(60.0));
        assert!(snapshot.has_swap);
        assert_eq!(snapshot.swap_free, Some(133_798_148 * 1024));
        assert_eq!(snapshot.swap_percent, Some(0.0));
        assert_eq!(snapshot.swap_free_percent, Some(100.0));

        // One sample is not a curve; the next tick draws the used history.
        let again = collector.collect(&fixture("proc"), &fixture_config());
        assert!(
            again.used_chart.line.starts_with('M'),
            "{}",
            again.used_chart.line
        );
    }

    /// The used, cache, other used, other reclaimable and free bytes and their
    /// bar widths both add up to the total.
    fn bar_sum(snapshot: &MemSnapshot) -> u64 {
        let width: f64 = snapshot.ram_segments().iter().map(|(_, width)| width).sum();
        assert!((width - 100.0).abs() < 1e-9, "widths sum to {width}");
        [
            snapshot.ram_used,
            snapshot.ram_cache,
            snapshot.ram_other.or(Some(0)),
            snapshot.ram_other_cache.or(Some(0)),
            snapshot.ram_free,
        ]
        .into_iter()
        .sum::<Option<u64>>()
        .expect("every segment")
    }

    /// MemAvailable below MemFree (the kernel reserve above the reclaimable
    /// memory): free is clamped to what used leaves, and cache is 0.
    #[test]
    fn memory_bar_clamps_free_when_available_is_below_memfree() {
        let root = temp_root("bar-clamp");
        write(
            &root.join("proc/meminfo"),
            "MemTotal:       1000 kB\nMemFree:         600 kB\nMemAvailable:    500 kB\nCached:          100 kB\n",
        );
        let snapshot =
            MemCollector::default().collect(&root.join("proc"), &empty_config(&root.join("proc")));
        assert_eq!(snapshot.ram_used, Some(500 * 1024));
        assert_eq!(snapshot.ram_free, Some(500 * 1024));
        assert_eq!(snapshot.ram_cache, Some(0));
        assert_eq!(bar_sum(&snapshot), 1000 * 1024);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn zfs_arc_is_added_only_when_enabled() {
        let expected = 1_048_576;
        let minimum = 262_144;

        let mut collector = MemCollector::default();
        let with_arc = collector.collect(&fixture("proc"), &fixture_config());
        assert_eq!(
            with_arc.ram_available,
            Some(490_578_592 * 1024 + expected - minimum)
        );

        let mut config = fixture_config();
        config.zfs_arc_cached = false;
        let mut collector = MemCollector::default();
        let without_arc = collector.collect(&fixture("proc"), &config);
        assert_eq!(without_arc.ram_available, Some(490_578_592 * 1024));
    }

    #[test]
    fn use_fstab_selects_only_fstab_mountpoints() {
        let mut collector = MemCollector::default();
        let snapshot = collector.collect(&fixture("proc"), &fixture_config());

        // Root first, then the swap pseudo-disk, then fstab order.
        assert_eq!(mounts(&snapshot.disks), ["/", "swap", "/home"]);
        let root = &snapshot.disks[0];
        assert_eq!(root.name, "root");
        assert_eq!(root.fstype.as_deref(), Some("overlay"));
        assert!(root.used_percent.is_some());
        assert_eq!(
            root.free_percent,
            root.used_percent.map(|percent| 100.0 - percent)
        );

        let home = &snapshot.disks[2];
        assert_eq!(home.name, "home");
        assert_eq!(home.fstype.as_deref(), Some("ext4"));
        assert_eq!(home.io_read, Some(0));
        assert_eq!(home.io_write, Some(0));
        assert_eq!(home.io_activity, Some(0.0));

        let swap = &snapshot.disks[1];
        assert_eq!(swap.total, snapshot.swap_total);
        assert_eq!(swap.used, snapshot.swap_used);
        assert_eq!(swap.free, snapshot.swap_free);
        assert_eq!(swap.used_percent, snapshot.swap_percent);
        assert_eq!(swap.free_percent, snapshot.swap_free_percent);
    }

    #[test]
    fn only_physical_filters_by_proc_filesystems() {
        let mut config = fixture_config();
        config.use_fstab = false;
        config.only_physical = true;
        let mut collector = MemCollector::default();
        let snapshot = collector.collect(&fixture("proc"), &config);

        // No root: the recorded root is overlay, which /proc/filesystems marks nodev.
        assert!(!mounts(&snapshot.disks).contains(&"/"));
        assert!(mounts(&snapshot.disks).contains(&"/home"));
        assert!(mounts(&snapshot.disks).contains(&"/gpu-bookings"));
        // NFS is nodev as well.
        assert!(!mounts(&snapshot.disks).contains(&"/mnt/nas-shared"));
        assert_eq!(snapshot.disks[0].mount, "swap");
    }

    #[test]
    fn missing_fstab_leaves_disks_unavailable() {
        let root = temp_root("missing-fstab");
        write(
            &root.join("proc/meminfo"),
            "MemTotal:       1000 kB\nMemFree:         500 kB\nCached:          100 kB\nSwapTotal:         0 kB\nSwapFree:          0 kB\n",
        );
        write(&root.join("proc/self/mounts"), "/dev/sdd / ext4 rw 0 0\n");

        let mut collector = MemCollector::default();
        let snapshot = collector.collect(&root.join("proc"), &empty_config(&root.join("proc")));
        assert!(snapshot.disks.is_empty());
        assert_eq!(snapshot.ram_total, Some(1000 * 1024));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mtab_is_preferred_over_proc_self_mounts() {
        let root = temp_root("mtab");
        write(
            &root.join("proc/meminfo"),
            "MemTotal:       1000 kB\nMemFree:         500 kB\nCached:          100 kB\nSwapTotal:         0 kB\nSwapFree:          0 kB\n",
        );
        write(&root.join("proc/self/mounts"), "/dev/sdd / ext4 rw 0 0\n");
        write(&root.join("etc/mtab"), "/dev/sdd /home ext4 rw 0 0\n");
        std::fs::create_dir_all(root.join("home")).expect("create mountpoint");

        let mut config = empty_config(&root.join("proc"));
        config.use_fstab = false;
        config.only_physical = false;
        let mut collector = MemCollector::default();
        let snapshot = collector.collect(&root.join("proc"), &config);
        assert_eq!(mounts(&snapshot.disks), ["/home"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_statvfs_mount_is_ignored_for_the_process_lifetime() {
        let root = temp_root("statvfs");
        write(
            &root.join("proc/meminfo"),
            "MemTotal:       1000 kB\nMemFree:         500 kB\nCached:          100 kB\nSwapTotal:         0 kB\nSwapFree:          0 kB\n",
        );
        write(
            &root.join("proc/self/mounts"),
            "/dev/sdd /btop-web-does-not-exist ext4 rw 0 0\n",
        );

        let mut config = empty_config(&root.join("proc"));
        config.use_fstab = false;
        config.only_physical = false;
        let mut collector = MemCollector::default();
        assert!(
            collector
                .collect(&root.join("proc"), &config)
                .disks
                .is_empty()
        );
        assert!(
            collector
                .ignore_list
                .contains(&"/btop-web-does-not-exist".to_string())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn block_io_deltas_and_activity_follow_btop() {
        let root = temp_root("block-io");
        write(
            &root.join("proc/meminfo"),
            "MemTotal:       1000 kB\nMemFree:         500 kB\nCached:          100 kB\nSwapTotal:         0 kB\nSwapFree:          0 kB\n",
        );
        write(&root.join("proc/uptime"), "100.00 0.00\n");
        write(&root.join("proc/self/mounts"), "/dev/sdd / ext4 rw 0 0\n");
        write(
            &root.join("sys/block/sdd/stat"),
            "100 0 200 0 50 0 300 0 0 100 0\n",
        );

        let mut config = empty_config(&root.join("proc"));
        config.use_fstab = false;
        config.only_physical = false;
        let mut collector = MemCollector::default();

        let first = collector.collect(&root.join("proc"), &config);
        assert_eq!(first.disks[0].io_read, Some(0));
        assert_eq!(first.disks[0].io_write, Some(0));
        assert_eq!(first.disks[0].io_activity, Some(0.0));

        write(
            &root.join("sys/block/sdd/stat"),
            "110 0 208 0 55 0 316 0 0 150 0\n",
        );
        write(&root.join("proc/uptime"), "102.00 0.00\n");
        let second = collector.collect(&root.join("proc"), &config);
        assert_eq!(second.disks[0].io_read, Some(8 * 512));
        assert_eq!(second.disks[0].io_write, Some(16 * 512));
        assert_eq!(second.disks[0].io_activity, Some(3.0));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn disk_name_matches_btops_fallbacks() {
        assert_eq!(disk_name("/"), "root");
        assert_eq!(disk_name("/home"), "home");
        assert_eq!(disk_name("/mnt/"), "/mnt/");
    }

    #[test]
    fn zfs_objset_values_and_dataset_name_are_parsed() {
        let root = temp_root("zfs");
        let objset = root.join("proc/spl/kstat/zfs/tank/objset-0x1");
        write(
            &objset,
            "13 1 0x01 36 1 0 0\nname type data\ntank/data 7 tank/data\nwrites 4 100\nnwritten 4 4096\nreads 4 50\nnread 4 8192\n",
        );

        assert_eq!(objset_values(&objset), Some([100, 4096, 50, 8192]));
        assert_eq!(objset_device(&objset).as_deref(), Some("tank/data"));
        assert_eq!(
            zfs_stat_file(&root.join("proc"), "tank/data", Some(4), false),
            Some(objset)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn disks_filter_includes_or_excludes_mountpoints() {
        let filter = Filter::parse("exclude=/boot /home").expect("filter");
        assert!(!filter.allows("/boot"));
        assert!(!filter.allows("/home"));
        assert!(filter.allows("/"));

        let filter = Filter::parse("/boot /home").expect("filter");
        assert!(filter.allows("/boot"));
        assert!(!filter.allows("/"));

        assert!(Filter::parse("").is_none());
    }

    fn in_container(cgroup: &Path) -> Config {
        let mut config = Config::load(None).expect("embedded defaults");
        config.container = Some(crate::collector::container::Container {
            runtime: String::from("docker"),
            signal: String::from("1 /.dockerenv"),
            cgroup: Some(cgroup.to_path_buf()),
            own_netns: true,
        });
        config
    }

    /// A copy of the recorded container cgroup with `file` replaced, or
    /// removed when `text` is `None`.
    fn cgroup_variant(name: &str, file: &str, text: Option<&str>) -> PathBuf {
        let root = temp_root(name);
        let cgroup = root.join("cgroup");
        std::fs::create_dir_all(&cgroup).expect("cgroup dir");
        for entry in std::fs::read_dir(fixture("container/sys/fs/cgroup")).expect("fixture") {
            let entry = entry.expect("entry");
            if entry.path().is_file() {
                std::fs::copy(entry.path(), cgroup.join(entry.file_name())).expect("copy");
            }
        }
        match text {
            Some(text) => write(&cgroup.join(file), text),
            None => std::fs::remove_file(cgroup.join(file)).expect("remove"),
        }
        cgroup
    }

    fn container_snapshot(cgroup: &Path) -> MemSnapshot {
        MemCollector::default().collect(&fixture("container/proc"), &in_container(cgroup))
    }

    const GIB: u64 = 1 << 30;

    /// DEF-VIEW-96-MEM-WRONG: used is docker's working set,
    /// memory.current - inactive_file, and cache is inactive_file;
    /// active_file and slab_reclaimable stay part of the container's use. The
    /// recorded memory.max (8 GiB) is below MemTotal, so it binds: the total
    /// is the limit, free is the room under it, and other is dropped.
    #[test]
    fn container_memory_reads_dockers_working_set() {
        let snapshot = container_snapshot(&fixture("container/sys/fs/cgroup"));
        assert_eq!(snapshot.ram_total, Some(8 * GIB));
        assert_eq!(snapshot.ram_limit, Some(8 * GIB));
        // 4 GiB memory.current - 1 GiB inactive_file; the 1 GiB active_file
        // and 256 MiB slab_reclaimable are not subtracted.
        assert_eq!(snapshot.ram_used, Some(3 * GIB));
        assert_eq!(snapshot.ram_cache, Some(GIB));
        // 8 GiB memory.max - 4 GiB memory.current, the room under the limit.
        assert_eq!(snapshot.ram_free, Some(4 * GIB));
        assert_eq!(snapshot.ram_other, None);
        assert_eq!(snapshot.ram_other_cache, None);
        assert_eq!(snapshot.ram_available, Some(5 * GIB));
        assert_eq!(bar_sum(&snapshot), 8 * GIB);
        assert_eq!(snapshot.ram_percent, Some(38.0));
        assert_eq!(snapshot.ram_cache_percent, Some(13.0));
        assert_eq!(snapshot.ram_other_percent, None);
        assert_eq!(snapshot.ram_free_percent, Some(50.0));
        assert_eq!(snapshot.ram_available_percent, Some(63.0));
    }

    /// DEF-VIEW-101-MEM-BREAKDOWN, recorded /proc/meminfo and cgroup: the five
    /// rows each read the source the row names and sum to MemTotal exactly,
    /// available is MemAvailable, and the gauge draws the first two rows' shares
    /// alone, the rest being the bare track (ACC-VIEW-192-GAUGE-CONTAINER).
    #[test]
    fn container_rows_sum_to_mem_total() {
        let (total, free, available): (u64, u64, u64) =
            (527_649_328 * 1024, 172_308_644 * 1024, 490_578_592 * 1024);
        let (current, inactive_file) = (4 * GIB, GIB);
        // The recorded active_file (1 GiB) and slab_reclaimable (256 MiB) of
        // this container, which MemAvailable counts as reclaimable.
        let container_reclaimable = inactive_file + GIB + (256 << 20);
        let cgroup = cgroup_variant("cg-rows", "memory.max", Some("max\n"));
        let snapshot = container_snapshot(&cgroup);
        let other_cache = available - free - container_reclaimable;
        let other = total - (current - inactive_file) - inactive_file - other_cache - free;
        assert_eq!(snapshot.ram_used, Some(current - inactive_file));
        assert_eq!(snapshot.ram_cache, Some(inactive_file));
        assert_eq!(snapshot.ram_other_cache, Some(other_cache));
        assert_eq!(snapshot.ram_other, Some(other));
        assert_eq!(snapshot.ram_free, Some(free));
        assert_eq!(snapshot.ram_available, Some(available));
        assert_eq!(snapshot.ram_available_percent, Some(93.0));
        assert_eq!(bar_sum(&snapshot), total);
        let share = |value: u64| value as f64 * 100.0 / total as f64;
        let expected = [
            ("used", share(current - inactive_file)),
            ("cache", share(inactive_file)),
        ];
        assert_eq!(snapshot.gauge_segments.len(), expected.len());
        for ((name, drawn), (want, value)) in snapshot.gauge_segments.iter().zip(expected) {
            assert_eq!(*name, want);
            assert!((drawn - value).abs() < 1e-9, "{name}: {drawn} vs {value}");
        }
        let _ = std::fs::remove_dir_all(cgroup.parent().expect("root"));
    }

    /// A memory.current read ahead of MemFree overshoots MemTotal: the
    /// clamps in [`split`] still sum to it, and other used is the row that
    /// states the difference.
    #[test]
    fn container_rows_clamp_to_mem_total_when_reads_overshoot() {
        let cgroup = cgroup_variant("cg-over", "memory.max", Some("max\n"));
        write(&cgroup.join("memory.current"), "600000000000\n");
        let snapshot = container_snapshot(&cgroup);
        assert_eq!(snapshot.ram_other, Some(0));
        assert_eq!(bar_sum(&snapshot), 527_649_328 * 1024);
        let _ = std::fs::remove_dir_all(cgroup.parent().expect("root"));
    }

    /// ACC-GRAPH-47-GAUGE-SMOOTH, ACC-VIEW-192-GAUGE-CONTAINER: the container's
    /// two segments draw, and each moves by the 1 s average.
    #[test]
    fn gauge_segments_draw_the_smoothed_shares_of_the_container_under_a_limit() {
        let config = in_container(&fixture("container/sys/fs/cgroup"));
        let mut collector = MemCollector {
            segment_ema: vec![0.0; 2],
            ..MemCollector::default()
        };
        let snapshot = collector.collect(&fixture("container/proc"), &config);
        let alpha = 1.0 - (-(config.update_ms as f64) / 1000.0).exp();
        assert_eq!(snapshot.gauge_segments.len(), 2);
        let (name, used) = snapshot.gauge_segments[0];
        assert_eq!(name, "used");
        assert!((used - alpha * 37.5).abs() < 1e-9, "{used}");
        let (name, cache) = snapshot.gauge_segments[1];
        assert_eq!(name, "cache");
        assert!((cache - alpha * 12.5).abs() < 1e-9, "{cache}");
    }

    /// DEF-VIEW-95-SCOPE-MIX: with memory.max `max` or above MemTotal the
    /// header total and free are the host's and the other rows are the host's
    /// memory outside this container; used and cache stay this container's.
    #[test]
    fn container_memory_without_a_limit_reads_the_host_total_and_free() {
        let total = 527_649_328 * 1024;
        let free = 172_308_644 * 1024;
        let available = 490_578_592 * 1024;
        for (name, limit) in [("cg-max", "max\n"), ("cg-above", "1000000000000000\n")] {
            let cgroup = cgroup_variant(name, "memory.max", Some(limit));
            let snapshot = container_snapshot(&cgroup);
            assert_eq!(snapshot.ram_total, Some(total));
            assert_eq!(snapshot.ram_limit, None);
            assert_eq!(snapshot.ram_used, Some(3 * GIB));
            assert_eq!(snapshot.ram_cache, Some(GIB));
            assert_eq!(snapshot.ram_free, Some(free));
            let other_cache = available - free - GIB - GIB - (256 << 20);
            assert_eq!(snapshot.ram_other_cache, Some(other_cache));
            assert_eq!(
                snapshot.ram_other,
                Some(total - 4 * GIB - free - other_cache)
            );
            assert_eq!(snapshot.ram_available, Some(available));
            assert_eq!(bar_sum(&snapshot), total);
            assert_eq!(snapshot.ram_percent, Some(1.0));
            assert_eq!(snapshot.ram_other_percent, Some(7.0));
            assert_eq!(snapshot.ram_other_cache_percent, Some(60.0));
            let _ = std::fs::remove_dir_all(cgroup.parent().expect("root"));
        }
    }

    /// ACC-SCOPE-64-CONTAINER: a container with no readable cgroup renders
    /// every memory value unavailable, never the host's.
    #[test]
    fn container_memory_without_a_cgroup_is_unavailable() {
        for cgroup in [None, Some(PathBuf::from("/btop-web-no-such-cgroup"))] {
            let mut config = in_container(Path::new("/btop-web-no-cgroup"));
            config.container.as_mut().expect("container").cgroup = cgroup;
            let snapshot = MemCollector::default().collect(&fixture("container/proc"), &config);
            assert_eq!(snapshot.ram_total, None);
            assert_eq!(snapshot.ram_used, None);
            assert_eq!(snapshot.ram_cache, None);
            assert_eq!(snapshot.ram_free, None);
            assert_eq!(snapshot.ram_other, None);
            assert_eq!(snapshot.ram_available, None);
            assert_eq!(snapshot.ram_limit, None);
        }
    }

    #[test]
    fn container_swap_follows_the_cgroup() {
        let snapshot = container_snapshot(&fixture("container/sys/fs/cgroup"));
        // memory.swap.max is max: the total is SwapTotal.
        assert_eq!(snapshot.swap_used, Some(512 << 20));
        assert_eq!(snapshot.swap_total, Some(137_438_953_472));
        assert_eq!(snapshot.swap_free, Some(136_902_082_560));
        assert_eq!(snapshot.swap_percent, Some(0.0));
        assert_eq!(snapshot.swap_free_percent, Some(100.0));
        assert!(snapshot.has_swap);

        let cgroup = cgroup_variant("cg-swap", "memory.swap.max", Some("1073741824\n"));
        let limited = container_snapshot(&cgroup);
        assert_eq!(limited.swap_total, Some(GIB));
        assert_eq!(limited.swap_percent, Some(50.0));
        let _ = std::fs::remove_dir_all(cgroup.parent().expect("root"));
    }

    #[test]
    fn container_swap_without_swap_current_is_unavailable() {
        let cgroup = cgroup_variant("cg-noswap", "memory.swap.current", None);
        let snapshot = container_snapshot(&cgroup);
        assert_eq!(snapshot.swap_used, None);
        assert_eq!(snapshot.swap_total, None);
        assert!(
            snapshot.swap_visible(),
            "a swap key is on: unavailable, not hidden"
        );
        assert_eq!(snapshot.swap_label(), "unavailable");
        let _ = std::fs::remove_dir_all(cgroup.parent().expect("root"));
    }

    /// The block stat and uptime that drive btop's IO exist, and two samples
    /// still give no IO figure inside a container.
    #[test]
    fn container_disks_withhold_io() {
        let root = temp_root("container-io");
        write(
            &root.join("proc/meminfo"),
            "MemTotal:       1000 kB\nMemFree:         500 kB\nCached:          100 kB\nSwapTotal:         0 kB\nSwapFree:          0 kB\n",
        );
        write(&root.join("proc/uptime"), "100.00 0.00\n");
        write(&root.join("proc/self/mounts"), "/dev/sdd / ext4 rw 0 0\n");
        write(
            &root.join("sys/block/sdd/stat"),
            "100 0 200 0 50 0 300 0 0 100 0\n",
        );

        let mut config = in_container(&fixture("container/sys/fs/cgroup"));
        config.use_fstab = false;
        config.only_physical = false;
        let mut collector = MemCollector::default();
        collector.collect(&root.join("proc"), &config);
        write(
            &root.join("sys/block/sdd/stat"),
            "110 0 208 0 55 0 316 0 0 150 0\n",
        );
        write(&root.join("proc/uptime"), "102.00 0.00\n");
        let snapshot = collector.collect(&root.join("proc"), &config);
        let disk = &snapshot.disks[0];
        assert_eq!(disk.mount, "/");
        assert!(
            disk.total.is_some(),
            "capacity stays the container's df view"
        );
        assert_eq!(disk.io_read, None);
        assert_eq!(disk.io_write, None);
        assert_eq!(disk.io_activity, None);
        let _ = std::fs::remove_dir_all(root);
    }
}

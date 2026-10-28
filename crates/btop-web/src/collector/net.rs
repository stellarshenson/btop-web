//! Network collector, mirroring `Net::collect` in btop's
//! `src/linux/btop_collect.cpp`.
//!
//! Raw source: `getifaddrs(3)` for the interface list, the IFF_RUNNING flag and
//! the first IPv4 and IPv6 address of each interface (the MAC from
//! `<sys_root>/class/net/<iface>/address` when neither family has one), and
//! `<sys_root>/class/net/<iface>/statistics/{rx_bytes,tx_bytes}` for the
//! counters.
//!
//! Formula (btop): if the new counter is below the saved one, add the saved
//! value to the rollover and restart the delta at zero; speed =
//! round((counter - last) / ((new_timestamp - timestamp) / 1000)). The saved
//! timestamp starts at zero, so the first sample divides the since-boot
//! counter by the seconds since the Unix epoch. `top` is the highest speed
//! seen; `total` = counter + rollover - offset; `offset` resets when it would
//! exceed the counter plus rollover. The selected interface keeps its choice
//! while it exists, and is re-chosen when the saved one is empty or gone:
//! `net_iface` when it names a current interface, otherwise the interface with
//! the largest download + upload total.
//!
//! Departure (DEF-NET-42-LOOPBACK-PICK, owner 2026-10-27, "Skip lo"): btop's
//! largest total picks `lo` on a host whose proxy traffic runs over loopback,
//! which shows download equal to upload. Without a matching `net_iface`,
//! btop-web picks the largest total among the interfaces other than `lo`, a
//! connected one before a disconnected one (btop's loop means to prefer a
//! connected interface, but its `break` takes the largest total either way),
//! and `lo` only when no other interface exists. `net_iface` still forces any
//! interface, `lo` included.
//!
//! Auto scale (btop `graph_max`, `max_count` and `rescale`): with `net_auto`
//! the selected interface's last `n` bandwidth samples set the ceiling, their
//! mean times 1.3 on scale 0 and times 3.0 on scale 1, never below 10 KiB. A
//! direction counts samples above the ceiling (scale 0) and samples below a
//! tenth of it (scale 1), and rescales once a count reaches `n`; `rescale`
//! forces the first pass and every new selection. btop's `n` is 5, a count of
//! samples designed for its 2000 ms interval, where 5 samples span 10 s; at
//! 250 ms the same count spans 1.25 s and rescaled 28 times in 4 min of real
//! traffic (DEF-GRAPH-39-NET-JUMP). On the owner's decision (2026-10-27) the
//! count keeps btop's 10 s span at any interval: `n = max(5, 10000 /
//! update_ms)`, 5 at 2000 ms (btop parity), 40 at 250 ms (1 rescale in the
//! same 4 min).
//!
//! `net_sync` copies the rescaled ceiling to the other direction and skips
//! the slower direction's count; the download test compares against the
//! previous tick's upload speed (btop updates upload after it tests
//! download), the upload test against the current download speed. With
//! `net_auto` false the ceilings are the fixed `net_download` and
//! `net_upload` Mebibits as `(value << 20) / 8` bytes per second.
//!
//! Smooth scale (ACC-GRAPH-67-SMOOTH-SCALE, a btop-web addition): each
//! direction's ceiling feeds a [`Scale`], which moves the drawn ceiling and the
//! scale label to a new value over 1 s or more instead of in one frame; a new
//! selection starts at its own ceiling without a change. `graph_max_down` and
//! `graph_max_up` keep btop's values.
//!
//! Floor (ACC-GRAPH-71-SCALE-FLOOR, a btop-web addition, owner 2026-10-27):
//! with `net_auto` the shown ceiling never goes below 1 MiB/s, so idle traffic
//! draws low and calm instead of filling the box; btop's own floor is 10 KiB.
//! btop's ceiling still follows its rule and its floor, and a rescale that
//! would show less than 1 MiB/s stops there, through the same smooth change.
//!
//! Period: every sample, `update_ms` (250 ms by default; btop's 2000).
//!
//! Container (ACC-SCOPE-64-CONTAINER): `getifaddrs` and
//! `<sys_root>/class/net` answer for btop-web's own network namespace, so in a
//! container with its own namespace every value above is the container's,
//! from the same sources, formula and period. In a container that shares the
//! host's namespace (detected once at startup, see [`super::container`]) the
//! collector departs from btop on the owner's order (2026-10-27): it reads
//! nothing and returns an empty snapshot, which the page shows as `network
//! unavailable`.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nix::ifaddrs::getifaddrs;
use nix::net::if_::InterfaceFlags;

use super::{NetInterface, NetSnapshot, sys_root};
use crate::chart::{History, NET_PLOT, Scale};
use crate::config::Config;

/// Download and upload indices into the per-direction state arrays.
const DOWN: usize = 0;
const UP: usize = 1;

/// The auto-scale sample count: btop's five samples span 10 s at its 2000 ms;
/// keep that span at any interval (DEF-GRAPH-39-NET-JUMP, owner 2026-10-27).
fn scale_samples(update_ms: u64) -> usize {
    ((10_000 / update_ms.max(1)) as usize).max(5)
}

/// btop's lowest graph ceiling, `10 << 10` bytes per second.
const GRAPH_MIN: u64 = 10 << 10;

/// The lowest auto-scaled ceiling shown, 1 MiB/s (ACC-GRAPH-71-SCALE-FLOOR).
const SCALE_FLOOR: u64 = 1 << 20;

/// btop `net_stat`.
#[derive(Default)]
struct Counter {
    speed: f64,
    top: f64,
    total: u64,
    last: u64,
    offset: u64,
    rollover: u64,
}

#[derive(Default)]
struct InterfaceCounters {
    down: Counter,
    up: Counter,
    /// btop `net_info::bandwidth` for download and upload, newest last.
    bandwidth: [VecDeque<u64>; 2],
    /// btop's per-interface graph histories, one sample per tick.
    down_history: History,
    up_history: History,
}

struct NetState {
    /// btop `Net::timestamp`, milliseconds since the epoch; it starts at 0.
    timestamp_ms: u64,
    /// btop `Net::errors`; never reset, and at three collection stops updating.
    errors: u8,
    interfaces: BTreeMap<String, InterfaceCounters>,
    selected_iface: Option<String>,
    /// btop `graph_max`, bytes per second.
    graph_max: [u64; 2],
    /// btop `max_count[dir][scale]`, the 1.3 (0) and 3.0 (1) counters.
    max_count: [[i32; 2]; 2],
    /// btop `rescale`, forced on the first sample and on every new selection.
    rescale: bool,
    /// The shown download and upload ceilings (ACC-GRAPH-67-SMOOTH-SCALE).
    scale: [Scale; 2],
    /// The last successful result, returned once the error gate closes.
    last: Option<NetSnapshot>,
}

impl Default for NetState {
    fn default() -> Self {
        Self {
            timestamp_ms: 0,
            errors: 0,
            interfaces: BTreeMap::new(),
            selected_iface: None,
            graph_max: [0; 2],
            max_count: [[0; 2]; 2],
            rescale: true,
            scale: Default::default(),
            last: None,
        }
    }
}

/// One `getifaddrs(3)` interface reduced to what `Net::collect` reads.
struct ObservedInterface {
    name: String,
    connected: bool,
    ipv4: Option<String>,
    ipv6: Option<String>,
}

/// One `net_stat` update for a readable counter.
#[derive(Clone, Copy, Default)]
struct CounterSample {
    speed: f64,
    top: f64,
    total: u64,
}

static LAST: Mutex<BTreeMap<PathBuf, NetState>> = Mutex::new(BTreeMap::new());

pub fn collect(proc_root: &Path, config: &Config) -> NetSnapshot {
    if config
        .container
        .as_ref()
        .is_some_and(|container| !container.own_netns)
    {
        return NetSnapshot::default();
    }
    let sys = sys_root(proc_root);
    let mut states = LAST.lock().unwrap_or_else(|error| error.into_inner());
    let state = states.entry(proc_root.to_path_buf()).or_default();
    // btop skips the update block once three lifetime getifaddrs calls have
    // failed, and keeps returning the last selected data.
    if state.errors >= 3 {
        return state.last.clone().unwrap_or_default();
    }
    let Ok(observed) = observed_interfaces() else {
        state.errors = state.errors.saturating_add(1);
        return NetSnapshot::default();
    };
    let snapshot = sample(state, &sys, epoch_ms(), &observed, config);
    state.last = Some(snapshot.clone());
    snapshot
}

/// One `Net::collect` update over the interfaces `getifaddrs` reported.
fn sample(
    state: &mut NetState,
    sys: &Path,
    timestamp_ms: u64,
    observed: &[ObservedInterface],
    config: &Config,
) -> NetSnapshot {
    // btop measures (new_timestamp - timestamp) milliseconds; the first sample
    // therefore spans the whole epoch, not the uptime.
    let dt_ms = timestamp_ms.saturating_sub(state.timestamp_ms).max(1);
    state.timestamp_ms = timestamp_ms;

    // btop erases interfaces that disappeared, forgetting their counters.
    state
        .interfaces
        .retain(|name, _| observed.iter().any(|iface| &iface.name == name));

    // The auto-scale counters use the selection from the previous cycle: btop
    // updates them before it re-selects, so a new selection starts at zero.
    let selected = state.selected_iface.clone();
    let n = scale_samples(config.update_ms);
    let mut interfaces = Vec::with_capacity(observed.len());
    let mut total_down = 0.0;
    let mut total_up = 0.0;
    let mut total_down_known = false;
    let mut total_up_known = false;
    for iface in observed {
        let base = sys.join("class/net").join(&iface.name);
        let counters = state.interfaces.entry(iface.name.clone()).or_default();
        // btop updates and tests download before it updates upload, so the
        // sync test below sees the previous tick's upload speed.
        let previous_up_speed = counters.up.speed;
        let down = update_counter(
            &mut counters.down,
            read_counter(&base.join("statistics/rx_bytes")),
            dt_ms,
        );
        let up = update_counter(
            &mut counters.up,
            read_counter(&base.join("statistics/tx_bytes")),
            dt_ms,
        );
        if let Some(sample) = down {
            push_bandwidth(&mut counters.bandwidth[DOWN], sample.speed, n);
            counters
                .down_history
                .push(Some(sample.speed), config.update_ms);
            total_down += sample.speed;
            total_down_known = true;
        }
        if let Some(sample) = up {
            push_bandwidth(&mut counters.bandwidth[UP], sample.speed, n);
            counters
                .up_history
                .push(Some(sample.speed), config.update_ms);
            total_up += sample.speed;
            total_up_known = true;
        }
        if config.net_auto && selected.as_deref() == Some(iface.name.as_str()) {
            // With net_sync only the faster direction counts, so the direction
            // the sync copies from is the one that drives the rescale.
            if let Some(sample) = down
                && !(config.net_sync && counters.down.speed < previous_up_speed)
            {
                count_scale(
                    &mut state.max_count[DOWN],
                    sample.speed,
                    state.graph_max[DOWN],
                );
            }
            if let Some(sample) = up
                && !(config.net_sync && counters.up.speed < counters.down.speed)
            {
                count_scale(&mut state.max_count[UP], sample.speed, state.graph_max[UP]);
            }
        }
        let mut ipv4 = iface.ipv4.clone();
        let ipv6 = iface.ipv6.clone();
        // btop falls back to the sysfs MAC when neither address family exists.
        if ipv4.is_none() && ipv6.is_none() {
            ipv4 = read_trimmed(&base.join("address")).filter(|value| !value.is_empty());
        }
        interfaces.push(NetInterface {
            ip: ipv4.clone().or_else(|| ipv6.clone()),
            name: iface.name.clone(),
            ipv4,
            ipv6,
            connected: Some(iface.connected),
            down_rate: down.map(|sample| sample.speed),
            up_rate: up.map(|sample| sample.speed),
            total_down: down.map(|sample| sample.total),
            total_up: up.map(|sample| sample.total),
            top_down: down.map(|sample| sample.top),
            top_up: up.map(|sample| sample.top),
        });
    }
    if interfaces.is_empty() {
        return NetSnapshot::default();
    }

    // btop re-selects only when the saved interface is gone; the configured
    // interface wins over the largest total.
    let selection_absent = state
        .selected_iface
        .as_ref()
        .is_none_or(|selected| !interfaces.iter().any(|iface| &iface.name == selected));
    if selection_absent {
        state.max_count = [[0; 2]; 2];
        state.scale = Default::default();
        if config.net_auto {
            state.rescale = true;
        }
        state.selected_iface = if !config.net_iface.is_empty()
            && interfaces
                .iter()
                .any(|iface| iface.name == config.net_iface)
        {
            Some(config.net_iface.clone())
        } else {
            // DEF-NET-42-LOOPBACK-PICK: not lo, then connected, then the
            // largest total; the first interface wins a tie, as before.
            let mut best: Option<&NetInterface> = None;
            let mut best_key = (false, false, 0_u64);
            for iface in &interfaces {
                let total = iface
                    .total_down
                    .unwrap_or(0)
                    .saturating_add(iface.total_up.unwrap_or(0));
                let key = (iface.name != "lo", iface.connected == Some(true), total);
                if best.is_none() || key > best_key {
                    best = Some(iface);
                    best_key = key;
                }
            }
            best.map(|iface| iface.name.clone())
        };
    }

    // btop's auto scale: rescale when forced or when a counter reached `n`,
    // taking the mean of the selected interface's last `n` samples.
    if config.net_auto {
        'directions: for direction in [DOWN, UP] {
            for scale in 0..2 {
                if !(state.rescale || state.max_count[direction][scale] >= n as i32) {
                    continue;
                }
                let average = match state.selected_iface.as_deref() {
                    Some(name) => {
                        let counters = &state.interfaces[name];
                        let bandwidth = &counters.bandwidth[direction];
                        if bandwidth.len() > n {
                            bandwidth.iter().rev().take(n).sum::<u64>() / n as u64
                        } else if direction == DOWN {
                            counters.down.speed.max(0.0) as u64
                        } else {
                            counters.up.speed.max(0.0) as u64
                        }
                    }
                    None => 0,
                };
                let factor = if scale == 0 { 1.3 } else { 3.0 };
                state.graph_max[direction] = ((average as f64 * factor) as u64).max(GRAPH_MIN);
                state.max_count[direction] = [0; 2];
                if config.net_sync {
                    let other = if direction == DOWN { UP } else { DOWN };
                    state.graph_max[other] = state.graph_max[direction];
                    state.max_count[other] = [0; 2];
                    break 'directions;
                }
                break;
            }
        }
    }
    state.rescale = false;

    let (graph_max_down, graph_max_up) = if config.net_auto {
        (Some(state.graph_max[DOWN]), Some(state.graph_max[UP]))
    } else {
        (
            Some(fixed_graph_max(config.net_download)),
            Some(fixed_graph_max(config.net_upload)),
        )
    };
    let floor = if config.net_auto { SCALE_FLOOR } else { 0 };
    let shown = |graph_max: Option<u64>| graph_max.unwrap_or(0).max(floor) as f64;
    let scale_down = state.scale[DOWN].update(shown(graph_max_down), config.update_ms, NET_PLOT);
    let scale_up = state.scale[UP].update(shown(graph_max_up), config.update_ms, NET_PLOT);
    let (down_chart, up_chart) = state
        .selected_iface
        .as_deref()
        .and_then(|name| state.interfaces.get(name))
        .map(|counters| {
            (
                counters
                    .down_history
                    .chart(NET_PLOT, 0.0, scale_down.draw_max),
                counters.up_history.chart(NET_PLOT, 0.0, scale_up.draw_max),
            )
        })
        .unwrap_or_default();
    NetSnapshot {
        interfaces,
        selected_iface: state.selected_iface.clone(),
        total_down: total_down_known.then_some(total_down),
        total_up: total_up_known.then_some(total_up),
        graph_max_down,
        graph_max_up,
        down_chart,
        up_chart,
        scale_down,
        scale_up,
    }
}

/// btop's `net_stat` update: rollover on a counter that went backwards, then
/// `round((val - last) / ((new_timestamp - timestamp) / 1000))`.
fn update_counter(counter: &mut Counter, value: Option<u64>, dt_ms: u64) -> Option<CounterSample> {
    let value = value?;
    if value < counter.last {
        counter.rollover = counter.rollover.saturating_add(counter.last);
        counter.last = 0;
    }
    if counter.rollover.checked_add(value).is_none() {
        counter.rollover = 0;
        counter.last = 0;
    }
    counter.speed = ((value - counter.last) as f64 / (dt_ms as f64 / 1000.0)).round();
    counter.top = counter.top.max(counter.speed);
    if counter.offset > value + counter.rollover {
        counter.offset = 0;
    }
    counter.total = value + counter.rollover - counter.offset;
    counter.last = value;
    Some(CounterSample {
        speed: counter.speed,
        top: counter.top,
        total: counter.total,
    })
}

/// btop's per-direction auto-scale counters: a sample above the ceiling pushes
/// towards the 1.3 factor, a sample below a tenth of it towards the 3.0 factor.
fn count_scale(max_count: &mut [i32; 2], speed: f64, graph_max: u64) {
    if speed > graph_max as f64 {
        max_count[0] += 1;
        if max_count[1] > 0 {
            max_count[1] -= 1;
        }
    } else if graph_max > GRAPH_MIN && speed < (graph_max / 10) as f64 {
        max_count[1] += 1;
        if max_count[0] > 0 {
            max_count[0] -= 1;
        }
    }
}

/// btop's fixed ceiling when `net_auto` is false:
/// `(net_download << 20) / 8` bytes per second (`btop_draw.cpp:1385-1386`),
/// widened before the shift as btop does.
fn fixed_graph_max(mebibits: i32) -> u64 {
    (((mebibits as i64) << 20) as u64) / 8
}

/// btop `net_info::bandwidth`: keep the newest `n + 1` samples for the auto
/// scale, which reproduces btop's `size() > 5` test as `len() > n`.
fn push_bandwidth(bandwidth: &mut VecDeque<u64>, speed: f64, n: usize) {
    bandwidth.push_back(speed.max(0.0) as u64);
    while bandwidth.len() > n + 1 {
        bandwidth.pop_front();
    }
}

/// btop `time_ms()`: milliseconds since the Unix epoch.
fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

/// btop enumerates interfaces from `getifaddrs(3)`, including those with no
/// address. The first entry for a name sets `connected`; the first IPv4 and
/// first IPv6 address of each family are kept.
fn observed_interfaces() -> Result<Vec<ObservedInterface>, ()> {
    let addresses = getifaddrs().map_err(|_| ())?;
    let mut interfaces: Vec<ObservedInterface> = Vec::new();
    for address in addresses {
        let index = match interfaces
            .iter()
            .position(|iface| iface.name == address.interface_name)
        {
            Some(index) => index,
            None => {
                interfaces.push(ObservedInterface {
                    name: address.interface_name.clone(),
                    connected: address.flags.contains(InterfaceFlags::IFF_RUNNING),
                    ipv4: None,
                    ipv6: None,
                });
                interfaces.len() - 1
            }
        };
        let iface = &mut interfaces[index];
        let Some(socket) = address.address else {
            continue;
        };
        if let Some(inet) = socket.as_sockaddr_in() {
            iface.ipv4.get_or_insert_with(|| inet.ip().to_string());
        } else if let Some(inet6) = socket.as_sockaddr_in6() {
            iface.ipv6.get_or_insert_with(|| inet6.ip().to_string());
        }
    }
    Ok(interfaces)
}

fn read_counter(path: &Path) -> Option<u64> {
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

    fn default_config() -> Config {
        Config::load(None).expect("embedded defaults")
    }

    fn observed(names: &[&str]) -> Vec<ObservedInterface> {
        names
            .iter()
            .map(|name| ObservedInterface {
                name: (*name).to_string(),
                connected: true,
                ipv4: None,
                ipv6: None,
            })
            .collect()
    }

    /// One sample over the `eth0` counters recorded in `net/auto-scale/<step>`.
    fn feed(
        state: &mut NetState,
        root: &Path,
        step: &str,
        timestamp_ms: u64,
        config: &Config,
    ) -> NetSnapshot {
        sample(
            state,
            &root.join(step).join("sys"),
            timestamp_ms,
            &observed(&["eth0"]),
            config,
        )
    }

    #[test]
    fn host_network_namespace_in_a_container_is_unavailable() {
        let mut config = default_config();
        config.container = Some(crate::collector::container::Container {
            runtime: String::from("docker"),
            signal: String::from("1 /.dockerenv"),
            cgroup: None,
            own_netns: false,
        });
        let snapshot = collect(&fixture("proc"), &config);
        assert!(snapshot.interfaces.is_empty());
        assert_eq!(snapshot.selected_iface, None);
        assert_eq!(snapshot.total_down, None);
    }

    #[test]
    fn first_sample_uses_btops_epoch_delta_window() {
        let mut state = NetState::default();
        let snapshot = sample(
            &mut state,
            &fixture("sys"),
            1_700_000_000_000,
            &observed(&["lo", "eth0", "eth1"]),
            &default_config(),
        );
        let eth0 = snapshot
            .interfaces
            .iter()
            .find(|iface| iface.name == "eth0")
            .expect("eth0");
        // 433283949346 bytes over 1.7e9 s since the epoch, not over boot.
        assert_eq!(eth0.down_rate, Some(255.0));
        assert_eq!(eth0.up_rate, Some(16.0));
        assert_eq!(eth0.total_down, Some(433_283_949_346));
        // Loopback carries the largest total; DEF-NET-42 skips it.
        assert_eq!(snapshot.selected_iface.as_deref(), Some("eth0"));
        assert_eq!(state.interfaces["eth0"].down_history.len(), 1);
        assert_eq!(state.interfaces["eth0"].up_history.len(), 1);

        // A second tick gives the selected interface's histories a curve.
        let second = sample(
            &mut state,
            &fixture("sys"),
            1_700_000_002_000,
            &observed(&["lo", "eth0", "eth1"]),
            &default_config(),
        );
        assert!(
            second.down_chart.line.starts_with('M'),
            "{}",
            second.down_chart.line
        );
        assert!(!second.up_chart.line.is_empty());
    }

    #[test]
    fn counter_reset_rolls_over_like_btop() {
        let root = fixture("net/counter-reset");
        let mut state = NetState::default();
        let config = default_config();
        let first = sample(
            &mut state,
            &root.join("before/sys"),
            1_700_000_000_000,
            &observed(&["eth0"]),
            &config,
        );
        assert_eq!(first.interfaces[0].down_rate, Some(0.0));
        assert_eq!(first.interfaces[0].total_down, Some(4096));

        let second = sample(
            &mut state,
            &root.join("after/sys"),
            1_700_000_002_000,
            &observed(&["eth0"]),
            &config,
        );
        let eth0 = &second.interfaces[0];
        // 1024 < 4096: rollover = 4096, last = 0, speed = 1024 / 2 s.
        assert_eq!(eth0.down_rate, Some(512.0));
        assert_eq!(eth0.total_down, Some(5120));
        assert_eq!(eth0.top_down, Some(512.0));
        // The upload counter only grew: (10000 - 8192) / 2 s.
        assert_eq!(eth0.up_rate, Some(904.0));
        assert_eq!(eth0.total_up, Some(10_000));
    }

    #[test]
    fn disappeared_interface_is_dropped_and_its_counters_forgotten() {
        let root = fixture("net/interface-gone");
        let mut state = NetState::default();
        let config = default_config();
        let first = sample(
            &mut state,
            &root.join("before/sys"),
            1_700_000_000_000,
            &observed(&["eth0", "eth1"]),
            &config,
        );
        assert_eq!(first.interfaces.len(), 2);
        assert_eq!(first.selected_iface.as_deref(), Some("eth1"));

        let second = sample(
            &mut state,
            &root.join("after/sys"),
            1_700_000_002_000,
            &observed(&["eth0"]),
            &config,
        );
        assert_eq!(second.interfaces.len(), 1);
        assert_eq!(second.interfaces[0].name, "eth0");
        assert_eq!(second.selected_iface.as_deref(), Some("eth0"));
        assert!(!state.interfaces.contains_key("eth1"));
    }

    #[test]
    fn net_iface_selects_the_named_interface_and_falls_back_to_the_largest_total() {
        let mut state = NetState::default();
        let mut config = default_config();
        config.net_iface = "eth0".to_string();
        let snapshot = sample(
            &mut state,
            &fixture("sys"),
            1_700_000_000_000,
            &observed(&["lo", "eth0", "eth1"]),
            &config,
        );
        // lo carries the largest total, but the configured name wins.
        assert_eq!(snapshot.selected_iface.as_deref(), Some("eth0"));

        let mut state = NetState::default();
        let mut config = default_config();
        config.net_iface = "absent0".to_string();
        let snapshot = sample(
            &mut state,
            &fixture("sys"),
            1_700_000_000_000,
            &observed(&["lo", "eth0", "eth1"]),
            &config,
        );
        // A name outside the interface list falls back to the largest total
        // other than lo (DEF-NET-42-LOOPBACK-PICK).
        assert_eq!(snapshot.selected_iface.as_deref(), Some("eth0"));
    }

    /// DEF-NET-42-LOOPBACK-PICK: in the fixture lo carries 815 GB against
    /// eth0's 460 GB. Without net_iface the pick skips lo, prefers a connected
    /// interface, and takes lo only when nothing else exists; net_iface = lo
    /// still forces it.
    #[test]
    fn the_pick_skips_lo_unless_it_is_the_only_interface_or_configured() {
        let pick = |observed: Vec<ObservedInterface>, net_iface: &str| {
            let mut config = default_config();
            config.net_iface = net_iface.to_string();
            sample(
                &mut NetState::default(),
                &fixture("sys"),
                1_700_000_000_000,
                &observed,
                &config,
            )
            .selected_iface
        };
        assert_eq!(
            pick(observed(&["lo", "eth0", "eth1"]), ""),
            Some("eth0".into())
        );
        assert_eq!(pick(observed(&["lo"]), ""), Some("lo".into()));
        assert_eq!(
            pick(observed(&["lo", "eth0", "eth1"]), "lo"),
            Some("lo".into())
        );
        let mut down = observed(&["lo", "eth0", "eth1"]);
        down[1].connected = false;
        assert_eq!(pick(down, ""), Some("eth1".into()));
    }

    #[test]
    fn auto_scale_rescales_up_from_the_last_five_samples() {
        let root = fixture("net/auto-scale");
        let mut state = NetState::default();
        // btop's 2000 ms interval: five samples, btop parity.
        let mut config = default_config();
        config.update_ms = 2000;
        let start = 1_700_000_000_000;
        for (index, step) in ["s0", "s1", "s2", "s3", "s4"].iter().enumerate() {
            let at = start + index as u64 * 1_000;
            let snapshot = feed(&mut state, &root, step, at, &config);
            // Four samples sit above the floor; five are needed to rescale.
            assert_eq!(snapshot.graph_max_down, Some(GRAPH_MIN), "{step}");
        }
        let snapshot = feed(&mut state, &root, "s5", start + 5_000, &config);
        // Mean of the last five speeds, (100k + 200k + 300k + 400k + 500k) / 5,
        // times 1.3. net_sync copies the ceiling to upload.
        assert_eq!(snapshot.graph_max_down, Some(390_000));
        assert_eq!(snapshot.graph_max_up, Some(390_000));
        let snapshot = feed(&mut state, &root, "s6", start + 6_000, &config);
        assert_eq!(snapshot.graph_max_down, Some(390_000));
    }

    #[test]
    fn auto_scale_rescales_down_with_the_30_factor_and_the_10_kib_floor() {
        let root = fixture("net/auto-scale");
        let mut state = NetState::default();
        // btop's 2000 ms interval: five samples, btop parity.
        let mut config = default_config();
        config.update_ms = 2000;
        let start = 1_700_000_000_000;
        for (index, step) in ["s0", "s1", "s2", "s3", "s4", "s5", "s6"]
            .iter()
            .enumerate()
        {
            feed(
                &mut state,
                &root,
                step,
                start + index as u64 * 1_000,
                &config,
            );
        }
        for (index, step) in ["l1", "l2", "l3", "l4", "l5"].iter().enumerate() {
            let at = start + 7_000 + index as u64 * 1_000;
            let snapshot = feed(&mut state, &root, step, at, &config);
            assert_eq!(snapshot.graph_max_down, Some(390_000), "{step}");
        }
        // l1 does not count: with net_sync the download is compared with the
        // previous tick's upload, which is still the 60 kB/s peak of s6. The
        // fifth low sample at l6 takes the mean 1 kB/s * 3.0 under the floor.
        let snapshot = feed(&mut state, &root, "l6", start + 12_000, &config);
        assert_eq!(snapshot.graph_max_down, Some(GRAPH_MIN));
    }

    #[test]
    fn net_sync_download_scale_compares_the_previous_upload_speed() {
        let root = fixture("net/sync-order");
        let mut state = NetState::default();
        // btop's 2000 ms interval: five samples, btop parity.
        let mut config = default_config();
        config.update_ms = 2000;
        config.net_sync = true;
        let start = 1_700_000_000_000;
        feed(&mut state, &root, "t0", start, &config);
        // t1: download 1 kB/s, upload 500 kB/s.
        feed(&mut state, &root, "t1", start + 1_000, &config);
        // t2: download jumps to 200 kB/s while upload drops to 1 kB/s. btop
        // tests download before it updates upload, so 200 kB/s is compared
        // with t1's 500 kB/s and does not count; the current 1 kB/s would.
        let snapshot = feed(&mut state, &root, "t2", start + 2_000, &config);
        assert_eq!(snapshot.graph_max_down, Some(GRAPH_MIN));
        assert_eq!(state.max_count[DOWN], [0, 0]);
    }

    #[test]
    fn net_sync_download_scale_steady_state_uses_the_current_upload_speed() {
        let root = fixture("net/sync-order");
        let mut state = NetState::default();
        // btop's 2000 ms interval: five samples, btop parity.
        let mut config = default_config();
        config.update_ms = 2000;
        config.net_sync = true;
        let start = 1_700_000_000_000;
        for (index, step) in ["t0", "t1", "t2"].iter().enumerate() {
            feed(
                &mut state,
                &root,
                step,
                start + index as u64 * 1_000,
                &config,
            );
        }
        // t3: the upload has held at 1 kB/s for a tick, so the previous and the
        // current comparison agree and the download counts under either rule.
        let snapshot = feed(&mut state, &root, "t3", start + 3_000, &config);
        assert_eq!(snapshot.graph_max_down, Some(GRAPH_MIN));
        assert_eq!(state.max_count[DOWN], [1, 0]);
        for (index, step) in ["t4", "t5", "t6"].iter().enumerate() {
            let at = start + 4_000 + index as u64 * 1_000;
            let snapshot = feed(&mut state, &root, step, at, &config);
            assert_eq!(snapshot.graph_max_down, Some(GRAPH_MIN), "{step}");
        }
        // t7 reaches btop's count of five one tick later than a current-tick
        // comparison would: 200 kB/s * 1.3, copied to upload by net_sync.
        let snapshot = feed(&mut state, &root, "t7", start + 7_000, &config);
        assert_eq!(snapshot.graph_max_down, Some(260_000));
        assert_eq!(snapshot.graph_max_up, Some(260_000));
    }

    #[test]
    fn net_sync_false_scales_each_direction_from_its_own_samples() {
        let root = fixture("net/auto-scale");
        let mut state = NetState::default();
        // btop's 2000 ms interval: five samples, btop parity.
        let mut config = default_config();
        config.update_ms = 2000;
        config.net_sync = false;
        let start = 1_700_000_000_000;
        let mut snapshot = NetSnapshot::default();
        for (index, step) in ["s0", "s1", "s2", "s3", "s4", "s5", "s6"]
            .iter()
            .enumerate()
        {
            snapshot = feed(
                &mut state,
                &root,
                step,
                start + index as u64 * 1_000,
                &config,
            );
        }
        // Download: 300k mean * 1.3. Upload: (20k + 30k + 40k + 50k + 60k) / 5
        // * 1.3, from its own five samples.
        assert_eq!(snapshot.graph_max_down, Some(390_000));
        assert_eq!(snapshot.graph_max_up, Some(52_000));
    }

    /// DEF-GRAPH-39-NET-JUMP: at 250 ms the auto scale counts 40 samples,
    /// btop's 10 s span, where btop's five would rescale after 1.25 s.
    #[test]
    fn auto_scale_at_250_ms_keeps_btops_10_s_span() {
        let root = std::env::temp_dir().join(format!("btop-web-net-span-{}", std::process::id()));
        let stats = root.join("sys/class/net/eth0/statistics");
        std::fs::create_dir_all(&stats).expect("temp tree");
        let mut state = NetState::default();
        let config = default_config();
        assert_eq!(config.update_ms, 250);
        let (mut rx, mut at) = (0_u64, 1_700_000_000_000_u64);
        let mut tick = |state: &mut NetState, bytes: u64| {
            rx += bytes;
            at += 250;
            std::fs::write(stats.join("rx_bytes"), rx.to_string()).expect("rx");
            std::fs::write(stats.join("tx_bytes"), "0").expect("tx");
            sample(state, &root.join("sys"), at, &observed(&["eth0"]), &config).graph_max_down
        };
        assert_eq!(tick(&mut state, 0), Some(GRAPH_MIN));
        // 100 kB/s: 39 counts keep the floor, the 40th takes 1.3 x the mean.
        for count in 1..40 {
            assert_eq!(tick(&mut state, 25_000), Some(GRAPH_MIN), "count {count}");
        }
        assert_eq!(tick(&mut state, 25_000), Some(130_000));
        // 1 kB/s is below a tenth of the ceiling: 39 counts keep it, the 40th
        // takes 3.0 x the mean, which is under the floor.
        for count in 1..40 {
            assert_eq!(tick(&mut state, 250), Some(130_000), "count {count}");
        }
        assert_eq!(tick(&mut state, 250), Some(GRAPH_MIN));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn net_auto_false_uses_the_fixed_mebibit_ceilings() {
        let mut config = default_config();
        config.net_auto = false;
        config.net_download = 250;
        config.net_upload = 50;
        let snapshot = sample(
            &mut NetState::default(),
            &fixture("sys"),
            1_700_000_000_000,
            &observed(&["lo", "eth0", "eth1"]),
            &config,
        );
        assert_eq!(snapshot.graph_max_down, Some(32_768_000));
        assert_eq!(snapshot.graph_max_up, Some(6_553_600));
    }

    /// ACC-GRAPH-67 and ACC-GRAPH-71: btop's rescale to 390 kB stays under the
    /// 1 MiB/s floor, so the shown ceiling holds the floor and no change runs;
    /// a fixed ceiling never runs one.
    #[test]
    fn a_rescale_under_the_floor_and_a_fixed_ceiling_run_no_change() {
        let root = fixture("net/auto-scale");
        for net_auto in [true, false] {
            let mut state = NetState::default();
            let mut config = default_config();
            config.update_ms = 2000;
            config.net_auto = net_auto;
            let start = 1_700_000_000_000;
            let mut snapshot = NetSnapshot::default();
            for (index, step) in ["s0", "s1", "s2", "s3", "s4", "s5"].iter().enumerate() {
                snapshot = feed(
                    &mut state,
                    &root,
                    step,
                    start + index as u64 * 1_000,
                    &config,
                );
                assert!(snapshot.scale_down.style.is_empty(), "{step}");
            }
            if net_auto {
                assert_eq!(snapshot.graph_max_down, Some(390_000));
                assert_eq!(snapshot.scale_down.label_max, Some(SCALE_FLOOR));
                assert_eq!(snapshot.scale_down.draw_max, SCALE_FLOOR as f64);
            } else {
                assert_eq!(snapshot.scale_down.label_max, snapshot.graph_max_down);
            }
        }
    }

    /// ACC-GRAPH-71-SCALE-FLOOR at btop's 2000 ms and at 250 ms: idle traffic
    /// keeps the shown ceiling at 1 MiB/s while btop's ceiling follows it; a
    /// burst above the floor rises through a change, and the fall afterwards
    /// moves smoothly and stops at the floor.
    #[test]
    fn the_shown_ceiling_never_goes_below_the_floor() {
        for update_ms in [2000_u64, 250] {
            let root = std::env::temp_dir().join(format!(
                "btop-web-net-floor-{}-{update_ms}",
                std::process::id()
            ));
            let stats = root.join("sys/class/net/eth0/statistics");
            std::fs::create_dir_all(&stats).expect("temp tree");
            let mut state = NetState::default();
            let mut config = default_config();
            config.update_ms = update_ms;
            let n = scale_samples(update_ms);
            let (mut rx, mut at) = (0_u64, 1_700_000_000_000_u64);
            let mut tick = |state: &mut NetState, rate: u64| {
                rx += rate * update_ms / 1000;
                at += update_ms;
                std::fs::write(stats.join("rx_bytes"), rx.to_string()).expect("rx");
                std::fs::write(stats.join("tx_bytes"), "0").expect("tx");
                sample(state, &root.join("sys"), at, &observed(&["eth0"]), &config)
            };
            tick(&mut state, 0);
            // 100 kB/s: btop rescales to 130 kB, the shown ceiling holds 1 MiB/s.
            for _ in 0..2 * n {
                let snapshot = tick(&mut state, 100_000);
                assert_eq!(snapshot.scale_down.label_max, Some(SCALE_FLOOR));
                assert!(snapshot.scale_down.style.is_empty());
            }
            assert_eq!(state.graph_max[DOWN], 130_000);
            // 2 MB/s for n ticks: the ceiling rises to 2.6 MB/s from the floor.
            for _ in 1..n {
                tick(&mut state, 2_000_000);
            }
            let snapshot = tick(&mut state, 2_000_000);
            assert_eq!(snapshot.graph_max_down, Some(2_600_000), "{update_ms} ms");
            assert_eq!(snapshot.scale_down.label_max, Some(SCALE_FLOOR));
            assert_eq!(snapshot.scale_down.draw_max, 2_600_000.0);
            assert!(!snapshot.scale_down.style.is_empty());
            // 100 kB/s again: btop falls to 300 kB; the shown ceiling falls from
            // 2.6 MB/s over several frames and stops at the floor.
            for _ in 1..n {
                tick(&mut state, 100_000);
            }
            let mut labels = Vec::new();
            for _ in 0..8 {
                let snapshot = tick(&mut state, 100_000);
                assert_eq!(snapshot.graph_max_down, Some(300_000));
                labels.push(snapshot.scale_down.label_max.expect("label"));
            }
            assert_eq!(labels[0], 2_600_000, "{update_ms} ms: {labels:?}");
            assert!(
                labels.windows(2).all(|pair| pair[1] <= pair[0]),
                "{labels:?}"
            );
            assert_eq!(labels.last(), Some(&SCALE_FLOOR), "{labels:?}");
            if update_ms == 250 {
                assert!(labels[1] > SCALE_FLOOR, "one frame: {labels:?}");
            }
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}

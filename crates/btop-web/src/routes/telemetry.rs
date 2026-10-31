//! ACC-NET-181-JSON-STREAM: the telemetry stream beside `/events`, as
//! newline-delimited JSON. One line per published frame, at the same interval
//! the page's SSE fragments arrive; a reconnecting client gets the held frame
//! at once, after the same wake and stale wait `/events` uses.
//!
//! The line is serialized from the published [`Frame`]'s snapshot only. The
//! sampler collects once and renders the HTML fragments once; this module adds
//! no collector, no timer and no subscription to the sampler, and a viewer
//! costs one serialization of the frame it was already sent.
//!
//! The schema, with units in the field suffix:
//!
//! ```json
//! {
//!   "events": ["cpu", "mem", "net", "proc", "gpu"],
//!   "update_ms": 250, "window_ms": 180000, "slow_ms": 1000, "slow_seq": 7,
//!   "container": false,
//!   "cpu": {
//!     "total_pct", "gauge_pct", "cores": [{"name", "usage_pct",
//!     "smoothed_pct"}], "frequency_mhz", "load_avg": [1m, 5m, 15m],
//!     "load_scale": "normalised" | "aggregate", "temperature_c",
//!     "battery_pct", "name", "cpu_hz", "fields": {btop's ten /proc/stat
//!     fields}, "core_temperatures_c", "temperature_max_c",
//!     "battery_seconds", "battery_status", "effective_cpus", "pressure_pct"
//!   },
//!   "mem": {
//!     "ram_total", "ram_used", "ram_free", "ram_available", "ram_cache",
//!     "ram_other", "ram_other_cache", "ram_limit", "ram_percent", "gauge_pct",
//!     "ram_free_percent", "ram_available_percent", "ram_cache_percent",
//!     "ram_other_percent", "ram_other_cache_percent", "swap_total",
//!     "swap_used", "swap_free",
//!     "swap_percent", "swap_free_percent", "has_swap", "swap_enabled",
//!     "disks": [{"name", "mount", "dev", "fstype", "used", "total", "free",
//!     "used_percent", "free_percent", "io_read", "io_write", "io_activity"}]
//!   },
//!   "net": {
//!     "interfaces": [{"name", "ip", "ipv4", "ipv6", "connected", "down_rate",
//!     "up_rate", "total_down", "total_up", "top_down", "top_up"}],
//!     "selected_iface", "total_down", "total_up", "graph_max_down",
//!     "graph_max_up",
//!     "link": {"speed_mbps", "duplex", "operstate", "errors", "drops"}
//!   },
//!   "proc": {
//!     "total", "running", "sleeping", "waiting", "zombie", "threads", "top",
//!     "cpu_80", "mem_80", "chart_max",
//!     "entries": [{"pid", "name", "user", "cpu_pct", "cpu_c", "mem_bytes",
//!     "ppid", "threads", "nice", "depth", "state", "command", "sum"}]
//!   },
//!   "gpu": {
//!     "devices": [{"name", "utilization_pct", "memory_utilization_pct",
//!     "vram_used", "vram_total", "power_w", "power_max_w", "power_percent",
//!     "temperature_c", "temperature_max_c", "gpu_clock_mhz",
//!     "memory_clock_mhz", "pstate", "pcie_tx", "pcie_rx"}],
//!     "average_utilization_pct", "vram_percent", "power_percent",
//!     "process_count", "process_unreadable"
//!   },
//!   "plots": {
//!     "cpu":  [{"line", "area", "width", "height", "max"}],
//!     "mem":  [{"line", "area", "width", "height", "max"}],
//!     "net":  [download, upload],
//!     "proc": [{"line", "area", "width", "height", "max"}],
//!     "gpu":  [one per device, in node order]
//!   }
//! }
//! ```
//!
//! `_pct` is percent 0-100 (a container's effective CPUs scale the CPU load),
//! `_bytes` is bytes, `_rate` is bytes per second, `_ms` is milliseconds,
//! `_mhz` is MHz, `_c` is degrees Celsius, `_w` is watts, `_seconds` is
//! seconds and `io_read`/`io_write` are bytes in the last sample. `pstate` is
//! NVML's performance state. A value the collector could not read is `null`,
//! never zero. `proc` and its plot change at the process refresh period, named
//! by `slow_seq`, exactly as the page's `proc` event does.
//!
//! `plots` carries the SVG paths the page draws, in that plot's viewBox:
//! `width` and `height` are the box, `max` is the y ceiling (the floor is 0). A
//! network `max` only rises, and only a page load lowers it; `graph_max_down`
//! and `graph_max_up` are btop's own ceilings, which fall after `net_fall` s.
//! The curves are the same server-drawn PCHIP paths the page receives, so a
//! consumer draws no second series. A path ends past `width` in a flat tail, the
//! newest value repeated, which the page scrolls over and its plot box clips.
//!
//! `?once=1` answers with the newest frame and closes, which is what the top
//! bar's copy button reads. Client mode serves no local sampler, so the route
//! is a 404 there; the process that samples serves the stream.

use std::convert::Infallible;
use std::time::Duration;

use async_stream::stream;
use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::AppState;
use crate::chart::{self, ChartPath};
use crate::collector::{
    CpuSnapshot, GpuSnapshot, MemSnapshot, NetSnapshot, ProcEntry, ProcSnapshot, Snapshot,
};
use crate::error::AppError;

/// ACC-NET-181-JSON-STREAM: newline-delimited JSON, one document per frame.
pub const CONTENT_TYPE: &str = "application/x-ndjson";

/// The five event names every frame carries, in the order `/events` sends
/// them.
pub const EVENTS: [&str; 5] = ["cpu", "mem", "net", "proc", "gpu"];

pub async fn events_json(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
) -> Result<Response, AppError> {
    // ACC-NET-143-AGENT-MODES: the client process holds no snapshot; its page
    // is filled by the relayed HTML fragments.
    if state.client.is_some() {
        return Err(AppError::NotFound);
    }
    let once = query
        .as_deref()
        .is_some_and(|query| query.split('&').any(|pair| pair == "once=1"));
    let subscription = state.wake.subscribe();
    let interval = Duration::from_millis(state.config.borrow().update_ms.max(1));
    let mut receiver = state.snapshot.clone();
    // The state's receiver never marks its value seen, so a clone must, or
    // `changed` would report an old sample as new.
    receiver.borrow_and_update();
    if state.wake.stale(interval) {
        let _ = tokio::time::timeout(interval, receiver.changed()).await;
    }
    let mut held = (*receiver.borrow_and_update()).clone();
    if once {
        let Some(frame) = held else {
            return Ok((StatusCode::SERVICE_UNAVAILABLE, "").into_response());
        };
        return Ok((
            [
                (header::CONTENT_TYPE, CONTENT_TYPE),
                (header::CACHE_CONTROL, "no-store"),
            ],
            frame_json(&frame.snapshot),
        )
            .into_response());
    }
    let stream = stream! {
        let _subscription = subscription;
        loop {
            if let Some(frame) = &held {
                yield Ok::<_, Infallible>(frame_json(&frame.snapshot));
            }
            if receiver.changed().await.is_err() {
                break;
            }
            held = (*receiver.borrow_and_update()).clone();
        }
    };
    Ok((
        [
            (header::CONTENT_TYPE, CONTENT_TYPE),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

/// One frame as a JSON document with its newline.
fn frame_json(snapshot: &Snapshot) -> String {
    let mut line = serde_json::to_string(&json!({
        "events": EVENTS,
        "update_ms": snapshot.update_ms,
        "window_ms": snapshot.window_ms,
        "slow_ms": snapshot.slow_ms,
        "slow_seq": snapshot.slow_seq,
        "container": snapshot.container,
        "cpu": cpu(&snapshot.cpu),
        "mem": mem(&snapshot.mem),
        "net": net(&snapshot.net),
        "proc": proc(&snapshot.proc),
        "gpu": gpu(&snapshot.gpu),
        "plots": plots(snapshot),
    }))
    .expect("a JSON object serializes");
    line.push('\n');
    line
}

fn cpu(cpu: &CpuSnapshot) -> Value {
    json!({
        "total_pct": cpu.total_pct,
        "gauge_pct": cpu.gauge_pct,
        "cores": cpu.cores.iter().map(|core| json!({
            "name": core.name,
            "usage_pct": core.usage_pct,
            "smoothed_pct": core.smoothed_pct,
        })).collect::<Vec<_>>(),
        "frequency_mhz": cpu.frequency_mhz,
        "load_avg": cpu.load_avg,
        "load_scale": cpu.load_scale.as_str(),
        "temperature_c": cpu.temperature_c,
        "battery_pct": cpu.battery_pct,
        "name": cpu.name.as_deref(),
        "cpu_hz": cpu.cpu_hz.as_deref(),
        "fields": cpu.fields,
        "core_temperatures_c": cpu.core_temperatures_c,
        "temperature_max_c": cpu.temperature_max_c,
        "battery_seconds": cpu.battery_seconds,
        "battery_status": cpu.battery_status.as_deref(),
        "effective_cpus": cpu.effective_cpus,
        "pressure_pct": cpu.pressure_pct,
    })
}

fn mem(mem: &MemSnapshot) -> Value {
    json!({
        "ram_total": mem.ram_total,
        "ram_used": mem.ram_used,
        "ram_free": mem.ram_free,
        "ram_available": mem.ram_available,
        "ram_cache": mem.ram_cache,
        "ram_other": mem.ram_other,
        "ram_other_cache": mem.ram_other_cache,
        "ram_limit": mem.ram_limit,
        "ram_percent": mem.ram_percent,
        "gauge_pct": mem.gauge_pct,
        "ram_free_percent": mem.ram_free_percent,
        "ram_available_percent": mem.ram_available_percent,
        "ram_cache_percent": mem.ram_cache_percent,
        "ram_other_percent": mem.ram_other_percent,
        "ram_other_cache_percent": mem.ram_other_cache_percent,
        "swap_total": mem.swap_total,
        "swap_used": mem.swap_used,
        "swap_free": mem.swap_free,
        "swap_percent": mem.swap_percent,
        "swap_free_percent": mem.swap_free_percent,
        "has_swap": mem.has_swap,
        "swap_enabled": mem.swap_enabled,
        "disks": mem.disks.iter().map(|disk| json!({
            "name": disk.name,
            "mount": disk.mount,
            "dev": disk.dev,
            "fstype": disk.fstype,
            "used": disk.used,
            "total": disk.total,
            "free": disk.free,
            "used_percent": disk.used_percent,
            "free_percent": disk.free_percent,
            "io_read": disk.io_read,
            "io_write": disk.io_write,
            "io_activity": disk.io_activity,
        })).collect::<Vec<_>>(),
    })
}

fn net(net: &NetSnapshot) -> Value {
    json!({
        "interfaces": net.interfaces.iter().map(|interface| json!({
            "name": interface.name,
            "ip": interface.ip.as_deref(),
            "ipv4": interface.ipv4.as_deref(),
            "ipv6": interface.ipv6.as_deref(),
            "connected": interface.connected,
            "down_rate": interface.down_rate,
            "up_rate": interface.up_rate,
            "total_down": interface.total_down,
            "total_up": interface.total_up,
            "top_down": interface.top_down,
            "top_up": interface.top_up,
        })).collect::<Vec<_>>(),
        "selected_iface": net.selected_iface.as_deref(),
        "total_down": net.total_down,
        "total_up": net.total_up,
        "graph_max_down": net.graph_max_down,
        "graph_max_up": net.graph_max_up,
        "link": {
            "speed_mbps": net.link.speed_mbps,
            "duplex": net.link.duplex.as_deref(),
            "operstate": net.link.operstate.as_deref(),
            "errors": net.link.errors,
            "drops": net.link.drops,
        },
    })
}

fn proc(proc: &ProcSnapshot) -> Value {
    json!({
        "total": proc.total,
        "running": proc.running,
        "sleeping": proc.sleeping,
        "waiting": proc.waiting,
        "zombie": proc.zombie,
        "threads": proc.threads,
        "top": proc.top.as_ref().map(entry),
        "cpu_80": proc.cpu_80,
        "mem_80": proc.mem_80,
        "chart_max": proc.chart_max,
        "entries": proc.entries.iter().map(entry).collect::<Vec<_>>(),
    })
}

fn entry(entry: &ProcEntry) -> Value {
    json!({
        "pid": entry.pid,
        "name": entry.name.as_deref(),
        "user": entry.user.as_deref(),
        "cpu_pct": entry.cpu_pct,
        "cpu_c": entry.cpu_c,
        "mem_bytes": entry.mem_bytes,
        "ppid": entry.ppid,
        "threads": entry.threads,
        "nice": entry.nice,
        "depth": entry.depth,
        "state": entry.state.as_deref(),
        "command": entry.command.as_ref(),
        "sum": entry.sum.as_ref().map(|sum| json!({
            "cpu_pct": sum.cpu_pct,
            "mem_bytes": sum.mem_bytes,
            "threads": sum.threads,
        })),
    })
}

fn gpu(gpu: &GpuSnapshot) -> Value {
    json!({
        "devices": gpu.devices.iter().map(|device| json!({
            "name": device.name,
            "utilization_pct": device.utilization_pct,
            "memory_utilization_pct": device.memory_utilization_pct,
            "vram_used": device.vram_used,
            "vram_total": device.vram_total,
            "power_w": device.power_w,
            "power_max_w": device.power_max_w,
            "power_percent": device.power_percent,
            "temperature_c": device.temperature_c,
            "temperature_max_c": device.temperature_max_c,
            "gpu_clock_mhz": device.gpu_clock_mhz,
            "memory_clock_mhz": device.memory_clock_mhz,
            "pstate": device.pstate,
            "pcie_tx": device.pcie_tx,
            "pcie_rx": device.pcie_rx,
        })).collect::<Vec<_>>(),
        "average_utilization_pct": gpu.average_utilization_pct,
        "vram_percent": gpu.vram_percent,
        "power_percent": gpu.power_percent,
        "process_count": gpu.process_count,
        "process_unreadable": gpu.process_unreadable,
    })
}

fn plots(snapshot: &Snapshot) -> Value {
    let lane = chart::gpu_lane(snapshot.gpu.devices.len());
    json!({
        "cpu": [series(&snapshot.cpu.chart, chart::CPU_HISTORY.width,
                       chart::CPU_HISTORY.height, 100.0)],
        "mem": [series(&snapshot.mem.used_chart, chart::MEM_HISTORY.width,
                       chart::MEM_HISTORY.height, 100.0)],
        "net": [
            series(&snapshot.net.down_chart, chart::NET_PLOT.width,
                   chart::NET_PLOT.height, snapshot.net.scale_down.draw_max),
            series(&snapshot.net.up_chart, chart::NET_PLOT.width,
                   chart::NET_PLOT.height, snapshot.net.scale_up.draw_max),
        ],
        "proc": [series(&snapshot.proc.chart, chart::PROC_COUNT.width,
                        chart::PROC_COUNT.height, snapshot.proc.chart_max as f64)],
        "gpu": snapshot.gpu.devices.iter()
            .map(|device| series(&device.chart, lane.width, lane.height, 100.0))
            .collect::<Vec<_>>(),
    })
}

fn series(chart: &ChartPath, width: f64, height: f64, max: f64) -> Value {
    json!({
        "line": chart.line,
        "area": chart.area,
        "width": width,
        "height": height,
        "max": max,
    })
}

//! The published frame: one snapshot plus its five SSE events, built once by
//! the sampler and shared by every stream.
//!
//! ACC-PERF-16-NO-OVERHEAD: N clients cost no extra collection or rendering.
//! The sampler builds each event once per sample and publishes the text behind
//! an `Arc`; a stream sends the held strings, so a viewer adds only its own
//! event framing and socket write.
//!
//! The CPU and memory panels travel as one pipe-delimited record of the values
//! the page shows, not as HTML: the browser writes them into the nodes the
//! first page already rendered, so a tick costs one `split` and the changed
//! text, attribute and path writes instead of parsing and patching a subtree
//! (ACC-PERF-133-PAGE-COST). An empty field is an unavailable value. The field
//! order is fixed and mirrored by `applyCpuFrame` and `applyMemFrame` in
//! `assets/app.js`; the version token first lets either side refuse the other's
//! format.

use std::sync::{Arc, OnceLock};

use askama::Template;

use super::{Snapshot, band, bytes, mount_tip};
use crate::chart;

/// One sample and the five SSE events built for it.
pub struct Frame {
    pub snapshot: Snapshot,
    /// The named events in send order, built once by [`Frame::new`].
    pub events: Vec<(&'static str, String)>,
    /// ACC-VIEW-210-COPY-HELD: the slow pass's `/events.json` text, one cell
    /// shared by the pass's frames. The first connection that asks builds it,
    /// and with no connection asking it stays empty.
    pub telemetry: Arc<OnceLock<String>>,
}

impl Frame {
    /// One sample with its five events, each built once.
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            events: fragments(&snapshot),
            telemetry: Arc::default(),
            snapshot,
        }
    }
}

#[derive(Template)]
#[template(path = "event_net.html")]
struct NetEvent<'a> {
    snapshot: &'a Snapshot,
}

#[derive(Template)]
#[template(path = "event_proc.html")]
struct ProcEvent<'a> {
    snapshot: &'a Snapshot,
}

#[derive(Template)]
#[template(path = "event_gpu.html")]
struct GpuEvent<'a> {
    snapshot: &'a Snapshot,
}

/// The five named events, in send order.
const EVENT_NAMES: [&str; 5] = ["cpu", "mem", "net", "proc", "gpu"];

/// The five named events for one snapshot, in send order.
pub fn fragments(snapshot: &Snapshot) -> Vec<(&'static str, String)> {
    EVENT_NAMES
        .into_iter()
        .filter_map(|name| fragment(name, snapshot))
        .collect()
}

/// One named event. An event that fails to build is logged and left out.
fn fragment(name: &'static str, snapshot: &Snapshot) -> Option<(&'static str, String)> {
    let rendered = match name {
        "cpu" => Ok(cpu_frame(snapshot)),
        "mem" => Ok(mem_frame(snapshot)),
        "net" => NetEvent { snapshot }.render(),
        "proc" => ProcEvent { snapshot }.render(),
        "gpu" => GpuEvent { snapshot }.render(),
        _ => return None,
    };
    match rendered {
        Ok(text) => Some((name, text)),
        Err(error) => {
            tracing::error!(event = name, %error, "failed to build panel");
            None
        }
    }
}

/// A record field: the backslash, the pipe and the newline are escaped,
/// because the pipe splits fields and an SSE data line cannot hold a raw
/// newline. The client decodes them in one pass.
fn field(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '|' => escaped.push_str("\\|"),
            '\n' => escaped.push_str("\\n"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// A number the page prints with its unit attached, or empty when unavailable.
fn percent(value: Option<f64>, decimals: usize) -> String {
    value.map_or_else(String::new, |value| format!("{value:.decimals$}%"))
}

/// The CPU event. Fields, in order:
///
/// 0 `c1`, 1 update_ms, 2 window_ms, 3 slow_ms, 4 slow_seq,
/// 5 total display, 6 total band, 7 gauge band, 8 gauge circle path,
/// 9 gauge squircle path, 10 gauge wide `--to`, 11 load reserve, 12 load tip,
/// 13/14/15 load one/five/fifteen figures, 16 user, 17 system, 18 pressure,
/// 19 frequency, 20 temperature, 21 cores label, 22 CPU name, 23 core rows,
/// 24 container-without-cores flag, 25 core count, 26 load-unavailable flag,
/// then per core its name, its `--value` and its printed number, then the
/// history line and area paths.
fn cpu_frame(snapshot: &Snapshot) -> String {
    let cpu = &snapshot.cpu;
    let total = cpu.total_pct;
    let gauge = cpu.gauge_pct;
    let load = cpu.load_readout();
    let mut fields = vec![
        "c1".to_string(),
        snapshot.update_ms.to_string(),
        snapshot.window_ms.to_string(),
        snapshot.slow_ms.to_string(),
        snapshot.slow_seq.to_string(),
        total.map(|pct| format!("{pct:.0}")).unwrap_or_default(),
        total.map_or("", band).to_string(),
        gauge.map_or("", band).to_string(),
        gauge.map_or_else(String::new, |pct| chart::gauge_arc_circle(Some(pct))),
        gauge.map_or_else(String::new, |pct| chart::gauge_arc(Some(pct))),
        gauge.map(|pct| format!("{pct:.2}")).unwrap_or_default(),
    ];
    match &load {
        Some(load) => {
            fields.push(load.reserve.clone());
            fields.push(field(&load.tip));
            fields.push(load.one.figure.clone());
            fields.push(load.five.figure.clone());
            fields.push(load.fifteen.figure.clone());
        }
        None => fields.extend(["", "", "", "", ""].map(String::from)),
    }
    fields.push(percent(cpu.field_pct("user"), 0));
    fields.push(percent(cpu.field_pct("system"), 0));
    fields.push(percent(cpu.pressure_pct, 2));
    fields.push(if cpu.cpu_hz.is_some() || cpu.frequency_mhz.is_some() {
        cpu.frequency_label()
    } else {
        String::new()
    });
    fields.push(
        cpu.temperature_c
            .map(|_| cpu.temperature_label())
            .unwrap_or_default(),
    );
    fields.push(cpu.cores_label().unwrap_or_default());
    fields.push(cpu.name.as_deref().map(field).unwrap_or_default());
    fields.push(cpu.core_rows().to_string());
    fields.push(if snapshot.container && cpu.cores.is_empty() {
        "1".to_string()
    } else {
        String::new()
    });
    fields.push(cpu.cores.len().min(64).to_string());
    fields.push(if snapshot.container && load.is_none() {
        "1".to_string()
    } else {
        String::new()
    });
    for core in cpu.cores.iter().take(64) {
        fields.push(field(&core.name));
        match core.smoothed_pct {
            Some(pct) => {
                fields.push(format!("{pct:.1}"));
                fields.push(format!("{pct:.0}"));
            }
            None => fields.extend([String::new(), String::new()]),
        }
    }
    fields.push(snapshot.cpu.chart.line.clone());
    fields.push(snapshot.cpu.chart.area.clone());
    fields.join("|")
}

/// The memory event. Fields, in order:
///
/// 0 `m1`, 1 slow_seq, 2 used value, 3 used unit, 4 used range, 5 used
/// percent, 6 available percent, 7 gauge label, 8 gauge tip, 9 total label,
/// 10 total suffix, 11 total tip, 12 used fact tip, 13 available fact tip,
/// 14 mount tip, 15 span count, then per span its name, circle path, squircle
/// path, from and to; then the bar segment count and per segment its name and
/// width; then the row count and per row its kind, size, percent, label and
/// tip; then the mount count and per mount its label, IO, total, used meter,
/// used percent, used size, free meter, free percent and free size; then the
/// history line and area paths.
fn mem_frame(snapshot: &Snapshot) -> String {
    let mem = &snapshot.mem;
    let container = snapshot.container;
    let parts = mem.ram_used_parts();
    let mut fields = vec![
        "m1".to_string(),
        snapshot.slow_seq.to_string(),
        parts
            .as_ref()
            .map(|(value, _)| value.clone())
            .unwrap_or_default(),
        parts
            .as_ref()
            .map(|(_, unit)| unit.clone())
            .unwrap_or_default(),
        parts
            .as_ref()
            .map(|_| mem.ram_used_range())
            .unwrap_or_default(),
        percent(mem.ram_percent, 0),
        percent(
            mem.ram_free_reclaimable_percent()
                .or(mem.ram_available_percent),
            0,
        ),
        mem.ram_percent.map(|_| mem.ram_label()).unwrap_or_default(),
        field(&mem.gauge_tip(container)),
        mem.ram_total
            .map_or_else(String::new, |total| bytes(Some(total))),
        if mem.ram_limit.is_some() {
            " (container limit)".to_string()
        } else {
            String::new()
        },
        field(&mem.total_tip(container)),
        field(&mem.used_tip(container)),
        field(&mem.available_tip(container)),
        field(&mount_tip(container)),
    ];
    let spans = mem.gauge_spans();
    fields.push(spans.len().to_string());
    for (name, from, to) in spans {
        fields.push(name.to_string());
        fields.push(chart::gauge_span_circle(from, to));
        fields.push(chart::gauge_span(from, to));
        fields.push(format!("{from:.2}"));
        fields.push(format!("{to:.2}"));
    }
    let segments = mem.ram_segments();
    fields.push(segments.len().to_string());
    for (name, width) in segments {
        fields.push(name.to_string());
        fields.push(format!("{width:.3}"));
    }
    let mut rows: Vec<(&'static str, String, String, &'static str, String)> = vec![
        (
            "used",
            bytes(mem.ram_used),
            percent(mem.ram_percent, 0),
            "used",
            mem.row_tip("used", container),
        ),
        (
            "cache",
            mem.ram_cache_label(),
            percent(mem.ram_cache_percent, 0),
            "reclaimable",
            mem.row_tip("cache", container),
        ),
    ];
    if mem.ram_other.is_some() {
        rows.push((
            "other",
            mem.ram_other_label(),
            percent(mem.ram_other_percent, 0),
            "other used",
            super::OTHER_TIP.to_string(),
        ));
        rows.push((
            "other-cache",
            mem.ram_other_cache_label(),
            percent(mem.ram_other_cache_percent, 0),
            "other reclaimable",
            super::OTHER_CACHE_TIP.to_string(),
        ));
    }
    rows.push((
        "free",
        mem.ram_free_label(),
        percent(mem.ram_free_percent, 0),
        if mem.ram_limit.is_some() {
            "container free"
        } else {
            "free"
        },
        mem.row_tip("free", container),
    ));
    rows.push((
        "free-cache",
        mem.ram_free_reclaimable_label(),
        percent(mem.ram_free_reclaimable_percent(), 0),
        "free + reclaimable",
        mem.row_tip("free-cache", container),
    ));
    if mem.swap_visible() {
        rows.push((
            "swap",
            mem.swap_label(),
            percent(mem.swap_row_percent(), 0),
            "swap",
            mem.swap_tip(container),
        ));
    }
    fields.push(rows.len().to_string());
    for (kind, size, pct, label, tip) in rows {
        fields.push(kind.to_string());
        fields.push(size);
        fields.push(pct);
        fields.push(label.to_string());
        fields.push(field(&tip));
    }
    let mounts: Vec<_> = mem
        .disks
        .iter()
        .filter(|disk| disk.mount != "swap")
        .collect();
    fields.push(mounts.len().to_string());
    for disk in mounts {
        fields.push(field(&disk.mount));
        fields.push(
            disk.io_activity
                .map_or_else(String::new, |io| format!("{io:.0}%")),
        );
        fields.push(
            disk.total
                .map_or_else(String::new, |total| bytes(Some(total))),
        );
        fields.push(
            disk.bar()
                .map(|value| value.to_string())
                .unwrap_or_default(),
        );
        fields.push(percent(disk.used_percent, 0));
        fields.push(bytes(disk.used));
        fields.push(
            super::bar(disk.free_percent)
                .map(|value| value.to_string())
                .unwrap_or_default(),
        );
        fields.push(percent(disk.free_percent, 0));
        fields.push(bytes(disk.free));
    }
    fields.push(mem.used_chart.line.clone());
    fields.push(mem.used_chart.area.clone());
    fields.join("|")
}

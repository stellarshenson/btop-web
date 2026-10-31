//! ACC-VIEW-176-PROC-POPUP: one process's popup body as an HTML fragment. The
//! row comes from the published frame and the rings from the sampler's slow
//! pass ([`proc::detail`]), so a request adds no collection beyond the first
//! read of that pid's files, and no JSON API. A pid the frame does not list is
//! a 404, so a request reaches only what the table shows.

use askama::Template;
use axum::extract::{Path, RawQuery, State};
use axum::response::{Html, IntoResponse, Response};

use crate::AppState;
use crate::chart::{Size, gpu_lane};
use crate::collector::proc::{self, ProcDetail};
use crate::collector::{GpuDevice, ProcEntry, Snapshot, bytes, bytes_short, gpu};
use crate::error::AppError;
use crate::routes::agent_name;

#[derive(Template)]
#[template(path = "partials/proc_detail.html")]
struct ProcDetailTemplate<'a> {
    entry: &'a ProcEntry,
    detail: &'a ProcDetail,
    snapshot: &'a Snapshot,
    /// The devices the process's GPU nodes reach, with their panel index.
    gpus: Vec<(usize, &'a GpuDevice)>,
    /// The viewBox of a device's chart, which the GPU panel's lanes share.
    lane: Size,
    memory: String,
    io_read: String,
    io_write: String,
    /// ACC-VIEW-202-PLOT-LABELS: the CPU and memory plots' scale gutters, top
    /// to bottom, from each plot's own scale frame.
    cpu_scale: [String; 3],
    mem_scale: [String; 3],
}

/// The CPU plot's scale gutter: the shown ceiling, its half and zero, in percent.
fn cpu_scale(label_max: Option<u64>) -> [String; 3] {
    label_max.map_or_else(Default::default, |max| {
        [
            format!("{max}%"),
            format!("{}%", max as f64 / 2.0),
            "0%".to_string(),
        ]
    })
}

/// The memory plot's scale gutter: the shown ceiling, its half and zero, in the
/// table's short byte form.
fn mem_scale(label_max: Option<u64>) -> [String; 3] {
    label_max.map_or_else(Default::default, |max| {
        [
            bytes_short(Some(max)),
            bytes_short(Some(max / 2)),
            "0 B".to_string(),
        ]
    })
}

pub async fn detail(
    State(state): State<AppState>,
    Path(pid): Path<i32>,
    RawQuery(query): RawQuery,
) -> Result<Response, AppError> {
    // ACC-NET-143-AGENT-MODES: a client relays the selected agent's body.
    if let Some(client) = state.client.as_deref() {
        let link = client
            .select(agent_name(query.as_deref()))
            .ok_or(AppError::NotFound)?;
        return match link.detail(pid).await {
            Ok(html) => Ok(Html(html).into_response()),
            Err(error) => {
                tracing::debug!(agent = %link.name, %error, "no process detail from the agent");
                Err(AppError::NotFound)
            }
        };
    }
    let frame = (*state.snapshot.borrow())
        .clone()
        .ok_or(AppError::NotFound)?;
    let snapshot = &frame.snapshot;
    let entry = snapshot
        .proc
        .entries
        .iter()
        .find(|entry| entry.pid == pid)
        .ok_or(AppError::NotFound)?;
    let proc_root = state.config.borrow().proc_root.clone();
    // The first request reads the pid's files, and the collector's lock waits
    // on a running pass.
    let detail = tokio::task::spawn_blocking(move || proc::detail(&proc_root, pid))
        .await
        .ok()
        .flatten()
        .ok_or(AppError::NotFound)?;
    let devices = &snapshot.gpu.devices;
    let gpus = detail
        .gpu_nodes
        .as_deref()
        .map(|nodes| gpu::reached_devices(nodes, devices.len()))
        .unwrap_or_default()
        .into_iter()
        .map(|index| (index, &devices[index]))
        .collect();
    let html = ProcDetailTemplate {
        entry,
        detail: &detail,
        snapshot,
        gpus,
        lane: gpu_lane(devices.len()),
        memory: bytes(entry.mem_bytes),
        io_read: bytes(detail.io_read),
        io_write: bytes(detail.io_write),
        cpu_scale: cpu_scale(detail.cpu.scale.label_max),
        mem_scale: mem_scale(detail.mem.scale.label_max),
    }
    .render()?;
    Ok(Html(html).into_response())
}

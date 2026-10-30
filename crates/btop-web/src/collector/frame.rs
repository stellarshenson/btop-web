//! The published frame: one snapshot plus its five SSE fragments, rendered
//! once by the sampler and shared by every stream.
//!
//! ACC-PERF-16-NO-OVERHEAD: N clients cost no extra collection or rendering.
//! The sampler renders each fragment once per sample and publishes the text
//! behind an `Arc`; a stream sends the held strings, so a viewer adds only its
//! own event framing and socket write.

use askama::Template;

use super::Snapshot;

/// One sample and the five SSE fragments rendered for it.
pub struct Frame {
    pub snapshot: Snapshot,
    /// The named fragments in event order, rendered once by [`Frame::new`].
    pub events: Vec<(&'static str, String)>,
}

impl Frame {
    /// One sample with its five fragments, each rendered once.
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            events: fragments(&snapshot),
            snapshot,
        }
    }
}

#[derive(Template)]
#[template(path = "event_cpu.html")]
struct CpuEvent<'a> {
    snapshot: &'a Snapshot,
}

#[derive(Template)]
#[template(path = "event_mem.html")]
struct MemEvent<'a> {
    snapshot: &'a Snapshot,
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

/// The five named fragments, in event order.
const EVENT_NAMES: [&str; 5] = ["cpu", "mem", "net", "proc", "gpu"];

/// The five named fragments for one snapshot, in event order.
pub fn fragments(snapshot: &Snapshot) -> Vec<(&'static str, String)> {
    EVENT_NAMES
        .into_iter()
        .filter_map(|name| fragment(name, snapshot))
        .collect()
}

/// One named fragment. A fragment that fails to render is logged and left out.
fn fragment(name: &'static str, snapshot: &Snapshot) -> Option<(&'static str, String)> {
    let rendered = match name {
        "cpu" => CpuEvent { snapshot }.render(),
        "mem" => MemEvent { snapshot }.render(),
        "net" => NetEvent { snapshot }.render(),
        "proc" => ProcEvent { snapshot }.render(),
        "gpu" => GpuEvent { snapshot }.render(),
        _ => return None,
    };
    match rendered {
        Ok(html) => Some((name, html)),
        Err(error) => {
            tracing::error!(event = name, %error, "failed to render panel");
            None
        }
    }
}

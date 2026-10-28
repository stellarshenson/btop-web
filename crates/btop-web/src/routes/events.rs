//! Server-sent events: one named event per collector for each snapshot.
//!
//! Each event body is a list of out-of-band wrappers, one per panel the
//! collector feeds (`hx-swap-oob="innerHTML"` on the panel's id), so one event
//! updates panels in different bands. The page's hidden `sse-swap` sinks
//! receive the events and htmx moves each wrapper's content into its panel.

use std::convert::Infallible;

use askama::Template;
use async_stream::stream;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};

use crate::AppState;
use crate::collector::Snapshot;
use crate::error::AppError;

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

pub async fn events(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let mut receiver = state.snapshot.clone();
    let stream = stream! {
        loop {
            let current = (*receiver.borrow_and_update()).clone();
            if let Some(snapshot) = current {
                for (name, html) in fragments(&snapshot) {
                    yield Ok::<_, Infallible>(Event::default().event(name).data(html));
                }
            }
            if receiver.changed().await.is_err() {
                break;
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// The five named fragments for one snapshot, in event order. A fragment
/// that fails to render is logged and left out.
pub fn fragments(snapshot: &Snapshot) -> Vec<(&'static str, String)> {
    [
        ("cpu", CpuEvent { snapshot }.render()),
        ("mem", MemEvent { snapshot }.render()),
        ("net", NetEvent { snapshot }.render()),
        ("proc", ProcEvent { snapshot }.render()),
        ("gpu", GpuEvent { snapshot }.render()),
    ]
    .into_iter()
    .filter_map(|(name, rendered)| match rendered {
        Ok(html) => Some((name, html)),
        Err(error) => {
            tracing::error!(event = name, %error, "failed to render panel");
            None
        }
    })
    .collect()
}

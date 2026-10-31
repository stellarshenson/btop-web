//! Server-sent events: one named event per collector for each published frame.
//!
//! Each event body is a list of out-of-band wrappers, one per panel the
//! collector feeds (`hx-swap-oob="innerHTML"` on the panel's id), so one event
//! updates panels in different bands. The page's hidden `sse-swap` sinks
//! receive the events and htmx moves each wrapper's content into its panel.

use std::convert::Infallible;
use std::time::Duration;

use async_stream::stream;
use axum::extract::{RawQuery, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::error::AppError;
use crate::routes::agent_name;

/// ACC-PERF-136-SLEEP: subscribing wakes a parked sampler. ACC-PERF-137-
/// WAKE-BURST: the held frame goes out at once for every panel, ring
/// window included; only a frame older than one interval makes the stream
/// wait, at most that interval, for the post-wake sample, so no viewer sees a
/// stale age from before the sleep. ACC-PERF-149-SPLIT-CADENCE: the process
/// fragment is sent only when a new slow pass produced it, while the four
/// value events keep the tick; the first frame of a stream always carries
/// the held slow result. ACC-PERF-16-NO-OVERHEAD: the sampler rendered the
/// fragments once for every viewer, so a stream clones a pointer and yields
/// the held text. DEF-VIEW-119-STALE-TAB: every connection opens with one
/// `stamp` event naming this build, so a page from another build reloads; no
/// frame carries it.
pub async fn events(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
) -> Result<Response, AppError> {
    // ACC-NET-143-AGENT-MODES: a client relays the selected agent's fragments
    // unchanged; the five event names and their HTML are the agent's own.
    if let Some(client) = state.client.as_deref() {
        let link = client
            .select(agent_name(query.as_deref()))
            .ok_or(AppError::NotFound)?;
        return Ok(link.relay(state.asset_stamp.clone()).into_response());
    }
    let subscription = state.wake.subscribe();
    let interval = Duration::from_millis(state.config.borrow().update_ms.max(1));
    let mut receiver = state.snapshot.clone();
    // The state's receiver never marks its value seen, so a clone must, or
    // `changed` would report an old sample as new.
    receiver.borrow_and_update();
    if state.wake.stale(interval) {
        let _ = tokio::time::timeout(interval, receiver.changed()).await;
    }
    let stamp = state.asset_stamp.clone();
    let stream = stream! {
        let _subscription = subscription;
        yield Ok::<_, Infallible>(Event::default().event("stamp").data(&*stamp));
        let mut slow_seq = None;
        loop {
            let frame = (*receiver.borrow_and_update()).clone();
            if let Some(frame) = frame {
                let slow = slow_seq != Some(frame.snapshot.slow_seq);
                for (name, html) in &frame.events {
                    if *name == "proc" && !slow {
                        continue;
                    }
                    yield Ok(Event::default().event(*name).data(html));
                }
                slow_seq = Some(frame.snapshot.slow_seq);
            }
            if receiver.changed().await.is_err() {
                break;
            }
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

//! ACC-NET-143-AGENT-MODES: client mode. The process samples nothing and
//! renders the dashboard from the agents named in its configuration.
//!
//! One upstream SSE connection per configured agent, opened at startup and
//! shared by every browser (ACC-PERF-16-NO-OVERHEAD): N viewers cost one
//! stream and one collection per agent tick, never one per viewer. The relay
//! forwards each event in arrival order and parses nothing inside a fragment
//! but the advertised interval - the same `data-update-ms` attribute
//! `assets/app.js` reads - so each agent's sample age and stale mark are
//! truthful.
//!
//! ACC-NET-144-ACCESS-KEY: each agent demands its key on every request, so
//! the client reads one key file per agent from the configuration and sends
//! it as `Authorization: Bearer KEY`; an unreadable file stops the process.
//!
//! ACC-NET-145-TLS-PINNING: the upstream connection is TLS 1.3, and the
//! certificate fingerprint the first handshake presents is written to the pin
//! file before the first request. Every later connection presents that pin to
//! the verifier, and any other certificate stops the connection with both
//! fingerprints named.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use hyper::header::{ACCEPT, AUTHORIZATION, HOST};
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::sync::broadcast;

use crate::config::{Connect, ConnectKey, DEFAULT_UPDATE_MS};
use crate::secret;
use crate::tls::{self, Pins};

/// How long the client waits before it retries an agent whose stream failed
/// or ended.
const RETRY: Duration = Duration::from_secs(1);

/// The longest one connect, handshake, first pin write and request may take
/// together, so a link that blackholes a handshake fails and retries instead
/// of waiting for the kernel's retransmit budget.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest a stream may stay silent. The agent keep-alives every 15 s, so
/// a link silent past this is treated as a stream end and retried.
const READ_TIMEOUT: Duration = Duration::from_secs(45);

/// Fragments held for a browser that reads slower than the agent sends. A
/// viewer that falls further behind skips to the next frame, because every
/// fragment overwrites its panel whole.
const BACKLOG: usize = 64;

/// ACC-NET-143-AGENT-MODES: the configured agents and their live streams.
pub struct Client {
    /// Every configured agent, in page order; the first is the default
    /// selection.
    pub links: Vec<Link>,
}

impl Client {
    /// Start one upstream task per configured agent, each with the key source
    /// its `connect_key` line names. Every source is read before any task
    /// starts, so an unreadable key or descriptor stops the process instead
    /// of leaving a half-connected client running. The pin file is read the
    /// same way: an unreadable or malformed one stops the client rather than
    /// re-pinning every agent.
    pub fn start(connects: &[Connect], keys: &[ConnectKey], pins_path: &Path) -> Result<Self> {
        let pins = Arc::new(Pins::load(pins_path)?);
        let mut resolved = Vec::with_capacity(connects.len());
        for connect in connects {
            let entry = keys
                .iter()
                .find(|key| key.name == connect.name)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "agent `{}` has no access key; add `connect_key = {}=PATH` to the configuration",
                        connect.name,
                        connect.name
                    )
                })?;
            // ACC-NET-146-SECRET-INPUT: a file or an inherited descriptor,
            // read once here; the source never holds the key itself.
            let bytes = secret::read(
                &entry.source,
                &format!("agent `{}` access key", connect.name),
            )?;
            let key = std::str::from_utf8(&bytes).with_context(|| {
                format!(
                    "agent `{}`: access key from {} is not UTF-8 text",
                    connect.name,
                    entry.source.describe()
                )
            })?;
            let key = key.trim();
            anyhow::ensure!(
                !key.is_empty(),
                "agent `{}`: access key from {} is empty",
                connect.name,
                entry.source.describe()
            );
            resolved.push((connect.name.clone(), connect.addr.clone(), key.to_string()));
        }
        let links = resolved
            .into_iter()
            .map(|(name, addr, key)| {
                let shared = Arc::new(Shared::new());
                let state = Arc::clone(&shared);
                let upstream_pins = Arc::clone(&pins);
                let upstream_name = name.clone();
                let (upstream_addr, upstream_key) = (addr.clone(), key.clone());
                tokio::spawn(async move {
                    upstream(
                        upstream_name,
                        upstream_addr,
                        upstream_key,
                        upstream_pins,
                        state,
                    )
                    .await;
                });
                Link {
                    name,
                    shared,
                    addr,
                    key,
                    pins: Arc::clone(&pins),
                }
            })
            .collect();
        Ok(Self { links })
    }

    /// The agent a request selects: the named one, or the first configured
    /// agent when the request names none.
    pub fn select(&self, name: Option<&str>) -> Option<&Link> {
        match name {
            Some(name) => self.links.iter().find(|link| link.name == name),
            None => self.links.first(),
        }
    }

    /// One chip per agent for the top-bar strip, the selected one marked.
    pub fn chips(&self, selected: &str) -> Vec<Chip> {
        self.links
            .iter()
            .map(|link| link.chip(link.name == selected))
            .collect()
    }
}

/// One configured agent and the state of its stream.
pub struct Link {
    /// The page's label for the agent.
    pub name: String,
    shared: Arc<Shared>,
    addr: String,
    key: String,
    pins: Arc<Pins>,
}

impl Link {
    /// ACC-VIEW-176-PROC-POPUP: the agent's popup body for `pid`, unchanged. An
    /// open popup asks once per slow pass, over one connection per request; the
    /// agent's sampler holds the rings.
    pub async fn detail(&self, pid: i32) -> Result<String> {
        tokio::time::timeout(CONNECT_TIMEOUT, async {
            let path = format!("/proc/{pid}");
            let response = get(
                &self.name,
                &self.addr,
                &self.key,
                &self.pins,
                &path,
                "text/html",
            )
            .await?;
            let body = response.into_body().collect().await?.to_bytes();
            anyhow::Ok(String::from_utf8_lossy(&body).into_owned())
        })
        .await
        .context("agent did not answer within 10 s")?
    }

    /// Whether the agent's stream is up; the strip chip reads it.
    pub fn connected(&self) -> bool {
        self.shared.view().connected
    }

    /// Relay this agent's fragments to one browser, in arrival order, after
    /// the stamp of the build that served the browser its page
    /// (DEF-VIEW-119-STALE-TAB).
    pub fn relay(&self, stamp: Arc<str>) -> impl IntoResponse {
        let mut receiver = self.shared.events.subscribe();
        let stream = async_stream::stream! {
            yield Ok::<_, std::convert::Infallible>(Event::default().event("stamp").data(&*stamp));
            loop {
                match receiver.recv().await {
                    Ok(fragment) => {
                        yield Ok(Event::default().event(fragment.name).data(fragment.data));
                    }
                    // A lagging viewer skips to the next frame: every
                    // fragment replaces its panel whole.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        };
        Sse::new(stream).keep_alive(KeepAlive::default())
    }

    /// The agent's chip: its name, its sample age and its stream state.
    fn chip(&self, selected: bool) -> Chip {
        let view = self.shared.view();
        let (state, text) = match (view.connected, view.last_at) {
            // No invented data: an agent that never answered, or stopped
            // answering, shows that word instead of an age or a zero.
            (false, _) => ("unavailable", "unavailable".to_string()),
            (true, None) => ("connecting", "connecting".to_string()),
            (true, Some(at)) => {
                let age = at.elapsed();
                let stale = age.as_millis() as u64 > 2 * view.interval_ms;
                (if stale { "stale" } else { "live" }, age_text(age))
            }
        };
        Chip {
            name: self.name.clone(),
            text,
            state,
            selected,
        }
    }
}

/// One agent in the top-bar strip, rendered by `partials/agents.html`.
pub struct Chip {
    pub name: String,
    /// The sample age, or the state word while there is no frame.
    pub text: String,
    /// `live`, `stale`, `connecting` or `unavailable`; also the CSS class.
    pub state: &'static str,
    pub selected: bool,
}

/// One named fragment from an agent, as received.
#[derive(Clone)]
struct Fragment {
    name: String,
    data: String,
}

#[derive(Clone, Copy)]
struct View {
    connected: bool,
    last_at: Option<Instant>,
    /// The agent's advertised sampling interval, learned from its cpu
    /// fragment; `DEFAULT_UPDATE_MS` until the first frame.
    interval_ms: u64,
}

/// The broadcast of one agent's fragments and the state its strip chip reads.
struct Shared {
    events: broadcast::Sender<Fragment>,
    view: Mutex<View>,
}

impl Shared {
    fn new() -> Self {
        let (events, _) = broadcast::channel(BACKLOG);
        Self {
            events,
            view: Mutex::new(View {
                connected: false,
                last_at: None,
                interval_ms: DEFAULT_UPDATE_MS,
            }),
        }
    }

    fn view(&self) -> View {
        *self.view.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn set_connected(&self, connected: bool) {
        self.view
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .connected = connected;
    }

    fn publish(&self, fragment: Fragment) {
        // DEF-VIEW-119-STALE-TAB: the agent's stamp names the agent's build, not
        // the page's, and it is not a sample.
        if fragment.name == "stamp" {
            return;
        }
        {
            let mut view = self.view.lock().unwrap_or_else(|error| error.into_inner());
            view.connected = true;
            view.last_at = Some(Instant::now());
            if fragment.name == "cpu"
                && let Some(interval) = advertised_interval(&fragment.data)
            {
                view.interval_ms = interval;
            }
        }
        let _ = self.events.send(fragment);
    }
}

/// One agent's upstream: connect, relay until the stream ends, retry. Losing
/// an established stream is a warning; a retry that fails again is debug, so
/// a stopped agent does not write one log line per second.
async fn upstream(name: String, addr: String, key: String, pins: Arc<Pins>, shared: Arc<Shared>) {
    loop {
        match follow(&name, &addr, &key, Arc::clone(&pins), &shared).await {
            // An established stream that ends or fails is a warning; a
            // connect that never established is debug, so a stopped agent
            // does not write one log line per second.
            Err(error) if shared.view().connected => {
                tracing::warn!(agent = %name, %error, "agent stream ended");
            }
            Err(error) => tracing::debug!(agent = %name, %error, "agent unreachable"),
            Ok(()) => tracing::warn!(agent = %name, "agent stream ended"),
        }
        shared.set_connected(false);
        tokio::time::sleep(RETRY).await;
    }
}

/// One connection to an agent and one GET of `path`; an answer other than
/// success is an error.
async fn get(
    name: &str,
    addr: &str,
    key: &str,
    pins: &Arc<Pins>,
    path: &str,
    accept: &str,
) -> Result<Response<Incoming>> {
    let pin = pins.get(name);
    let (stream, presented) = tls::connect(addr, name, pin.as_deref()).await?;
    // ACC-NET-145-TLS-PINNING: the first fingerprint is written
    // before the first request; a pin that cannot be written fails
    // the connection instead of trusting an agent the client cannot
    // remember. The write syncs a file, so it runs off the async
    // workers.
    if pin.is_none() {
        let presented = presented.context("the handshake presented no certificate")?;
        let writer = Arc::clone(pins);
        let agent = name.to_string();
        tokio::task::spawn_blocking(move || writer.record(&agent, &presented))
            .await
            .context("the pin writer task panicked")??;
    }
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .context("HTTP handshake")?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let request = Request::builder()
        .uri(format!("http://{addr}{path}"))
        .header(HOST, addr)
        .header(ACCEPT, accept)
        // ACC-NET-144-ACCESS-KEY: the key never appears in the URL,
        // where a log or a proxy would keep it.
        .header(AUTHORIZATION, format!("Bearer {key}"))
        .body(Empty::<Bytes>::new())?;
    let response = sender.send_request(request).await?;
    anyhow::ensure!(
        response.status().is_success(),
        "agent answered {}",
        response.status()
    );
    Ok(response)
}

async fn follow(name: &str, addr: &str, key: &str, pins: Arc<Pins>, shared: &Shared) -> Result<()> {
    // ACC-PERF-136-SLEEP is not the only bound: a link that blackholes a
    // connect, handshake or request now fails within one deadline and
    // retries, instead of waiting out the kernel's retransmit budget.
    let response = tokio::time::timeout(
        CONNECT_TIMEOUT,
        get(name, addr, key, &pins, "/events", "text/event-stream"),
    )
    .await
    .context("agent did not answer within 10 s")??;
    shared.set_connected(true);
    let mut body = response.into_body();
    let mut decoder = Decoder::default();
    loop {
        // The agent keep-alives every 15 s; a silent link is a dead one.
        let Some(frame) = tokio::time::timeout(READ_TIMEOUT, body.frame())
            .await
            .context("agent stream silent for 45 s")?
        else {
            break;
        };
        if let Some(chunk) = frame?.data_ref() {
            for fragment in decoder.push(chunk) {
                shared.publish(fragment);
            }
        }
    }
    Ok(())
}

/// An SSE record decoder for the agent's two fields, `event:` and one or more
/// `data:` lines; keep-alive comments are ignored.
#[derive(Default)]
struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    fn push(&mut self, chunk: &[u8]) -> Vec<Fragment> {
        self.buffer.extend_from_slice(chunk);
        let mut fragments = Vec::new();
        while let Some((end, skip)) = record_end(&self.buffer) {
            let record: Vec<u8> = self.buffer.drain(..end + skip).collect();
            if let Some(fragment) = parse_record(&record[..end]) {
                fragments.push(fragment);
            }
        }
        fragments
    }
}

/// The first blank line ends a record: `\n\n`, or `\r\n\r\n` from a peer that
/// writes CRLF.
fn record_end(buffer: &[u8]) -> Option<(usize, usize)> {
    (0..buffer.len()).find_map(|index| {
        if buffer[index..].starts_with(b"\r\n\r\n") {
            Some((index, 4))
        } else if buffer[index..].starts_with(b"\n\n") {
            Some((index, 2))
        } else {
            None
        }
    })
}

fn parse_record(record: &[u8]) -> Option<Fragment> {
    let text = String::from_utf8_lossy(record);
    let mut name = String::new();
    let mut data: Option<String> = None;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            name = value.strip_prefix(' ').unwrap_or(value).to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            let value = value.strip_prefix(' ').unwrap_or(value);
            match &mut data {
                None => data = Some(value.to_string()),
                Some(data) => {
                    data.push('\n');
                    data.push_str(value);
                }
            }
        }
    }
    data.filter(|_| !name.is_empty())
        .map(|data| Fragment { name, data })
}

/// The interval the cpu fragment advertises, the same `data-update-ms`
/// attribute `assets/app.js` reads; `None` when the fragment carries none.
fn advertised_interval(data: &str) -> Option<u64> {
    let value = data.split_once("data-update-ms=\"")?.1;
    value.split_once('"')?.0.parse().ok()
}

/// The sample age as the strip shows it: one decimal under ten seconds.
fn age_text(age: Duration) -> String {
    let seconds = age.as_secs_f64();
    if seconds < 10.0 {
        format!("{seconds:.1} s")
    } else {
        format!("{seconds:.0} s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decoder splits on the blank line the agent writes, joins data
    /// lines, ignores keep-alive comments and keeps a fragment that arrives
    /// in two chunks.
    #[test]
    fn decoder_reassembles_fragments() {
        let mut decoder = Decoder::default();
        assert!(
            decoder
                .push(b": keep-alive\n\nevent: cpu\ndata: <div>\n")
                .is_empty()
        );
        let fragments = decoder.push(b"data: </div>\n\nevent: mem\ndata: x\n\n");
        assert_eq!(fragments.len(), 2);
        assert_eq!(fragments[0].name, "cpu");
        assert_eq!(fragments[0].data, "<div>\n</div>");
        assert_eq!(fragments[1].name, "mem");
        assert_eq!(fragments[1].data, "x");
    }

    /// DEF-VIEW-119-STALE-TAB: an agent's reconnect while a page is open must
    /// not put the agent's stamp in front of that page, or a client and an
    /// agent of different builds would reload the page on every reconnect.
    #[test]
    fn an_agents_stamp_reaches_no_viewer() {
        let shared = Shared::new();
        let mut viewer = shared.events.subscribe();
        for (name, data) in [("stamp", "another-build"), ("cpu", "<div>")] {
            shared.publish(Fragment {
                name: name.to_string(),
                data: data.to_string(),
            });
        }
        assert_eq!(viewer.try_recv().expect("the cpu fragment").name, "cpu");
        assert!(viewer.try_recv().is_err());
    }

    /// ACC-PERF-149-SPLIT-CADENCE: the advertised interval comes from the
    /// fragment the page already reads it from.
    #[test]
    fn advertised_interval_is_read() {
        assert_eq!(
            advertised_interval(r#"<span data-update-ms="2000" data-slow-ms="2000">"#),
            Some(2000)
        );
        assert_eq!(advertised_interval("<div></div>"), None);
    }

    #[test]
    fn ages_are_formatted_for_the_strip() {
        assert_eq!(age_text(Duration::from_millis(230)), "0.2 s");
        assert_eq!(age_text(Duration::from_secs(12)), "12 s");
    }

    /// ACC-NET-144-ACCESS-KEY: a client with no key for an agent, or with a
    /// key file that cannot be read, stops before any stream starts and names
    /// the agent and the file.
    #[tokio::test]
    async fn the_client_refuses_to_start_without_a_readable_key() {
        let connects = [Connect {
            name: "nas".to_string(),
            addr: "127.0.0.1:1".to_string(),
        }];
        let pins = Path::new("/nonexistent/btop-web-known-agents");
        let missing_line = Client::start(&connects, &[], pins)
            .err()
            .expect("a client agent with no key line");
        let message = missing_line.to_string();
        assert!(message.contains("nas"), "{message}");
        assert!(message.contains("connect_key"), "{message}");

        let keys = [ConnectKey {
            name: "nas".to_string(),
            source: crate::secret::Source::Path("/nonexistent/btop-web-access-key".into()),
        }];
        let unreadable = Client::start(&connects, &keys, pins)
            .err()
            .expect("an unreadable key file");
        let message = format!("{unreadable:#}");
        assert!(message.contains("nas"), "{message}");
        assert!(
            message.contains("/nonexistent/btop-web-access-key"),
            "{message}"
        );
    }
}

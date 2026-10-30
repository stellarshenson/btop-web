//! DEF-PERF-76-SERVER-COST: the process walk's output must not change when
//! its reads get cheaper, so the proc fragment is byte-compared against a
//! fragment recorded before the cache work.
//!
//! The recording helper is kept behind `#[ignore]`: run it only to replace the
//! golden after a deliberate output change.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use btop_web::AppState;
use btop_web::collector::frame::fragments;
use btop_web::collector::{self, Sampler, Snapshot, proc};
use btop_web::config::Config;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::timeout;

// shortcut: the golden holds the host page size's MemB text, so a different
// page size fails and re-records; upgrade when a non-4 KiB host runs the suite.
const GOLDEN: &str = "tests/fixtures/perf/proc-fragment.html";

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN)
}

/// A private copy of the recorded tree, so the shared `LAST` state of the
/// fixture path cannot mix this test's ticks with another test's.
fn temp_tree(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("btop-web-perf-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    copy_tree(&fixture("proc"), &root);
    root
}

fn copy_tree(source: &Path, target: &Path) {
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.path().is_dir() {
            fs::create_dir_all(&destination).unwrap();
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), &destination).unwrap();
        }
    }
}

/// The proc SSE fragment after `ticks` samples of `root`.
fn proc_fragment(root: &Path, ticks: usize) -> String {
    let config = Config::load(None).expect("embedded defaults");
    let mut snapshot = Snapshot::default();
    for _ in 0..ticks {
        snapshot.proc = proc::collect(root, &config);
        snapshot.update_ms = config.update_ms;
        snapshot.window_ms = btop_web::chart::window(config.window_ms(), config.update_ms);
    }
    fragments(&snapshot)
        .into_iter()
        .find(|(name, _)| *name == "proc")
        .expect("proc fragment")
        .1
}

#[test]
fn proc_fragment_matches_the_recorded_fragment() {
    let root = temp_tree("fragment");
    let fragment = proc_fragment(&root, 3);
    let _ = fs::remove_dir_all(&root);
    let golden = fs::read_to_string(golden()).expect("golden fragment");
    assert_eq!(fragment, golden, "the proc fragment's bytes changed");
}

/// Re-record the golden; run with `cargo test --test perf -- --ignored`.
#[test]
#[ignore]
fn record_proc_fragment() {
    let root = temp_tree("fragment");
    let fragment = proc_fragment(&root, 3);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(golden().parent().expect("fixtures/perf")).unwrap();
    fs::write(golden(), fragment).unwrap();
}

/* ---------- ACC-PERF-136-SLEEP and ACC-PERF-137-WAKE-BURST ---------- */

/// A sampler over a private recorded tree with its sleep gate attached to an
/// app served on a loopback port, so the tests drive the real socket path.
struct Guarded {
    sampler: Sampler,
    address: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    tree: PathBuf,
}

impl Drop for Guarded {
    fn drop(&mut self) {
        self.server.abort();
        self.sampler.task.abort();
        let _ = fs::remove_dir_all(&self.tree);
    }
}

async fn guarded(name: &str, update_ms: u64) -> Guarded {
    let tree = temp_tree(name);
    let mut config = Config::load(None).expect("embedded defaults");
    config.proc_root = tree.clone();
    config.update_ms = update_ms;
    let (sender, receiver) = watch::channel(config);
    let sampler = collector::spawn(receiver);
    let mut state = AppState::new(sampler.snapshots.clone(), true);
    state.wake = Arc::clone(&sampler.wake);
    state.config = sender;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("address");
    let app = btop_web::app(state);
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Guarded {
        sampler,
        address,
        server,
        tree,
    }
}

/// The first SSE event block named `kind` on a fresh `/events` connection,
/// with the time from writing the request to the complete block.
async fn first_event(address: SocketAddr, kind: &str) -> (Duration, String) {
    let started = Instant::now();
    let mut stream = TcpStream::connect(address).await.expect("connect");
    let request =
        format!("GET /events HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("request");
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    let marker = format!("event: {kind}");
    loop {
        let read = timeout(Duration::from_secs(10), stream.read(&mut chunk))
            .await
            .expect("a frame within 10 s")
            .expect("read");
        assert!(read > 0, "the stream ended before a {kind} frame");
        response.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&response);
        let Some((_, body)) = text.split_once("\r\n\r\n") else {
            continue;
        };
        let Some(start) = body.find(&marker) else {
            continue;
        };
        if let Some(end) = body[start..].find("\n\n") {
            return (started.elapsed(), body[start..start + end].to_string());
        }
    }
}

async fn get_page(address: SocketAddr) -> String {
    let mut stream = TcpStream::connect(address).await.expect("connect");
    let request = format!("GET / HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("request");
    let mut response = Vec::new();
    timeout(Duration::from_secs(10), stream.read_to_end(&mut response))
        .await
        .expect("the page within 10 s")
        .expect("read");
    String::from_utf8_lossy(&response).into_owned()
}

/// Add `busy` jiffies to the aggregate `cpu` line's user field, so the next
/// sample computes a CPU total of 100 percent against the previous one.
fn add_busy_jiffies(stat: &Path, busy: u64) {
    let text = fs::read_to_string(stat).expect("recorded stat");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let line = lines
        .iter_mut()
        .find(|line| line.starts_with("cpu "))
        .expect("aggregate cpu line");
    let mut fields: Vec<String> = line.split_whitespace().map(str::to_string).collect();
    let user: u64 = fields[1].parse().expect("user jiffies");
    fields[1] = (user + busy).to_string();
    *line = fields.join(" ");
    // The recordings are read-only; replace the file instead of writing it.
    let replaced = stat.with_extension("busy");
    fs::write(&replaced, lines.join("\n") + "\n").expect("write stat");
    fs::rename(&replaced, stat).expect("replace stat");
}

/// ACC-PERF-136-SLEEP: with nobody subscribed and no page request, the sampler
/// emits nothing; a request wakes it to an immediate sample, then it parks.
#[tokio::test]
async fn sampler_parks_with_nobody_connected_and_wakes_for_a_request() {
    let mut guarded = guarded("park", 100).await;
    assert!(
        timeout(
            Duration::from_millis(500),
            guarded.sampler.snapshots.changed()
        )
        .await
        .is_err(),
        "the sampler ticked with nobody connected"
    );
    let at = Instant::now();
    guarded.sampler.wake.request();
    timeout(
        Duration::from_millis(500),
        guarded.sampler.snapshots.changed(),
    )
    .await
    .expect("a page request wakes the sampler")
    .expect("snapshot");
    assert!(
        at.elapsed() < Duration::from_millis(250),
        "the wake sample took {:?}",
        at.elapsed()
    );
    guarded.sampler.snapshots.borrow_and_update();
    assert!(
        timeout(
            Duration::from_millis(500),
            guarded.sampler.snapshots.changed()
        )
        .await
        .is_err(),
        "the sampler kept ticking after the request was served"
    );
}

/// ACC-PERF-136-SLEEP: an open stream holds the sampler awake at its interval;
/// the stream's end parks it, so no timer remains.
#[tokio::test]
async fn a_stream_keeps_the_sampler_awake_and_its_end_parks_it() {
    let guarded = guarded("stream", 100).await;
    let stream = guarded.sampler.wake.subscribe();
    let mut snapshots = guarded.sampler.snapshots.clone();
    timeout(Duration::from_millis(500), snapshots.changed())
        .await
        .expect("the first sample")
        .expect("snapshot");
    let at = Instant::now();
    for _ in 0..3 {
        timeout(Duration::from_millis(500), snapshots.changed())
            .await
            .expect("a sample while the stream is open")
            .expect("snapshot");
    }
    // Three intervals of samples cannot arrive in less than 250 ms.
    assert!(
        at.elapsed() >= Duration::from_millis(250),
        "the cadence ran ahead of the interval: {:?}",
        at.elapsed()
    );
    drop(stream);
    // Let the tick already due land, then require silence.
    tokio::time::sleep(Duration::from_millis(300)).await;
    snapshots.borrow_and_update();
    assert!(
        timeout(Duration::from_millis(500), snapshots.changed())
            .await
            .is_err(),
        "the sampler kept its timer after the last stream ended"
    );
}

/// Hold one `/events` stream until it has delivered `count` cpu events.
async fn hold_cpu_events(address: SocketAddr, count: usize) {
    let mut stream = TcpStream::connect(address).await.expect("connect");
    let request =
        format!("GET /events HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("request");
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = timeout(Duration::from_secs(10), stream.read(&mut chunk))
            .await
            .expect("frames within 10 s")
            .expect("read");
        assert!(read > 0, "the stream ended before {count} cpu events");
        response.extend_from_slice(&chunk[..read]);
        if String::from_utf8_lossy(&response)
            .matches("event: cpu")
            .count()
            >= count
        {
            return;
        }
    }
}

/// ACC-PERF-149-SPLIT-CADENCE: on the wire, the value events keep the tick
/// while the process event arrives once per slow pass, and the first frame
/// still carries every panel (ACC-PERF-137-WAKE-BURST).
#[tokio::test]
async fn proc_events_follow_the_slow_pass_while_cpu_events_keep_the_tick() {
    let guarded = guarded("split-events", 100).await;
    let address = guarded.address;
    let mut stream = TcpStream::connect(address).await.expect("connect");
    let request =
        format!("GET /events HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("request");
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    let deadline = Instant::now() + Duration::from_millis(1_300);
    while Instant::now() < deadline {
        let Ok(Ok(read)) = timeout(Duration::from_millis(200), stream.read(&mut chunk)).await
        else {
            continue;
        };
        if read == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..read]);
    }
    let text = String::from_utf8_lossy(&response);
    let cpu = text.matches("event: cpu").count();
    let proc = text.matches("event: proc").count();
    assert!(
        text.contains("event: proc"),
        "the first frame carried no process panel"
    );
    assert!(cpu >= 8, "{cpu} cpu events in 1.3 s at 100 ms");
    assert!(
        (2..=3).contains(&proc),
        "{proc} proc events in 1.3 s at a 1000 ms slow pass"
    );
    assert!(
        cpu >= 4 * proc,
        "{cpu} cpu events for {proc} proc events: the values do not run faster"
    );
}

/// ACC-PERF-137-WAKE-BURST and ACC-PERF-136-SLEEP: a subscriber gets its
/// first frame at once with the held ring, and after a sleep the first frame
/// is the post-wake sample, not the pre-sleep one, inside one interval.
#[tokio::test]
async fn first_frame_after_a_sleep_carries_the_post_wake_sample() {
    let guarded = guarded("first-frame", 250).await;
    // The recorded counters do not move, so the first sample is a zero delta.
    let (elapsed, block) = first_event(guarded.address, "cpu").await;
    assert!(block.contains(r#"id="cpu-total">0<"#), "{block}");
    assert!(
        elapsed < Duration::from_millis(250),
        "the first frame took {elapsed:?}, past one interval"
    );
    // Four samples fill the ring; the stream then ends and the sampler parks.
    hold_cpu_events(guarded.address, 4).await;
    tokio::time::sleep(Duration::from_millis(750)).await;
    add_busy_jiffies(&guarded.tree.join("stat"), 1000);
    let (elapsed, block) = first_event(guarded.address, "cpu").await;
    assert!(
        block.contains(r#"id="cpu-total">100<"#),
        "the first frame carried the pre-sleep sample: {block}"
    );
    assert!(
        block.contains(r#"<path class="line" d="M"#),
        "the ring did not ride in the first frame: {block}"
    );
    assert!(
        elapsed < Duration::from_millis(250),
        "the wake frame took {elapsed:?}, past one interval"
    );
}

/// ACC-PERF-136-SLEEP: a page request after a sleep waits for the wake sample,
/// so the server-rendered values are current.
#[tokio::test]
async fn page_after_a_sleep_waits_for_the_first_sample() {
    let guarded = guarded("page", 250).await;
    let page = get_page(guarded.address).await;
    assert!(page.contains(r#"id="cpu-total">0<"#), "{page}");
    tokio::time::sleep(Duration::from_millis(750)).await;
    add_busy_jiffies(&guarded.tree.join("stat"), 1000);
    let at = Instant::now();
    let page = get_page(guarded.address).await;
    assert!(
        page.contains(r#"id="cpu-total">100<"#),
        "the page carried the pre-sleep sample: {page}"
    );
    assert!(
        at.elapsed() < Duration::from_millis(2000),
        "the page took {:?}",
        at.elapsed()
    );
}

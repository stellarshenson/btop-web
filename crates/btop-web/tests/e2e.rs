//! End-to-end: boot the real binary against the recorded `/proc` and `/sys`
//! trees and read the page and one SSE frame over a socket.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

/// btop's since-boot CPU total, computed from the recorded `/proc/stat`.
fn fixture_cpu_total() -> String {
    let stat = std::fs::read_to_string(fixture("proc/stat")).expect("recorded stat");
    let line = stat
        .lines()
        .find(|line| line.starts_with("cpu "))
        .expect("aggregate cpu line");
    let values: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(10)
        .filter_map(|value| value.parse().ok())
        .collect();
    let total: u64 = values.iter().sum::<u64>() - values.iter().skip(8).sum::<u64>();
    let idles = values[3] + values.get(4).copied().unwrap_or(0);
    let percent = (total.saturating_sub(idles) as f64 * 100.0 / total as f64).round();
    format!("{percent:.0}")
}

/// Start the binary on a free loopback port against the recorded tree.
fn start(update_ms: Option<&str>, args: &[&str]) -> (Server, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("free port");
    let address = listener.local_addr().expect("address");
    drop(listener);
    let mut command = Command::new(env!("CARGO_BIN_EXE_btop-web"));
    command
        .args(args)
        .env("BTOP_WEB_LISTEN", address.to_string())
        .env("BTOP_WEB_PROC_ROOT", fixture("proc"))
        .env_remove("BTOP_WEB_UPDATE_MS")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(update_ms) = update_ms {
        command.env("BTOP_WEB_UPDATE_MS", update_ms);
    }
    (Server(command.spawn().expect("spawn btop-web")), address)
}

/// The exit code, stdout and stderr of the binary run with `args`; it must
/// exit within 10 s.
fn run(args: &[&str]) -> (Option<i32>, String, String) {
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_btop-web"))
            .args(args)
            .env("BTOP_WEB_LISTEN", "127.0.0.1:0")
            .env("BTOP_WEB_PROC_ROOT", fixture("proc"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn btop-web"),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = server.0.try_wait().expect("wait") {
            break status;
        }
        assert!(Instant::now() < deadline, "{args:?} did not exit in 10 s");
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = server.0.stdout.take() {
        pipe.read_to_string(&mut stdout).expect("stdout");
    }
    if let Some(mut pipe) = server.0.stderr.take() {
        pipe.read_to_string(&mut stderr).expect("stderr");
    }
    (status.code(), stdout, stderr)
}

/// The page once it carries `marker`, within 20 s.
fn page_with(address: SocketAddr, marker: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(page) = get(address, "/")
            && page.starts_with("HTTP/1.1 200")
            && page.contains(marker)
        {
            return page;
        }
        assert!(
            Instant::now() < deadline,
            "the binary did not serve {marker} in 20 s"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn get(address: SocketAddr, path: &str) -> Option<String> {
    exchange(
        address,
        &format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"),
    )
}

/// One request written as is, and the whole response.
fn exchange(address: SocketAddr, request: &str) -> Option<String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => response.extend_from_slice(&buffer[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
    Some(String::from_utf8_lossy(&response).into_owned())
}

fn read_event_stream(address: SocketAddr) -> String {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    write!(
        stream,
        "GET /events HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\n\r\n"
    )
    .expect("request");
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8192];
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                response.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&response);
                if text.contains("event: cpu") && text.contains("data: ") && text.contains("<h2") {
                    return text.into_owned();
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    panic!(
        "no SSE frame in 15 s; received: {}",
        String::from_utf8_lossy(&response)
    );
}

/// The number of `cpu` events the stream carries in `window` after its first.
fn cpu_events_in(address: SocketAddr, window: Duration) -> usize {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("timeout");
    write!(
        stream,
        "GET /events HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\n\r\n"
    )
    .expect("request");
    let mut response = Vec::new();
    let mut buffer = [0_u8; 65536];
    let mut first: Option<(Instant, usize)> = None;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(read) = stream.read(&mut buffer) {
            if read == 0 {
                break;
            }
            response.extend_from_slice(&buffer[..read]);
        }
        let count = String::from_utf8_lossy(&response)
            .matches("event: cpu")
            .count();
        match first {
            None if count > 0 => first = Some((Instant::now(), count)),
            Some((at, before)) if at.elapsed() >= window => return count - before,
            _ => {}
        }
    }
    panic!("the stream carried no cpu event in 15 s");
}

/// ACC-CONF-39-SAMPLE-RATE: with no configured interval the binary samples
/// every 250 ms; `-u` still overrides it, and the top bar shows the interval
/// in effect.
#[test]
fn default_interval_and_update_flag() {
    let (_server, address) = start(None, &[]);
    let page = page_with(address, ">250 ms<");
    assert!(page.contains(r#"data-update-ms="250""#));
    let events = cpu_events_in(address, Duration::from_secs(2));
    assert!(events >= 7, "{events} cpu events in 2 s at the default");

    let (_server, address) = start(None, &["-u", "2000"]);
    let page = page_with(address, ">2000 ms<");
    assert!(page.contains(r#"data-update-ms="2000""#));
}

/// ACC-CONF-60-VIEW-FLAG: `--proc-view` sets the starting process view; a
/// bad or missing value exits 2 and names the flag; `--help` lists it.
#[test]
fn proc_view_flag() {
    let (_server, address) = start(None, &["--proc-view", "tree"]);
    page_with(address, r#"data-proc-view="tree""#);
    for args in [&["--proc-view", "x"][..], &["--proc-view"]] {
        let (code, _, stderr) = run(args);
        assert_eq!(code, Some(2), "{args:?}");
        assert!(stderr.contains("--proc-view"), "{args:?}: {stderr}");
    }
    let (code, stdout, _) = run(&["--help"]);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("--proc-view <VIEW>"), "{stdout}");
}

/// ACC-CONF-63-TITLE-FLAG: `--title` sets the page title; an empty, blank or
/// missing value exits 2 and names the flag; `--help` lists it.
#[test]
fn title_flag() {
    let (_server, address) = start(None, &["--title", "lab-1"]);
    page_with(address, "<title>lab-1</title>");
    for args in [&["--title", ""][..], &["--title", " "], &["--title"]] {
        let (code, _, stderr) = run(args);
        assert_eq!(code, Some(2), "{args:?}");
        assert!(stderr.contains("--title"), "{args:?}: {stderr}");
    }
    let (_, stdout, _) = run(&["--help"]);
    assert!(stdout.contains("--title <TEXT>"), "{stdout}");
}

#[test]
fn binary_serves_fixture_metrics_and_an_event_stream() {
    // The first sample only seeds the CPU counters, as btop's init does. The
    // recorded counters do not move, so the second sample is a real delta of
    // zero; a short period lets it land before the page is read.
    let (_server, address) = start(Some("200"), &[]);

    let command = {
        let raw = std::fs::read(fixture("proc/111/cmdline")).expect("recorded cmdline");
        let first = raw.split(|byte| *byte == 0).next().unwrap_or_default();
        String::from_utf8_lossy(first).into_owned()
    };
    assert_eq!(command, "ttyd");

    let page = page_with(address, r#"id="cpu-total">0<"#);
    assert!(page.contains(&command), "no recorded process");
    assert!(page.contains(">200 ms<"), "the top bar lacks the interval");

    assert!(!page.contains("per-core usage unavailable"), "{page}");
    // btop never draws the since-boot sample its init collect computes.
    assert!(
        !page.contains(&format!(r#"id="cpu-total">{}<"#, fixture_cpu_total())),
        "the since-boot CPU average leaked into a frame"
    );
    assert!(page.contains("cpu0"), "no per-core rows");
    assert!(page.contains("GiB"), "no memory total");
    assert!(page.contains("eth0"), "no recorded interface");
    assert!(
        page.contains("GPU metrics unavailable"),
        "the recorded card0 has no vendor file and must report unavailable"
    );

    let events = read_event_stream(address);
    assert!(
        events
            .to_ascii_lowercase()
            .contains("content-type: text/event-stream")
    );
    assert!(events.contains("event: cpu"));
    assert!(events.contains("data: "));
}

/// ACC-CONF-61-CONFIG-PAGES: with the guard off, a same-origin POST changes
/// `update_ms` for the one sampler and rewrites only that line of the `-c`
/// file, a copy of the shipped defaults.
#[test]
fn config_post_sets_the_interval_and_its_line() {
    let original =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("config/btop-web.conf"))
            .expect("shipped defaults");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("e2e-config-post");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("btop.conf");
    std::fs::write(&path, &original).expect("copy");
    let (_server, address) = start(
        None,
        &["-c", path.to_str().expect("utf-8 path"), "--no-readonly"],
    );
    page_with(address, ">250 ms<");
    let body = "key=update_ms&value=500";
    let response = exchange(
        address,
        &format!(
            "POST /config HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
    .expect("response");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let written = std::fs::read_to_string(&path).expect("file");
    let changed: Vec<(&str, &str)> = written
        .lines()
        .zip(original.lines())
        .filter(|(now, was)| now != was)
        .collect();
    assert_eq!(changed, [("update_ms = 500", "update_ms = 250")]);
    assert_eq!(written.lines().count(), original.lines().count());
    let events = cpu_events_in(address, Duration::from_secs(3));
    assert!(
        (4..=8).contains(&events),
        "{events} cpu events in 3 s at 500 ms"
    );
}

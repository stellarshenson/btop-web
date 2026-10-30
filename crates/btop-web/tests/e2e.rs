//! End-to-end: boot the real binary against the recorded `/proc` and `/sys`
//! trees and read the page and one SSE frame over a socket.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
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

/// The XDG config directory for `name`, without clearing it, so a test can
/// find a file the binary wrote there.
fn xdg_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("e2e-xdg")
        .join(name)
}

/// A fresh XDG_CONFIG_HOME under the test target's temp dir, so no test reads
/// or writes the real ~/.config.
fn xdg_home(name: &str) -> PathBuf {
    let dir = xdg_path(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("xdg dir");
    dir
}

/// ACC-NET-144-ACCESS-KEY: the key an agent wrote in its config directory,
/// once the file exists, checked to be mode 0600.
fn agent_key(xdg: &Path) -> String {
    let path = xdg.join("btop-web").join("access-key");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} was not created",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        std::fs::metadata(&path)
            .expect("key metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "the access key file is private"
    );
    std::fs::read_to_string(&path)
        .expect("key file")
        .trim()
        .to_string()
}

/// Start the binary on a free loopback port against the recorded tree, with
/// its own XDG_CONFIG_HOME.
fn start(update_ms: Option<&str>, args: &[&str]) -> (Server, SocketAddr) {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    start_in(&xdg_home(&format!("run-{run}")), update_ms, args)
}

/// Start the binary with unambiguous environment variables: `envs` sets the
/// ones given, and the collector root is removed unless named, so a client
/// never inherits it.
fn start_mode(xdg: &str, args: &[String], envs: &[(&str, &str)]) -> (Server, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("free port");
    let address = listener.local_addr().expect("address");
    drop(listener);
    let mut command = Command::new(env!("CARGO_BIN_EXE_btop-web"));
    command
        .args(args)
        .env("BTOP_WEB_LISTEN", address.to_string())
        .env("XDG_CONFIG_HOME", xdg_home(xdg))
        .env_remove("BTOP_WEB_UPDATE_MS")
        .env_remove("BTOP_WEB_PROC_ROOT")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in envs {
        command.env(key, value);
    }
    (Server(command.spawn().expect("spawn btop-web")), address)
}

fn start_in(xdg: &Path, update_ms: Option<&str>, args: &[&str]) -> (Server, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("free port");
    let address = listener.local_addr().expect("address");
    drop(listener);
    let mut command = Command::new(env!("CARGO_BIN_EXE_btop-web"));
    command
        .args(args)
        .env("BTOP_WEB_LISTEN", address.to_string())
        .env("BTOP_WEB_PROC_ROOT", fixture("proc"))
        .env("XDG_CONFIG_HOME", xdg)
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
    run_env(args, &[])
}

/// [`run`] with extra environment variables set for the child, so a test can
/// name a passphrase variable without touching this process's environment.
fn run_env(args: &[&str], envs: &[(&str, &str)]) -> (Option<i32>, String, String) {
    // One XDG directory per run: tests run in parallel, and a shared
    // directory is cleared while another child still holds its key file.
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut command = Command::new(env!("CARGO_BIN_EXE_btop-web"));
    command
        .args(args)
        .env("BTOP_WEB_LISTEN", "127.0.0.1:0")
        .env("BTOP_WEB_PROC_ROOT", fixture("proc"))
        .env("XDG_CONFIG_HOME", xdg_home(&format!("run-{run}")))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut server = Server(command.spawn().expect("spawn btop-web"));
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

/// Start an agent on a free loopback port against the recorded tree, with its
/// own XDG directory and extra environment variables.
fn start_agent(xdg: &str, args: &[&str], envs: &[(&str, &str)]) -> (Server, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("free port");
    let address = listener.local_addr().expect("address");
    drop(listener);
    let mut command = Command::new(env!("CARGO_BIN_EXE_btop-web"));
    command
        .args(args)
        .env("BTOP_WEB_LISTEN", address.to_string())
        .env("BTOP_WEB_PROC_ROOT", fixture("proc"))
        .env("XDG_CONFIG_HOME", xdg_home(xdg))
        .env_remove("BTOP_WEB_UPDATE_MS")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in envs {
        command.env(key, value);
    }
    (Server(command.spawn().expect("spawn btop-web")), address)
}

/// A single-quoted shell word for a path, so the descriptor script carries
/// paths but no secret value.
fn shell_word(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// The page once it carries `marker`, within 20 s.
fn page_with(address: SocketAddr, marker: &str) -> String {
    response_with(address, "/", marker)
}

/// The response from `path` once it carries `marker`, within 20 s.
fn response_with(address: SocketAddr, path: &str, marker: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(response) = get(address, path)
            && response.starts_with("HTTP/1.1 200")
            && response.contains(marker)
        {
            return response;
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

/// ACC-NET-145-TLS-PINNING: a TLS 1.3 request accepted on first use, read
/// until `done` holds for the text received, the peer closes, or 10 s pass.
/// The real pinning is the client's `known_agents`, checked in the test.
fn tls_read(address: SocketAddr, request: &str, done: impl Fn(&str) -> bool) -> Option<String> {
    let (config, _verifier) = btop_web::tls::client_config(None).ok()?;
    let server_name = rustls::pki_types::ServerName::try_from("127.0.0.1")
        .ok()?
        .to_owned();
    let connection = rustls::ClientConnection::new(config, server_name).ok()?;
    let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(1)).ok()?;
    tcp.set_read_timeout(Some(Duration::from_millis(200)))
        .ok()?;
    let mut stream = rustls::StreamOwned::new(connection, tcp);
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8192];
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                response.extend_from_slice(&buffer[..read]);
                if done(&String::from_utf8_lossy(&response)) {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
    }
    Some(String::from_utf8_lossy(&response).into_owned())
}

/// ACC-NET-145-TLS-PINNING: a TLS GET, keyless when `key` is `None`.
fn tls_get(address: SocketAddr, path: &str, key: Option<&str>) -> Option<String> {
    let authorization = key.map_or_else(String::new, |key| {
        format!("Authorization: Bearer {key}\r\n")
    });
    tls_read(
        address,
        &format!(
            "GET {path} HTTP/1.1\r\nHost: {address}\r\nAccept: */*\r\n{authorization}Connection: close\r\n\r\n"
        ),
        |_| false,
    )
}

/// ACC-NET-145-TLS-PINNING: the TLS response from `path` once it carries
/// `marker`.
fn tls_response_with_key(address: SocketAddr, path: &str, key: &str, marker: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(response) = tls_get(address, path, Some(key))
            && response.starts_with("HTTP/1.1 200")
            && response.contains(marker)
        {
            return response;
        }
        assert!(
            Instant::now() < deadline,
            "the agent did not serve {marker} over TLS in 20 s"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// ACC-NET-145-TLS-PINNING: the agent's TLS event stream with its key.
fn tls_event_stream(address: SocketAddr, key: &str) -> String {
    tls_read(
        address,
        &format!(
            "GET /events HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\nAuthorization: Bearer {key}\r\n\r\n"
        ),
        |text| text.contains("event: cpu") && text.contains("data-update-ms=\""),
    )
    .expect("TLS event stream")
}

/// The bytes after a response's head; a 401 body must be empty.
fn response_body(response: &str) -> &str {
    response.split_once("\r\n\r\n").map_or("", |(_, body)| body)
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
    read_event_stream_at(address, "/events", None)
}

fn read_event_stream_at(address: SocketAddr, path: &str, key: Option<&str>) -> String {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let authorization = key.map_or_else(String::new, |key| {
        format!("Authorization: Bearer {key}\r\n")
    });
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\n{authorization}\r\n"
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
                if text.contains("event: cpu")
                    && text.contains("data: ")
                    && text.contains("<h2")
                    && text.contains("data-update-ms=\"")
                {
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

/// The number of `cpu` events the stream at `path` carries in `window` after
/// its first.
fn cpu_events_in(address: SocketAddr, path: &str, window: Duration) -> usize {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("timeout");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\n\r\n"
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

/// An SSE connection opened before a config write; a viewer that never
/// reloads keeps this connection, so its events after the write prove the
/// change reached it.
struct LiveEvents {
    stream: TcpStream,
    received: Vec<u8>,
}

impl LiveEvents {
    fn open(address: SocketAddr) -> Self {
        let mut stream =
            TcpStream::connect_timeout(&address, Duration::from_secs(2)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("timeout");
        write!(
            stream,
            "GET /events HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\n\r\n"
        )
        .expect("request");
        let mut events = Self {
            stream,
            received: Vec::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            events.read_once();
            if events.cpu_events() > 0 {
                return events;
            }
        }
        panic!("the stream carried no cpu event in 15 s");
    }

    fn read_once(&mut self) {
        let mut buffer = [0_u8; 65536];
        if let Ok(read) = self.stream.read(&mut buffer)
            && read > 0
        {
            self.received.extend_from_slice(&buffer[..read]);
        }
    }

    fn read_for(&mut self, window: Duration) {
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            self.read_once();
        }
    }

    fn cpu_events(&self) -> usize {
        String::from_utf8_lossy(&self.received)
            .matches("event: cpu")
            .count()
    }
}

/// ACC-CONF-39-SAMPLE-RATE: with no configured interval the binary samples
/// every 250 ms; `-u` still overrides it, and the top bar shows the interval
/// in effect.
#[test]
fn default_interval_and_update_flag() {
    let (_server, address) = start(None, &[]);
    let page = page_with(address, ">250 ms<");
    assert!(page.contains(r#"data-update-ms="250""#));
    let events = cpu_events_in(address, "/events", Duration::from_secs(2));
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

/// ACC-CONF-147-OPTIONS-EDIT and ACC-CONF-148-OWN-FILE: with the guard off, a
/// same-origin POST changes `update_ms` for the one sampler and writes it to
/// btop-web.conf under XDG_CONFIG_HOME; a second viewer already connected
/// follows the new cadence without a reload; the `-c` btop.conf stays byte
/// for byte, a restart with both files runs at the new value, and a read-only
/// restart renders the row as text again.
#[test]
fn config_post_writes_the_web_file_and_leaves_btop_conf_alone() {
    use std::os::unix::fs::PermissionsExt;
    let original =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("config/btop-web.conf"))
            .expect("shipped defaults");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("e2e-config-post");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let base = dir.join("btop.conf");
    std::fs::write(&base, &original).expect("copy");
    let base_before = std::fs::read(&base).expect("base");
    let xdg = xdg_home("config-post");
    let args = ["-c", base.to_str().expect("utf-8 path"), "--no-readonly"];
    let (_server, address) = start_in(&xdg, None, &args);
    page_with(address, ">250 ms<");
    // The second viewer is connected before the write and never reloads.
    let mut viewer = LiveEvents::open(address);
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
    assert_eq!(
        std::fs::read(&base).expect("base"),
        base_before,
        "btop.conf changed"
    );
    let web = xdg.join("btop-web").join("btop-web.conf");
    assert_eq!(
        std::fs::read_to_string(&web).expect("web file"),
        "update_ms = 500\n"
    );
    assert_eq!(
        std::fs::metadata(&web)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "a created btop-web.conf is private"
    );
    // The open stream carries the new interval: one second drains the
    // transition, then three seconds carry six events at 500 ms and twelve at
    // the old 250 ms.
    viewer.read_for(Duration::from_secs(1));
    let before = viewer.cpu_events();
    viewer.read_for(Duration::from_secs(3));
    let events = viewer.cpu_events() - before;
    assert!(
        (4..=8).contains(&events),
        "{events} cpu events in 3 s on the open stream, not the new 500 ms interval"
    );

    // A restart with both files: the web file overrides the btop.conf baseline.
    let (_server, address) = start_in(&xdg, None, &args);
    let page = page_with(address, ">500 ms<");
    assert!(page.contains(r#"data-update-ms="500""#), "{page}");

    // A read-only restart shows the saved value and the row as text again.
    let (_server, address) = start_in(&xdg, None, &["-c", args[1], "--readonly"]);
    let page = page_with(address, r#"data-key="update_ms""#);
    assert!(
        page.contains(r#"<span class="cfg-val">500</span>"#),
        "{page}"
    );
    assert!(!page.contains(r#"hx-post="config""#), "{page}");
}

/// ACC-NET-144-ACCESS-KEY: a client with no key line, or with a key file it
/// cannot read, refuses to start and names the agent and the file.
#[test]
fn a_client_without_an_access_key_refuses_to_start() {
    let (code, _, stderr) = run(&["--client", "--connect", "nas=http://127.0.0.1:1"]);
    assert_ne!(code, Some(0), "{stderr}");
    assert!(stderr.contains("nas"), "{stderr}");
    assert!(stderr.contains("connect_key"), "{stderr}");

    let conf = Path::new(env!("CARGO_TARGET_TMPDIR")).join("missing-key.conf");
    std::fs::write(
        &conf,
        "connect_key = nas=/nonexistent/btop-web-access-key\n",
    )
    .expect("conf");
    let (code, _, stderr) = run(&[
        "--client",
        "--connect",
        "nas=http://127.0.0.1:1",
        "-c",
        conf.to_str().expect("utf-8 config"),
    ]);
    assert_ne!(code, Some(0), "{stderr}");
    assert!(stderr.contains("nas"), "{stderr}");
    assert!(
        stderr.contains("/nonexistent/btop-web-access-key"),
        "{stderr}"
    );
}

/// ACC-NET-144-ACCESS-KEY: the start that creates the key shows it on the
/// controlling terminal only. Captured stdout and stderr are the shape a
/// service logs (journald, docker logs, `| tee`), and neither may hold it
/// while the key file does.
#[test]
fn the_created_key_never_reaches_stdout_or_stderr() {
    let xdg = xdg_home("key-print");
    let logs = Path::new(env!("CARGO_TARGET_TMPDIR")).join("key-print");
    let _ = std::fs::remove_dir_all(&logs);
    std::fs::create_dir_all(&logs).expect("log dir");
    let stdout_path = logs.join("stdout.log");
    let stderr_path = logs.join("stderr.log");
    let listener = TcpListener::bind("127.0.0.1:0").expect("free port");
    let address = listener.local_addr().expect("address");
    drop(listener);
    let mut command = Command::new(env!("CARGO_BIN_EXE_btop-web"));
    command
        .arg("--agent")
        .env("BTOP_WEB_LISTEN", address.to_string())
        .env("BTOP_WEB_PROC_ROOT", fixture("proc"))
        .env("XDG_CONFIG_HOME", &xdg)
        .stdout(Stdio::from(
            std::fs::File::create(&stdout_path).expect("stdout log"),
        ))
        .stderr(Stdio::from(
            std::fs::File::create(&stderr_path).expect("stderr log"),
        ));
    let _server = Server(command.spawn().expect("spawn btop-web"));
    let key = agent_key(&xdg);
    // Read the logs once the creating start's line has been written.
    let deadline = Instant::now() + Duration::from_secs(10);
    let (out, err) = loop {
        let out = std::fs::read_to_string(&stdout_path).expect("stdout");
        let err = std::fs::read_to_string(&stderr_path).expect("stderr");
        if out.contains("access key created") || err.contains("access key created") {
            break (out, err);
        }
        assert!(
            Instant::now() < deadline,
            "the creating start logged no line: {out} {err}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!out.contains(&key), "stdout holds the access key: {out}");
    assert!(!err.contains(&key), "stderr holds the access key: {err}");
}

/// ACC-NET-143-AGENT-MODES: an agent serves telemetry and no page; a client on
/// loopback renders from two agents, names each with its sample age, selects
/// by name and marks a stopped agent `unavailable` while the other keeps
/// streaming. Two viewers share one upstream per agent.
#[test]
fn agent_and_client_modes() {
    let root = fixture("proc");
    let root = root.to_str().expect("utf-8 fixture path");
    let (_alpha, alpha_addr) = start_mode(
        "modes-alpha",
        &["--agent".to_string()],
        &[("BTOP_WEB_PROC_ROOT", root), ("BTOP_WEB_UPDATE_MS", "250")],
    );
    let (beta, beta_addr) = start_mode(
        "modes-beta",
        &["--agent".to_string()],
        &[("BTOP_WEB_PROC_ROOT", root), ("BTOP_WEB_UPDATE_MS", "500")],
    );
    // ACC-NET-144-ACCESS-KEY: each agent created its key on first start, mode
    // 0600. ACC-NET-145-TLS-PINNING: each also generated its TLS pair, key
    // mode 0600, and the listener speaks TLS alone.
    let alpha_key = agent_key(&xdg_path("modes-alpha"));
    agent_key(&xdg_path("modes-beta"));
    for name in ["modes-alpha", "modes-beta"] {
        let key = xdg_path(name).join("btop-web").join("agent.key");
        assert!(key.exists(), "{} was not generated", key.display());
        assert_eq!(
            std::fs::metadata(&key)
                .expect("agent key metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "the agent TLS key is not private"
        );
        assert!(xdg_path(name).join("btop-web").join("agent.crt").exists());
    }
    let health = tls_response_with_key(alpha_addr, "/healthz", &alpha_key, "ok");
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    let root_page = tls_get(alpha_addr, "/", Some(&alpha_key)).expect("agent root");
    assert!(
        root_page.starts_with("HTTP/1.1 404"),
        "an agent serves no page: {root_page}"
    );
    for response in [
        tls_get(alpha_addr, "/events", None).expect("keyless events"),
        tls_get(alpha_addr, "/events", Some("wrong-key")).expect("wrong key"),
    ] {
        assert!(
            response.starts_with("HTTP/1.1 401"),
            "an agent without its key: {response}"
        );
        assert!(
            response_body(&response).is_empty(),
            "a refusal carries a body: {response}"
        );
    }
    let agent_events = tls_event_stream(alpha_addr, &alpha_key);
    assert!(agent_events.contains("event: cpu"), "{agent_events}");

    // The key file paths live in a configuration file, never on the command
    // line; the --connect flags carry the URLs only.
    let key_file = Path::new(env!("CARGO_TARGET_TMPDIR")).join("modes-connect-keys.conf");
    std::fs::write(
        &key_file,
        format!(
            "connect_key = alpha={}\nconnect_key = beta={}\n",
            xdg_path("modes-alpha")
                .join("btop-web")
                .join("access-key")
                .display(),
            xdg_path("modes-beta")
                .join("btop-web")
                .join("access-key")
                .display(),
        ),
    )
    .expect("client key config");
    let client_args = [
        "--client".to_string(),
        "-c".to_string(),
        key_file.to_str().expect("utf-8 key config").to_string(),
        "--connect".to_string(),
        format!("alpha=http://{alpha_addr}"),
        "--connect".to_string(),
        format!("beta=http://{beta_addr}"),
    ];
    let (_client, client_addr) = start_mode("modes-client", &client_args, &[]);

    // The client names both agents; the first is selected by default.
    let page = response_with(client_addr, "/", "href=\"?agent=alpha\"");
    assert!(
        page.contains("href=\"?agent=alpha\" aria-current=\"page\""),
        "{page}"
    );
    assert!(page.contains("href=\"?agent=beta\""), "{page}");
    assert!(
        !page.contains("href=\"?agent=beta\" aria-current=\"page\""),
        "{page}"
    );
    assert!(
        page.contains("sse-connect=\"events?agent=alpha\""),
        "{page}"
    );
    // ACC-NET-143-AGENT-MODES: the strip's poll and chip links are relative,
    // so they resolve under the lab proxy's trailing-slash path, and the
    // strip is never a priority-tier field.
    assert!(
        page.contains("hx-get=\"agents?agent=alpha\""),
        "the strip poll is not relative: {page}"
    );
    assert!(!page.contains("hx-get=\"/agents"), "{page}");
    assert!(!page.contains("id=\"agents\" data-p="), "{page}");

    // A chip selects by name; an unknown name is a 404 on the page and on
    // the stream.
    let page = response_with(client_addr, "/?agent=beta", "aria-current");
    assert!(
        page.contains("href=\"?agent=beta\" aria-current=\"page\""),
        "{page}"
    );
    assert!(page.contains("sse-connect=\"events?agent=beta\""), "{page}");
    for path in ["/?agent=nope", "/events?agent=nope", "/agents?agent=nope"] {
        let missing = get(client_addr, path).expect("unknown agent");
        assert!(missing.starts_with("HTTP/1.1 404"), "{path}: {missing}");
    }

    // The strip carries one sample age per agent, and each selection relays
    // that agent's own interval, 250 ms against 500 ms.
    let deadline = Instant::now() + Duration::from_secs(20);
    let strip = loop {
        let strip = get(client_addr, "/agents?agent=alpha").expect("strip");
        if strip.matches(" s</b>").count() == 2 && strip.contains(">alpha</span>") {
            break strip;
        }
        assert!(
            Instant::now() < deadline,
            "the strip shows no two sample ages: {strip}"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        strip.contains("href=\"?agent=alpha\" aria-current=\"page\""),
        "{strip}"
    );
    assert!(
        strip.contains("hx-get=\"agents?agent=alpha\""),
        "the strip's own poll is not relative: {strip}"
    );
    // ACC-NET-145-TLS-PINNING: the client recorded one pin per agent in the
    // config directory, mode 0600, as `NAME sha256:HEX`.
    let pin_file = xdg_path("modes-client")
        .join("btop-web")
        .join("known_agents");
    let deadline = Instant::now() + Duration::from_secs(20);
    let pins = loop {
        match std::fs::read_to_string(&pin_file) {
            Ok(pins) if pins.lines().count() == 2 => break pins,
            _ => {
                assert!(
                    Instant::now() < deadline,
                    "the client wrote no pin file at {}",
                    pin_file.display()
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };
    for name in ["alpha", "beta"] {
        assert!(
            pins.lines()
                .any(|line| line.starts_with(&format!("{name} sha256:"))),
            "{name} has no pin: {pins}"
        );
    }
    assert_eq!(
        std::fs::metadata(&pin_file)
            .expect("pin metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "the pin file is not private"
    );
    let beta_events = read_event_stream_at(client_addr, "/events?agent=beta", None);
    assert!(
        beta_events.contains("data-update-ms=\"500\""),
        "{beta_events}"
    );
    let alpha_events = read_event_stream_at(client_addr, "/events?agent=alpha", None);
    assert!(
        alpha_events.contains("data-update-ms=\"250\""),
        "{alpha_events}"
    );

    // Two viewers at once: each receives the agent's frames at its tick rate.
    let first = std::thread::spawn(move || {
        cpu_events_in(client_addr, "/events?agent=alpha", Duration::from_secs(2))
    });
    let second = std::thread::spawn(move || {
        cpu_events_in(client_addr, "/events?agent=alpha", Duration::from_secs(2))
    });
    let first = first.join().expect("first viewer");
    let second = second.join().expect("second viewer");
    assert!(
        first >= 6 && second >= 6,
        "{first} and {second} cpu events in 2 s with two viewers"
    );

    // Stopping one agent marks it unavailable; the other keeps its age and
    // keeps streaming. No invented data: the stopped chip carries the word,
    // not a zero age.
    drop(beta);
    let strip = response_with(client_addr, "/agents?agent=alpha", "unavailable");
    assert!(
        strip.contains("beta</span><b class=\"agent-age\" data-w=\"unavailable\">unavailable"),
        "{strip}"
    );
    assert!(
        strip.contains("alpha</span><b class=\"agent-age\" data-w=\"unavailable\">"),
        "{strip}"
    );
    assert!(
        read_event_stream_at(client_addr, "/events?agent=alpha", None).contains("event: cpu"),
        "the surviving agent stopped streaming"
    );
}

/// ACC-NET-146-SECRET-INPUT: the certificate, encrypted key, access key and
/// passphrase all arrive on inherited descriptors, the agent starts and
/// answers over TLS with the access key read from fd 5, and its command line
/// carries sources but no secret value.
#[test]
fn descriptors_carry_every_secret_and_the_agent_starts() {
    const ACCESS_KEY: &str = "descriptor-access-key-0123456789";
    const PASSPHRASE: &str = "btop-web-test-passphrase";
    let dir = xdg_home("descriptors");
    let access = dir.join("access-key");
    std::fs::write(&access, format!("{ACCESS_KEY}\n")).expect("access key file");
    let passphrase = dir.join("passphrase");
    std::fs::write(&passphrase, format!("{PASSPHRASE}\n")).expect("passphrase file");
    let cert = fixture("tls/agent.crt");
    let key = fixture("tls/agent-encrypted.key");
    let binary = Path::new(env!("CARGO_BIN_EXE_btop-web"));

    let listener = TcpListener::bind("127.0.0.1:0").expect("free port");
    let address = listener.local_addr().expect("address");
    drop(listener);
    let script = format!(
        "exec 3<{}; exec 4<{}; exec 5<{}; exec 6<{}; exec {} --agent \
         --tls-cert-fd 3 --tls-key-fd 4 --access-key-fd 5 \
         --tls-key-passphrase-fd 6",
        shell_word(&cert),
        shell_word(&key),
        shell_word(&access),
        shell_word(&passphrase),
        shell_word(binary),
    );
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(script)
        .env("BTOP_WEB_LISTEN", address.to_string())
        .env("BTOP_WEB_PROC_ROOT", fixture("proc"))
        .env("XDG_CONFIG_HOME", &dir)
        .env_remove("BTOP_WEB_UPDATE_MS")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let server = Server(command.spawn().expect("spawn sh"));

    // The key read from fd 5 is the one the listener demands.
    let health = tls_response_with_key(address, "/healthz", ACCESS_KEY, "ok");
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");

    // Reading the command line after the agent answers proves the process is
    // btop-web, and no secret value is in it.
    let cmdline = std::fs::read(format!("/proc/{}/cmdline", server.0.id()))
        .expect("the agent's command line");
    let cmdline = String::from_utf8_lossy(&cmdline);
    for flag in [
        "--tls-cert-fd",
        "--tls-key-fd",
        "--access-key-fd",
        "--tls-key-passphrase-fd",
    ] {
        assert!(
            cmdline.contains(flag),
            "{flag} is not on the command line: {cmdline}"
        );
    }
    for secret in [ACCESS_KEY, PASSPHRASE, "PRIVATE KEY"] {
        assert!(
            !cmdline.contains(secret),
            "a secret value is on the command line: {cmdline}"
        );
    }
}

/// ACC-NET-146-SECRET-INPUT: a descriptor that is not open stops the agent
/// with the secret and the descriptor number named.
#[test]
fn a_closed_descriptor_stops_the_agent_naming_the_secret() {
    let key = fixture("tls/agent.key");
    let (code, _, stderr) = run(&[
        "--agent",
        "--tls-cert-fd",
        "99",
        "--tls-key",
        key.to_str().expect("utf-8 key path"),
    ]);
    assert_ne!(code, Some(0), "a closed descriptor started the agent");
    assert!(stderr.contains("TLS certificate"), "{stderr}");
    assert!(stderr.contains("descriptor 99"), "{stderr}");
}

/// ACC-NET-146-SECRET-INPUT: a missing certificate or access key file stops
/// the agent with the secret and the path named, never a generic failure.
#[test]
fn a_missing_secret_file_stops_the_agent_naming_the_secret() {
    let key = fixture("tls/agent.key");
    let (code, _, stderr) = run(&[
        "--agent",
        "--tls-cert",
        "/nonexistent/btop-web-agent.crt",
        "--tls-key",
        key.to_str().expect("utf-8 key path"),
    ]);
    assert_ne!(code, Some(0), "a missing certificate started the agent");
    assert!(stderr.contains("TLS certificate"), "{stderr}");
    assert!(
        stderr.contains("/nonexistent/btop-web-agent.crt"),
        "{stderr}"
    );

    let (code, _, stderr) = run(&[
        "--agent",
        "--access-key-file",
        "/nonexistent/btop-web-access-key",
    ]);
    assert_ne!(code, Some(0), "a missing access key started the agent");
    assert!(stderr.contains("access key"), "{stderr}");
    assert!(
        stderr.contains("/nonexistent/btop-web-access-key"),
        "{stderr}"
    );
}

/// ACC-NET-146-SECRET-INPUT: an encrypted key with no passphrase, a named
/// variable that is unset and a wrong passphrase each stop the agent naming
/// which secret failed; the right passphrase decrypts the key and the agent
/// serves TLS.
#[test]
fn an_encrypted_key_names_its_passphrase_source_and_serves_tls_when_it_is_right() {
    let cert = fixture("tls/agent.crt");
    let key = fixture("tls/agent-encrypted.key");
    let cert = cert.to_str().expect("utf-8 certificate path");
    let key = key.to_str().expect("utf-8 key path");
    let args = [
        "--agent",
        "--tls-cert",
        cert,
        "--tls-key",
        key,
        "--tls-key-passphrase-env",
        "BTOP_WEB_TEST_PASSPHRASE",
    ];

    // No passphrase source at all: the encrypted key is refused by name.
    let (code, _, stderr) = run(&["--agent", "--tls-cert", cert, "--tls-key", key]);
    assert_ne!(
        code,
        Some(0),
        "an encrypted key without a passphrase started"
    );
    assert!(stderr.contains("TLS key"), "{stderr}");
    assert!(stderr.contains("passphrase"), "{stderr}");

    // The named variable is not set.
    let (code, _, stderr) = run(&args);
    assert_ne!(code, Some(0), "an unset passphrase variable started");
    assert!(stderr.contains("TLS key passphrase"), "{stderr}");
    assert!(stderr.contains("BTOP_WEB_TEST_PASSPHRASE"), "{stderr}");
    assert!(stderr.contains("not set"), "{stderr}");

    // A wrong passphrase fails to decrypt, naming the key and the source.
    let (code, _, stderr) = run_env(&args, &[("BTOP_WEB_TEST_PASSPHRASE", "wrong-passphrase")]);
    assert_ne!(code, Some(0), "a wrong passphrase started");
    assert!(stderr.contains("cannot decrypt TLS key"), "{stderr}");
    assert!(stderr.contains("BTOP_WEB_TEST_PASSPHRASE"), "{stderr}");

    // The right passphrase decrypts the key, and the agent serves TLS.
    let (_server, address) = start_agent(
        "encrypted-agent",
        &args,
        &[("BTOP_WEB_TEST_PASSPHRASE", "btop-web-test-passphrase")],
    );
    let access_key = agent_key(&xdg_path("encrypted-agent"));
    let response = tls_response_with_key(address, "/healthz", &access_key, "ok");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
}

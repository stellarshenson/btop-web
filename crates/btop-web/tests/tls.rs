//! ACC-NET-145-TLS-PINNING: an agent listener speaks TLS 1.3 only, the
//! client pins the certificate fingerprint it saw on the first connect, and
//! the access key still guards every request. Every test binds a free
//! loopback port and collects the recorded fixture tree.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use btop_web::AppState;
use btop_web::client::Client;
use btop_web::collector::frame::Frame;
use btop_web::collector::{Snapshot, cpu, gpu, mem, net, proc};
use btop_web::config::{Config, Connect, ConnectKey};
use btop_web::secret::Source;
use btop_web::tls;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_rustls::client::TlsStream;

/// The key the test agent demands, and the file the client reads it from.
const KEY: &str = "the-agent-key";

/// A fresh directory under the test target's temp dir.
fn temp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("tls-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

/// One snapshot of the recorded host, so `/events` has fragments to send.
fn fixture_snapshot() -> Snapshot {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proc");
    let config = Config::load(None).expect("config");
    let mut mem = mem::MemCollector::default();
    let mut snapshot = Snapshot::default();
    for _ in 0..2 {
        snapshot = Snapshot {
            cpu: cpu::collect(&root, &config),
            mem: mem.collect(&root, &config),
            net: net::collect(&root, &config),
            proc: proc::collect(&root, &config),
            gpu: gpu::collect(&root, &config),
            update_ms: config.update_ms,
            window_ms: btop_web::chart::window(config.window_ms(), config.update_ms),
            slow_ms: config.proc_refresh_ms.max(config.update_ms),
            slow_seq: 1,
            container: false,
        };
    }
    snapshot
}

/// The agent's TLS listener on a free loopback port, with a generated pair at
/// `dir/agent.crt` and `dir/agent.key`.
struct Agent {
    addr: std::net::SocketAddr,
    fingerprint: String,
    /// Held for the test's lifetime so the watch keeps its frame and the SSE
    /// stream stays open after the first frame.
    _sender: watch::Sender<Option<Arc<Frame>>>,
}

async fn start_agent(dir: &Path) -> Agent {
    let identity = tls::identity_at(&dir.join("agent.crt"), &dir.join("agent.key"))
        .expect("generated identity");
    let fingerprint = identity.fingerprint.clone();
    let config = tls::server_config(identity.certs, identity.key).expect("server config");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("free loopback port");
    let addr = listener.local_addr().expect("bound address");
    let (sender, receiver) = watch::channel(Some(Arc::new(Frame::new(fixture_snapshot()))));
    let mut state = AppState::new(receiver, true);
    state.access_key = Some(KEY.into());
    let app = btop_web::routes::agent_router(state);
    tokio::spawn(tls::serve(listener, app, config, std::future::pending()));
    Agent {
        addr,
        fingerprint,
        _sender: sender,
    }
}

/// A TLS 1.3 client over a tokio stream, pinned to `pin` when given.
async fn connect(addr: std::net::SocketAddr, pin: Option<&str>) -> TlsStream<TcpStream> {
    let (config, _verifier) = tls::client_config(pin).expect("client config");
    tls_connect(config, addr).await.expect("TLS handshake")
}

async fn tls_connect(
    config: Arc<rustls::ClientConfig>,
    addr: std::net::SocketAddr,
) -> Result<TlsStream<TcpStream>, std::io::Error> {
    let connector = tokio_rustls::TlsConnector::from(config);
    let server_name = ServerName::try_from("127.0.0.1")
        .expect("an IP literal is a server name")
        .to_owned();
    connector
        .connect(server_name, TcpStream::connect(addr).await?)
        .await
        .map_err(std::io::Error::other)
}

/// One request over the TLS stream, read to the connection's end.
async fn get(stream: &mut TlsStream<TcpStream>, path: &str, key: Option<&str>) -> (u16, String) {
    let authorization = key.map_or_else(String::new, |key| {
        format!("Authorization: Bearer {key}\r\n")
    });
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: */*\r\n{authorization}Connection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.expect("request");
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(read)) => response.extend_from_slice(&buffer[..read]),
        }
    }
    let response = String::from_utf8_lossy(&response).into_owned();
    let status = response
        .split(' ')
        .nth(1)
        .and_then(|status| status.parse().ok())
        .unwrap_or(0);
    let body = response
        .split_once("\r\n\r\n")
        .map_or(String::new(), |(_, body)| body.to_string());
    (status, body)
}

/// The SSE stream over TLS, read until the first cpu fragment.
async fn events(stream: &mut TlsStream<TcpStream>) -> String {
    let request = format!(
        "GET /events HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\nAuthorization: Bearer {KEY}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.expect("request");
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8192];
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), stream.read(&mut buffer)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(read)) => {
                response.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&response);
                if text.contains("HTTP/1.1 200")
                    && text.contains("event: cpu")
                    && text.contains("data-update-ms=\"")
                {
                    return text.into_owned();
                }
            }
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
    panic!(
        "no cpu fragment over TLS in 15 s; received: {}",
        String::from_utf8_lossy(&response)
    );
}

/// ACC-NET-145-TLS-PINNING: the agent answers TLS 1.3, and a client that
/// offers TLS 1.2 alone does not complete a handshake.
#[tokio::test]
async fn an_agent_answers_tls_13_and_refuses_a_tls_12_handshake() {
    let dir = temp("versions");
    let agent = start_agent(&dir).await;

    let mut stream = connect(agent.addr, Some(&agent.fingerprint)).await;
    assert_eq!(
        stream.get_ref().1.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_3),
        "the negotiated protocol is not TLS 1.3"
    );
    let (status, body) = get(&mut stream, "/healthz", Some(KEY)).await;
    assert_eq!(status, 200);
    assert_eq!(body, "ok");
    drop(stream);

    // A TLS 1.2-only client: the server's version list holds TLS 1.3 alone,
    // so the handshake fails before any request.
    let verifier = Arc::new(tls::PinVerifier::new(Some(&agent.fingerprint)));
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS12])
    .expect("TLS 1.2 configuration")
    .dangerous()
    .with_custom_certificate_verifier(verifier)
    .with_no_client_auth();
    let result = tls_connect(Arc::new(config), agent.addr).await;
    assert!(
        result.is_err(),
        "a TLS 1.2-only client completed a handshake with the agent"
    );

    // The refused handshake did not stop the listener.
    let mut stream = connect(agent.addr, Some(&agent.fingerprint)).await;
    assert_eq!(get(&mut stream, "/healthz", Some(KEY)).await.0, 200);
}

/// ACC-NET-145-TLS-PINNING: the client records the first fingerprint in the
/// pin file mode 0600, a later connect with the same certificate passes, and
/// a rotated certificate is refused naming the agent and both fingerprints.
#[tokio::test]
async fn the_client_pins_the_first_fingerprint_and_refuses_a_changed_certificate() {
    let dir = temp("pins");
    let agent = start_agent(&dir.join("first")).await;

    let pins_path = dir.join("known_agents");
    let key_path = dir.join("access-key");
    std::fs::write(&key_path, format!("{KEY}\n")).expect("key file");
    let connects = [Connect {
        name: "nas".to_string(),
        addr: agent.addr.to_string(),
    }];
    let keys = [ConnectKey {
        name: "nas".to_string(),
        source: Source::Path(key_path),
    }];
    let client = Client::start(&connects, &keys, &pins_path).expect("client start");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !client.links[0].connected() {
        assert!(Instant::now() < deadline, "the client never streamed");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // The pin file carries the presented fingerprint and is private.
    let pins = tls::Pins::load(&pins_path).expect("pin file");
    assert_eq!(pins.get("nas").as_deref(), Some(agent.fingerprint.as_str()));
    assert_eq!(
        std::fs::read_to_string(&pins_path).expect("pin file"),
        format!("nas {}\n", agent.fingerprint)
    );
    assert_eq!(mode(&pins_path), 0o600, "the pin file is not private");

    // The same certificate with the stored pin is accepted.
    let (stream, presented) =
        tls::connect(&agent.addr.to_string(), "nas", Some(&agent.fingerprint))
            .await
            .expect("a matching pin is accepted");
    assert_eq!(presented.as_deref(), Some(agent.fingerprint.as_str()));
    drop(stream);

    // A second agent generated its own random key, so its fingerprint differs.
    let rotated = start_agent(&dir.join("rotated")).await;
    assert_ne!(rotated.fingerprint, agent.fingerprint);
    let error = tls::connect(&rotated.addr.to_string(), "nas", Some(&agent.fingerprint))
        .await
        .expect_err("a changed certificate was accepted");
    let message = format!("{error:#}");
    assert!(message.contains("nas"), "{message}");
    assert!(message.contains(&agent.fingerprint), "{message}");
    assert!(message.contains(&rotated.fingerprint), "{message}");
    assert!(message.contains("certificate changed"), "{message}");

    // The refusal never re-pins: the file still holds the old fingerprint.
    let pins = tls::Pins::load(&pins_path).expect("pin file after refusal");
    assert_eq!(pins.get("nas").as_deref(), Some(agent.fingerprint.as_str()));
}

/// ACC-NET-145-TLS-PINNING: a plaintext request on the TLS port gets no HTTP
/// response; the bytes that come back are not an HTTP answer and the handler
/// never runs.
#[tokio::test]
async fn a_plaintext_request_to_the_tls_port_gets_no_http_response() {
    let dir = temp("plaintext");
    let agent = start_agent(&dir).await;

    let mut stream = TcpStream::connect(agent.addr).await.expect("connect");
    stream
        .write_all(
            format!(
                "GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {KEY}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("plaintext request");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        match tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buffer)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(read)) => bytes.extend_from_slice(&buffer[..read]),
        }
    }
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        !text.contains("HTTP/"),
        "a plaintext request got an HTTP response: {text:?}"
    );
    assert!(
        !text.contains("ok"),
        "the health handler answered plaintext: {text:?}"
    );

    // The listener still speaks TLS after the plaintext attempt.
    let mut stream = connect(agent.addr, Some(&agent.fingerprint)).await;
    assert_eq!(get(&mut stream, "/healthz", Some(KEY)).await.0, 200);
}

/// ACC-NET-144-ACCESS-KEY over ACC-NET-145-TLS-PINNING: the access guard runs
/// on the TLS listener, so no key is a 401 with an empty body and the right
/// key streams every fragment.
#[tokio::test]
async fn the_access_key_guards_every_request_over_tls() {
    let dir = temp("access");
    let agent = start_agent(&dir).await;

    for key in [None, Some("wrong-key")] {
        let mut stream = connect(agent.addr, Some(&agent.fingerprint)).await;
        let (status, body) = get(&mut stream, "/healthz", key).await;
        assert_eq!(status, 401, "key {key:?}");
        assert!(body.is_empty(), "a refusal carries a body: {body:?}");
    }

    let mut stream = connect(agent.addr, Some(&agent.fingerprint)).await;
    let (status, body) = get(&mut stream, "/healthz", Some(KEY)).await;
    assert_eq!(status, 200);
    assert_eq!(body, "ok");

    let mut stream = connect(agent.addr, Some(&agent.fingerprint)).await;
    let fragments = events(&mut stream).await;
    assert!(fragments.contains("event: cpu"), "{fragments}");
    assert!(fragments.contains("data-update-ms=\""), "{fragments}");
}

/// ACC-NET-145-TLS-PINNING: the first start generates the pair with the key
/// mode 0600, a later start reuses both files byte for byte, and an operator
/// pair replaces generation without being written anywhere.
#[test]
fn the_agent_generates_once_reuses_and_accepts_an_operator_pair() {
    let dir = temp("identity");
    let cert = dir.join("agent.crt");
    let key = dir.join("agent.key");
    let first = tls::identity_at(&cert, &key).expect("generated pair");
    assert!(first.created);
    assert_eq!(
        first.fingerprint.len(),
        "sha256:".len() + 64,
        "the fingerprint is not sha256"
    );
    assert_eq!(mode(&key), 0o600, "the generated key is not private");
    let cert_bytes = std::fs::read(&cert).expect("certificate");
    let key_bytes = std::fs::read(&key).expect("key");

    let second = tls::identity_at(&cert, &key).expect("reused pair");
    assert!(!second.created);
    assert_eq!(second.fingerprint, first.fingerprint);
    assert_eq!(std::fs::read(&cert).expect("certificate"), cert_bytes);
    assert_eq!(std::fs::read(&key).expect("key"), key_bytes);

    // A pair given by the operator is read from the named paths.
    let given = temp("identity-given");
    let given_cert = given.join("operator.crt");
    let given_key = given.join("operator.key");
    tls::identity_at(&given_cert, &given_key).expect("operator pair");
    let given_cert_source = Source::Path(given_cert.clone());
    let given_key_source = Source::Path(given_key.clone());
    let operator = tls::identity(
        Some(&given_cert_source),
        Some(&given_key_source),
        None,
        None,
    )
    .expect("operator identity");
    assert!(!operator.created);
    assert_eq!(
        operator.cert_source,
        format!("file {}", given_cert.display())
    );
    assert_eq!(operator.key_source, format!("file {}", given_key.display()));

    // One source without its partner, and a half pair on disk, are refused.
    assert!(tls::identity(Some(&given_cert_source), None, None, None).is_err());
    assert!(tls::identity(None, Some(&given_key_source), None, None).is_err());
    let half = temp("identity-half");
    std::fs::copy(&cert, half.join("agent.crt")).expect("copy certificate");
    assert!(tls::identity_at(&half.join("agent.crt"), &half.join("agent.key")).is_err());
}

/// A malformed or unparseable pin file is refused, never read as an empty one
/// that would silently re-pin every agent.
#[test]
fn a_malformed_pin_file_is_refused() {
    let dir = temp("pins-malformed");
    let path = dir.join("known_agents");
    std::fs::write(&path, "nas not-a-fingerprint\n").expect("pin file");
    assert!(tls::Pins::load(&path).is_err());
    std::fs::write(&path, "nas\n").expect("pin file");
    assert!(tls::Pins::load(&path).is_err());
}

/// ACC-NET-146-SECRET-INPUT: an encrypted PKCS#8 key is opened by the
/// passphrase read from its file, and a wrong passphrase is refused naming
/// the TLS key and the source.
#[test]
fn an_encrypted_key_is_opened_by_its_passphrase() {
    let dir = temp("encrypted-key");
    let cert = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tls/agent.crt");
    let key = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tls/agent-encrypted.key");
    let cert_source = Source::Path(cert);
    let key_source = Source::Path(key);

    let right = dir.join("right");
    std::fs::write(&right, "btop-web-test-passphrase\n").expect("passphrase file");
    let identity = tls::identity(
        Some(&cert_source),
        Some(&key_source),
        None,
        Some(&Source::Path(right)),
    )
    .expect("the right passphrase opens the key");
    assert_eq!(identity.fingerprint.len(), "sha256:".len() + 64);

    let wrong = dir.join("wrong");
    std::fs::write(&wrong, "not-the-passphrase\n").expect("wrong passphrase file");
    let error = tls::identity(
        Some(&cert_source),
        Some(&key_source),
        None,
        Some(&Source::Path(wrong)),
    )
    .err()
    .expect("a wrong passphrase");
    let message = format!("{error:#}");
    assert!(message.contains("cannot decrypt TLS key"), "{message}");
    assert!(message.contains("TLS key"), "{message}");
    assert!(message.contains("passphrase"), "{message}");
}

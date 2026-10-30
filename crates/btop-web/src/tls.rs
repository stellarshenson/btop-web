//! ACC-NET-145-TLS-PINNING: TLS 1.3 on every agent listener, and a pinned
//! certificate fingerprint on the client.
//!
//! The agent's pair is generated on the first start with a random ECDSA P-256
//! key and reused on every later start; `--tls-cert` and `--tls-key` (config
//! keys `tls_cert` and `tls_key`) replace generation. The client records the
//! end-entity fingerprint it saw on the first handshake with an agent in
//! `known_agents`, and a later handshake presenting any other certificate is
//! refused with both values named.
//!
//! Both roles configure TLS 1.3 alone through rustls with the ring provider;
//! there is no branch that speaks plain HTTP on an agent listener, and no
//! fallback that accepts a certificate other than the pinned one.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme,
};
use sha2::{Digest, Sha256};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use crate::secret::{self, PassphraseSource, Source};

/// The mode of the agent's generated key file; rcgen's default key is ECDSA
/// P-256, which TLS 1.3 requires for its handshake signatures.
const KEY_FILE_MODE: u32 = 0o600;

/// The directory that holds btop-web's own files, or `None` when neither
/// XDG_CONFIG_HOME nor HOME resolves.
fn state_dir() -> Option<PathBuf> {
    crate::config::config_base_in(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
    .map(|base| base.join("btop-web"))
}

/// The agent's generated certificate:
/// `$XDG_CONFIG_HOME/btop-web/agent.crt`, else
/// `$HOME/.config/btop-web/agent.crt`; `None` when neither is set.
pub fn certificate_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("agent.crt"))
}

/// The agent's generated key, beside [`certificate_path`].
pub fn private_key_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("agent.key"))
}

/// The client's pin file:
/// `$XDG_CONFIG_HOME/btop-web/known_agents`, else
/// `$HOME/.config/btop-web/known_agents`; `None` when neither is set.
pub fn pins_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("known_agents"))
}

/// The certificate fingerprint as the log, the pin file and the refusal show
/// it: `sha256:` and the 32 digest bytes as 64 lowercase hex characters.
pub fn fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    let mut text = String::with_capacity("sha256:".len() + 64);
    text.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// The agent's certificate chain, key and fingerprint, and where they came
/// from.
pub struct Identity {
    /// The chain sent in the handshake; the first certificate is the
    /// end-entity one whose fingerprint is pinned.
    pub certs: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
    pub fingerprint: String,
    /// The source as the log shows it: `file /x` or `descriptor 3`.
    pub cert_source: String,
    pub key_source: String,
    /// Whether this start generated the pair.
    pub created: bool,
}

/// The agent's identity: the operator's pair when both sources are given,
/// otherwise the state pair, generated on the start that finds neither file.
/// ACC-NET-146-SECRET-INPUT: each source is a path or `fd:N`, and an
/// encrypted PKCS#8 key is opened with the passphrase from the named
/// environment variable or from the file or descriptor.
pub fn identity(
    cert: Option<&Source>,
    key: Option<&Source>,
    passphrase_env: Option<&str>,
    passphrase_file: Option<&Source>,
) -> Result<Identity> {
    match (cert, key) {
        (Some(cert), Some(key)) => {
            let certs = read_certs(cert)?;
            let passphrase = match (passphrase_env, passphrase_file) {
                (Some(name), None) => Some(PassphraseSource::Env(name.to_string())),
                (None, Some(source)) => Some(PassphraseSource::File(source.clone())),
                _ => None,
            };
            let key_der = read_key(key, passphrase.as_ref())?;
            Ok(Identity {
                fingerprint: fingerprint(certs[0].as_ref()),
                certs,
                key: key_der,
                cert_source: cert.describe(),
                key_source: key.describe(),
                created: false,
            })
        }
        (None, None) => {
            let cert_path = certificate_path()
                .context("agent mode needs XDG_CONFIG_HOME or HOME to store its TLS certificate")?;
            let key_path = private_key_path()
                .context("agent mode needs XDG_CONFIG_HOME or HOME to store its TLS key")?;
            identity_at(&cert_path, &key_path)
        }
        _ => anyhow::bail!("tls_cert and tls_key must be given together"),
    }
}

/// The pair at explicit paths: both files reused, neither generated, and a
/// half pair refused. [`identity`] calls it for the default paths; tests call
/// it for a temporary directory.
pub fn identity_at(cert_path: &Path, key_path: &Path) -> Result<Identity> {
    let cert = cert_path.exists();
    let key = key_path.exists();
    match (cert, key) {
        (true, true) => {
            let cert_source = Source::Path(cert_path.to_path_buf());
            let key_source = Source::Path(key_path.to_path_buf());
            let certs = read_certs(&cert_source)?;
            let key = read_key(&key_source, None)?;
            Ok(Identity {
                fingerprint: fingerprint(certs[0].as_ref()),
                certs,
                key,
                cert_source: cert_source.describe(),
                key_source: key_source.describe(),
                created: false,
            })
        }
        (false, false) => create_pair(cert_path, key_path),
        (true, false) => anyhow::bail!(
            "{} exists but {} does not; restore the key or delete the certificate to generate a new pair",
            cert_path.display(),
            key_path.display()
        ),
        (false, true) => anyhow::bail!(
            "{} exists but {} does not; restore the certificate or delete the key to generate a new pair",
            key_path.display(),
            cert_path.display()
        ),
    }
}

/// Generate an ECDSA P-256 pair, self-signed, with the host name as its DNS
/// SAN, and write both files. The key is written first with `create_new`: a
/// concurrent first start loses that race and reads the winner's pair, and a
/// crash between the writes leaves a key without a certificate, which the
/// next start refuses with the path in the message rather than replacing a
/// pair a client may already have pinned.
fn create_pair(cert_path: &Path, key_path: &Path) -> Result<Identity> {
    let key_pair = rcgen::KeyPair::generate().context("generate the agent's TLS key")?;
    let hostname = nix::unistd::gethostname()
        .ok()
        .and_then(|name| name.into_string().ok())
        .unwrap_or_else(|| "btop-web-agent".to_string());
    // A host name the DNS-name parser refuses falls back to a literal name;
    // the pin ignores names, so this only keeps other tools quiet.
    let params = rcgen::CertificateParams::new(vec![hostname])
        .or_else(|_| rcgen::CertificateParams::new(vec!["btop-web-agent".to_string()]))
        .context("build the agent certificate parameters")?;
    let cert = params
        .self_signed(&key_pair)
        .context("self-sign the agent certificate")?;
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));
    let cert_der = cert.der().clone();

    let directory = key_path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(directory)
        .with_context(|| format!("create {}", directory.display()))?;
    match write_new(key_path, key_pair.serialize_pem().as_bytes(), KEY_FILE_MODE) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return identity_at(cert_path, key_path);
        }
        Err(error) => {
            return Err(error).with_context(|| format!("write {}", key_path.display()));
        }
    }
    write_new(cert_path, cert.pem().as_bytes(), 0o644)
        .with_context(|| format!("write {}", cert_path.display()))?;
    let certs = vec![cert_der];
    Ok(Identity {
        fingerprint: fingerprint(certs[0].as_ref()),
        certs,
        key: key_der,
        cert_source: Source::Path(cert_path.to_path_buf()).describe(),
        key_source: Source::Path(key_path.to_path_buf()).describe(),
        created: true,
    })
}

/// Create a file that must not exist, with `mode` after the umask is set
/// aside, and flush it to disk.
fn write_new(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// Every certificate in the PEM source, end-entity first.
fn read_certs(source: &Source) -> Result<Vec<CertificateDer<'static>>> {
    let data = secret::read(source, "TLS certificate")?;
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&data)
        .collect::<Result<_, _>>()
        .with_context(|| format!("cannot parse TLS certificate from {}", source.describe()))?;
    anyhow::ensure!(
        !certs.is_empty(),
        "TLS certificate from {} holds no PEM certificate",
        source.describe()
    );
    Ok(certs)
}

/// The private key in the PEM source, PKCS#8, PKCS#1 or SEC1; an encrypted
/// PKCS#8 key is opened with `passphrase`, and refused by name when none is
/// given (ACC-NET-146-SECRET-INPUT).
fn read_key(
    source: &Source,
    passphrase: Option<&PassphraseSource>,
) -> Result<PrivateKeyDer<'static>> {
    let data = secret::read(source, "TLS key")?;
    const ENCRYPTED_LABEL: &[u8] = b"BEGIN ENCRYPTED PRIVATE KEY";
    if data
        .windows(ENCRYPTED_LABEL.len())
        .any(|window| window == ENCRYPTED_LABEL)
    {
        let Some(passphrase) = passphrase else {
            anyhow::bail!(
                "TLS key from {} is an encrypted PKCS#8 key; give its passphrase with --tls-key-passphrase-env or --tls-key-passphrase-file",
                source.describe()
            );
        };
        let pass = secret::passphrase(passphrase)?;
        let text = std::str::from_utf8(&data).with_context(|| {
            format!(
                "cannot parse the encrypted TLS key from {}",
                source.describe()
            )
        })?;
        let (label, document) = pkcs8::SecretDocument::from_pem(text).with_context(|| {
            format!(
                "cannot parse the encrypted TLS key from {}",
                source.describe()
            )
        })?;
        anyhow::ensure!(
            label == "ENCRYPTED PRIVATE KEY",
            "TLS key from {} is not a PKCS#8 encrypted private key",
            source.describe()
        );
        let info: pkcs8::EncryptedPrivateKeyInfoRef<'_> =
            document.decode_msg().with_context(|| {
                format!(
                    "cannot parse the encrypted TLS key from {}",
                    source.describe()
                )
            })?;
        let decrypted = info.decrypt(&*pass).with_context(|| {
            format!(
                "cannot decrypt TLS key from {} with the passphrase from {}; the passphrase is wrong or the encryption is unsupported",
                source.describe(),
                passphrase.describe()
            )
        })?;
        return Ok(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            decrypted.as_bytes().to_vec(),
        )));
    }
    PrivateKeyDer::from_pem_slice(&data)
        .with_context(|| format!("cannot parse TLS key from {}", source.describe()))
}

/// The agent listener's configuration: TLS 1.3 alone through ring, with
/// `http/1.1` as the only ALPN protocol. A TLS 1.2 hello is not answered with
/// a version it can use, so there is no downgrade path.
pub fn server_config(
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<Arc<ServerConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("rustls TLS 1.3 configuration")?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("the TLS certificate and key do not match")?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// The client's configuration: TLS 1.3 alone, `http/1.1` ALPN, and a verifier
/// that pins the end-entity fingerprint. The verifier is returned beside the
/// configuration because it records the fingerprint the server presented.
pub fn client_config(pin: Option<&str>) -> Result<(Arc<ClientConfig>, Arc<PinVerifier>)> {
    let provider = rustls::crypto::ring::default_provider();
    let verifier = Arc::new(PinVerifier::new(pin));
    let mut config = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("rustls TLS 1.3 configuration")?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok((Arc::new(config), verifier))
}

/// The client's remembered fingerprints, one `NAME sha256:HEX` line per
/// agent, written whole under one lock and read before any stream starts.
pub struct Pins {
    path: PathBuf,
    map: Mutex<BTreeMap<String, String>>,
}

impl Pins {
    /// Read the pin file; a missing file is no pins, while an unreadable or
    /// malformed one is an error, never an empty file, because an empty file
    /// would silently re-pin every agent.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("cannot read the pin file {}", path.display()));
            }
        };
        let mut map = BTreeMap::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let line_number = index + 1;
            let Some((name, fingerprint)) = line.split_once(' ') else {
                anyhow::bail!(
                    "{}:{line_number}: expected `NAME sha256:HEX`",
                    path.display()
                );
            };
            anyhow::ensure!(
                valid_fingerprint(fingerprint),
                "{}:{line_number}: `{fingerprint}` is not a sha256 fingerprint",
                path.display()
            );
            map.insert(name.to_string(), fingerprint.to_string());
        }
        Ok(Self {
            path: path.to_path_buf(),
            map: Mutex::new(map),
        })
    }

    /// The pin held for `name`, `None` on a first connect.
    pub fn get(&self, name: &str) -> Option<String> {
        self.map
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(name)
            .cloned()
    }

    /// Remember `fingerprint` for `name`, rewriting the file with mode 0600
    /// and syncing it before the connection's first request.
    pub fn record(&self, name: &str, fingerprint: &str) -> Result<()> {
        anyhow::ensure!(
            valid_fingerprint(fingerprint),
            "`{fingerprint}` is not a sha256 fingerprint"
        );
        let mut map = self.map.lock().unwrap_or_else(|error| error.into_inner());
        // The in-memory map keeps the new pin only after the file write
        // succeeds, so a connection that could not be remembered is retried
        // as an unpinned first connect instead of trusting a pin no restart
        // will know.
        let mut candidate = map.clone();
        candidate.insert(name.to_string(), fingerprint.to_string());
        let mut text = String::new();
        for (name, fingerprint) in candidate.iter() {
            use std::fmt::Write as _;
            let _ = writeln!(text, "{name} {fingerprint}");
        }
        let directory = self.path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(directory)
            .with_context(|| format!("create {}", directory.display()))?;
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        let written = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
            std::fs::rename(&temporary, &self.path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written.with_context(|| format!("cannot write the pin file {}", self.path.display()))?;
        *map = candidate;
        Ok(())
    }
}

/// A fingerprint as [`fingerprint`] writes it: `sha256:` and 64 lowercase
/// hex characters.
fn valid_fingerprint(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// ACC-NET-145-TLS-PINNING: the verifier that replaces the certificate
/// authority. It computes the end-entity fingerprint, accepts it when no pin
/// is held or it equals the held pin, and refuses anything else; chain,
/// hostname and expiry are deliberately not checked, because the pin is the
/// whole trust decision. The handshake signature is still verified, so a pin
/// cannot be replayed by a peer that does not hold the private key.
pub struct PinVerifier {
    /// The remembered `sha256:` fingerprint, `None` on a first connect.
    expected: Option<String>,
    /// The fingerprint the last handshake presented, read after it returns.
    presented: Mutex<Option<String>>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinVerifier {
    /// A verifier holding `pin`, or accepting the first certificate when
    /// `None`; [`client_config`] wraps one in a TLS 1.3 client configuration.
    pub fn new(pin: Option<&str>) -> Self {
        Self {
            expected: pin.map(str::to_string),
            presented: Mutex::new(None),
            algorithms: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        }
    }

    /// The fingerprint the last handshake presented, `None` before one.
    pub fn presented(&self) -> Option<String> {
        self.presented
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

impl fmt::Debug for PinVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinVerifier")
            .field("expected", &self.expected)
            .finish_non_exhaustive()
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let presented = fingerprint(end_entity.as_ref());
        if let Some(expected) = &self.expected
            && !crate::routes::ct_eq(expected.as_bytes(), presented.as_bytes())
        {
            *self
                .presented
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(presented);
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        *self
            .presented
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(presented);
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Connect to an agent over TLS 1.3, pinned to `pin` when one is held. On
/// success the presented fingerprint is returned so the caller can record it
/// before the first request. A presented certificate other than the pinned
/// one fails with both fingerprints named.
pub async fn connect(
    addr: &str,
    name: &str,
    pin: Option<&str>,
) -> Result<(tokio_rustls::client::TlsStream<TcpStream>, Option<String>)> {
    let (config, verifier) = client_config(pin)?;
    let host = host(addr);
    let server_name = ServerName::try_from(host.to_string())
        .with_context(|| format!("agent `{name}`: {addr}"))?;
    let stream = TcpStream::connect(addr)
        .await
        .with_context(|| format!("cannot connect {addr}"))?;
    let connector = tokio_rustls::TlsConnector::from(config);
    match connector.connect(server_name, stream).await {
        Ok(stream) => Ok((stream, verifier.presented())),
        Err(error) => {
            let presented = verifier.presented();
            match (pin, presented) {
                (Some(pinned), Some(presented)) if pinned != presented => anyhow::bail!(
                    "agent `{name}` ({addr}): certificate changed; pinned {pinned}, presented {presented}"
                ),
                _ => Err(error)
                    .with_context(|| format!("agent `{name}` ({addr}): TLS handshake failed")),
            }
        }
    }
}

/// The host part of `HOST:PORT`, without the brackets rustls rejects around
/// an IPv6 literal.
fn host(addr: &str) -> &str {
    let (host, _) = addr.rsplit_once(':').unwrap_or((addr, ""));
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

/// Serve `app` on an already bound listener, TLS 1.3 only, until `shutdown`
/// completes. A peer that writes plaintext fails inside rustls before a
/// request reaches the router, so no handler sees it; the connection is
/// dropped and the accept loop continues.
pub async fn serve(
    listener: TcpListener,
    app: Router,
    config: Arc<ServerConfig>,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<()> {
    let acceptor = TlsAcceptor::from(config);
    tokio::pin!(shutdown);
    loop {
        let (stream, _) = tokio::select! {
            _ = &mut shutdown => return Ok(()),
            accepted = listener.accept() => accepted.context("accept on the agent listener")?,
        };
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let Ok(stream) = acceptor.accept(stream).await else {
                return;
            };
            let service = TowerToHyperService::new(app);
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

# btop-web - Agent and client modes v1

Status: DRAFT for owner review, 2026-10-29.

## Contents

- [1. Overview](#1.-Overview)
- [2. Modes](#2.-Modes)
  - [2.1 Standalone, the default](#2.1-Standalone,-the-default)
  - [2.2 Agent](#2.2-Agent)
  - [2.3 Client](#2.3-Client)
  - [2.4 What crosses the wire](#2.4-What-crosses-the-wire)
  - [2.5 Fit with the SSE fragments](#2.5-Fit-with-the-SSE-fragments)
- [3. Access key](#3.-Access-key)
  - [3.1 Generation](#3.1-Generation)
  - [3.2 Delivery](#3.2-Delivery)
  - [3.3 Storage](#3.3-Storage)
  - [3.4 Comparison](#3.4-Comparison)
  - [3.5 Refusal](#3.5-Refusal)
  - [3.6 Rotation](#3.6-Rotation)
  - [3.7 Failure modes](#3.7-Failure-modes)
  - [3.8 Rejected alternatives](#3.8-Rejected-alternatives)
- [4. Transport](#4.-Transport)
  - [4.1 TLS 1.3 through rustls](#4.1-TLS-1.3-through-rustls)
  - [4.2 The self-signed certificate](#4.2-The-self-signed-certificate)
  - [4.3 Pinning](#4.3-Pinning)
  - [4.4 Operational cost](#4.4-Operational-cost)
  - [4.5 Crates](#4.5-Crates)
  - [4.6 What implementation must confirm](#4.6-What-implementation-must-confirm)
- [5. Secret input](#5.-Secret-input)
  - [5.1 Sources](#5.1-Sources)
  - [5.2 Refusing argv](#5.2-Refusing-argv)
  - [5.3 Printing the key once](#5.3-Printing-the-key-once)
  - [5.4 Headless client](#5.4-Headless-client)
  - [5.5 Errors and memory](#5.5-Errors-and-memory)
- [6. Threat model](#6.-Threat-model)
  - [6.1 The owner's decision](#6.1-The-owner's-decision)
- [7. Implementation plan](#7.-Implementation-plan)
- [8. Open questions](#8.-Open-questions)

## 1. Overview

This note designs two new roles for the binary, an agent that serves telemetry over TLS and a client that renders the dashboard from one or more agents, with the key, certificate and secret handling that carry them. It answers ACC-NET-143-MODES, ACC-NET-144-ACCESS-KEY, ACC-NET-145-TLS-PINNING and ACC-NET-146-SECRET-INPUT.

Three properties of the current server decide most of the design.

- **Age is a browser fact** - `assets/app.js` computes the LIVE, PAUSED and STALE chip and the sample age from the arrival time of each SSE frame, so any process that relays frames shows a truthful age with no new code
- **Fragments are finished HTML** - each event carries `hx-swap-oob` wrappers keyed by panel id (`routes/events.rs`, `collector/frame.rs`), so a relay forwards bytes and parses nothing inside them
- **The page survives an empty snapshot** - `routes/dashboard.rs` renders the default `Snapshot` before the first sample, so a client page can open before its agent has answered

The design keeps one collector, one set of templates and one page, and adds a transport and a second entry point around them.

## 2. Modes

This section defines the three roles the binary can take, which process samples, and what each one serves.

| Role | Selection | Samples its host | Serves | Listens with |
|---|---|:-:|---|---|
| Standalone | no mode flag | yes | page, assets, `/config`, `/events` | plain HTTP, `listen` |
| Agent | `--agent` | yes | `/events`, `/agent`, `/healthz` | TLS 1.3 and the access key |
| Client | `--client` plus one or more `--connect NAME=URL` | no | page, assets, `/config`, `/events?agent=NAME` | plain HTTP, `listen` |

### 2.1 Standalone, the default

No mode flag keeps today's behaviour: one process samples this host and serves the page on `listen`, `127.0.0.1:40290` by default. The criteria's "standalone stays the default" is read literally: nothing on this path changes, so the existing test suite passes unmodified.

### 2.2 Agent

The agent samples its host and serves telemetry only. It carries no page, no assets and no config menu, and it listens on `listen` exactly as standalone does; with the default address it answers only the same host, which is a valid deployment for a client beside it.

- **`GET /events`** - the same five named fragments the standalone server sends, rendered from the same `Frame`
- **`GET /agent`** - flat identity text in btop's own `key = value` form: `version`, `host`, `container`, `interval_ms`. Not JSON. `container` is the runtime token alone (`docker`, `podman`, or `none`), never a display tip, and `host` is the bare host name
- **`GET /healthz`** - `ok` after the key check, as every request passes it

The agent has no mutating route, so the read-only guard is not part of its router.

The identity endpoint exists so the client can name the remote host in the top bar. `index.html` takes `host` and `container` as template fields today, and renders `host` escaped but `container` through `|safe`. The client fills `host` from `/agent`; it builds the container chip tip itself from the runtime token, because remote text must never reach a `|safe` field.

### 2.3 Client

The client samples nothing. It serves the page and relays telemetry from the agents named on the command line.

- **Selection** - `--connect NAME=URL` repeats; `NAME` matches `[A-Za-z0-9._-]+` and a repeat is refused; `URL` must parse as `https://host:port`, and any other scheme stops the client at startup, so a downgrade is a configuration error rather than a cleartext attempt; the client follows no redirects
- **One selected agent per page** - `GET /?agent=NAME` renders the shell for that agent; `GET /events?agent=NAME` relays its fragments; `agent` is matched against the configured names and anything else answers 404, so the parameter never becomes an address
- **Agent strip** - a chip per configured agent in the top bar, the selected one marked. Only the selected agent has an open stream, so it is the only chip with a live age, and the page's existing `#sample-age` carries it; every other chip reads `idle`, because a server-rendered age for an agent with no stream would freeze at its render time
- **Upstream lifetime** - one upstream SSE connection per agent, opened when the first browser subscribes to that agent and dropped when the last one leaves. A client never holds a remote agent awake with nobody watching, because the agent's `Wake` gate parks the sampler when no stream is open. A failed upstream retries with a bounded backoff while a browser stays subscribed, and a 401 ends the retry and marks the agent unauthorized
- **Local settings stay local** - theme, panel show and hide, motion and the read-only guard are the client's own; the interval follows the agent, because the `#update-interval` fragment already carries `data-update-ms`, `data-window-ms` and `data-slow-seq`

The local host is not sampled in client mode. A person who wants the local host on the page runs an agent on it and connects to it, which is the same two processes with the loopback address.

### 2.4 What crosses the wire

This table states every request each role accepts and who may read the answer.

| Role | Request | Body | Auth |
|---|---|---|---|
| Agent | `GET /agent` | four `key = value` lines | bearer key |
| Agent | `GET /events` | named SSE events with HTML fragments | bearer key |
| Agent | `GET /healthz` | `ok` | bearer key |
| Client | `GET /` and `GET /?agent=` | the dashboard page | none, as standalone today |
| Client | `GET /events?agent=` | the selected agent's fragments, re-emitted verbatim | none |
| Client | `GET /assets/*`, `POST /config` | unchanged | unchanged |

The client's browser listener is plain HTTP, as standalone is today. TLS protects the agent link; it does not protect the browser link, and a client exposed beyond loopback needs the operator's own access layer.

### 2.5 Fit with the SSE fragments

The design fits the fragment shape, and one property of it forces the single selected agent.

- **Fits** - fragments are opaque to the tier in the middle; the client forwards each fragment's bytes unchanged, so it needs no collector, no `Snapshot` parser and no second serialisation format
- **Fits** - the browser's age, stale timer and number animation are driven by frame arrival (`app.js`), so relayed fragments animate exactly as local ones do
- **Fits** - one upstream per agent per client keeps ACC-PERF-16-NO-OVERHEAD: N browsers on the client cost one agent stream, and the agent still renders once
- **Does not fit** - fragments carry no agent identity, and panel ids are global; two agents' fragments on one page would overwrite each other's panels. Hence one selected agent, and the strip carries the others
- **Order, not assembly** - the client forwards each upstream event to its browsers in arrival order and buffers nothing; one upstream is one ordered stream whose `cpu` event starts a frame, so a browser receives one sample's events together and never `cpu` from one sample beside `mem` from another, with no added delay; each browser has its own ordered queue, so a slow tab is disconnected rather than fed a partial frame
- **Cadence kept** - forwarding verbatim keeps the agent's `proc` rule of one fragment per slow pass, so the client does not turn a 1 s process table into a 250 ms one; a browser that joins mid-frame fills the remaining panels on the next frame, as a standalone subscriber does

## 3. Access key

This section specifies the bearer secret: how the agent makes it, how the client receives it, how both store it, and how a wrong key is refused.

### 3.1 Generation

- **Source** - `getrandom` fills 32 bytes from the operating system CSPRNG, the same call btop's own entropy users make
- **Size** - 256 bits, encoded base64url without padding as 43 ASCII characters, which fits one terminal line and one config line
- **When** - on the first agent start when no key is supplied; the file is the source of truth on every later start

### 3.2 Delivery

The agent prints the new key once, to the controlling terminal, and names the file that holds it in the startup log. The operator copies it into the client's keys file.

- **Terminal only** - the key goes to `/dev/tty` through `OpenOptions`, never to stdout or stderr, so a run whose output is piped to `tee` or captured by journald does not put the secret in a log
- **No flag prints it later** - "shows it once" is literal; after the first start the key is read from the file, which is why its path is logged and its mode is 0600
- **Cert fingerprint is different** - a certificate fingerprint is public, so the agent logs it at startup and can print it on request

### 3.3 Storage

This table states where each file lives, who writes it and its mode.

| File | Written by | Contents | Mode |
|---|---|---|---|
| `$STATE/access.key` | agent | the 43-character key, one line | 0600 |
| `$STATE/agent.key`, `$STATE/agent.crt` | agent | the generated TLS pair, when none is given | 0600 |
| `$STATE/known_agents` | client | `NAME sha256:HEX` per agent, one line | 0600 in the 0700 dir |
| agent keys file | operator | `NAME = KEY` per agent, read by the client | 0600, checked |

- **Creation** - `OpenOptions::create_new(true).mode(0o600)`, from `std::os::unix::fs::OpenOptionsExt`; std does the job, so no crate and no unsafe
- **State directory** - `$XDG_STATE_HOME/btop-web`, else `$HOME/.local/state/btop-web`, overridden by `--state-dir` or `BTOP_WEB_STATE_DIR`; created 0700
- **Checked on load** - every start re-checks that `access.key` and `agent.key` are no wider than 0600 and stops naming the file otherwise, so a restored backup that lost its mode is refused rather than used; the key file must hold exactly 43 base64url characters
- **Private key creation** - the generated TLS private key goes through the same 0600 `OpenOptions` path as the access key, never `fs::write`, whose mode follows the umask
- **One directory, one agent** - the state directory identifies one agent instance: a second start that also generates into it fails on `create_new` and names the file, while an existing directory is reused, so two agent instances on one host need two `--state-dir` values
- **Not config** - neither the key file nor the pin file is a btop config file; the config menu never rewrites them

### 3.4 Comparison

- **Header** - `Authorization: Bearer <43 characters>`, the only accepted carrier; the key never appears in a query string, a cookie, a form body or a path
- **Length first** - a wrong length is refused by length, which leaks nothing because every valid key has 43 characters
- **Then constant time** - the presented token is base64url-decoded to 32 bytes and compared with `subtle::ConstantTimeEq`; a plain `==` would return at the first differing byte
- **Stored decoded** - the agent decodes its key once at startup and compares bytes, so encoding differences cannot mask a key; a token that does not decode is a 401 on the same path

### 3.5 Refusal

A missing key, a wrong key and a malformed key all get the same answer: `401`, `WWW-Authenticate: Bearer realm="btop-web"`, `Content-Length: 0`, no body. The criterion's test reads the empty body.

The auth layer is the only gate above the agent's handlers and sits below `TraceLayer`, so a refusal is logged with its path and status and still runs before any handler or extractor. It runs before the SSE handler, so a refused `GET /events` never calls `Wake::subscribe` or `Wake::request`; a parked agent stays parked when a stranger knocks. A unit test asserts the subscriber count is zero after a refused request, through a `#[cfg(test)]` accessor on `Wake`.

The span records `uri.path()` without the query string, because `TraceLayer::new_for_http` would otherwise write a misconfigured client's `?key=` value into the log on its way to the 401. The query is never read for authentication, and a second unit test asserts the span carries the path alone.

### 3.6 Rotation

- **`--rotate-key` is a start-up flag** - the agent generates a new 32-byte key, writes a temporary file in the state directory, renames it over `access.key` at 0600, prints it once to the terminal, and only then binds the listener; the flag is refused with `--access-key-file` or `--access-key-fd` and exits 2, because a supplied key is authoritative
- **Streams end with the process** - rotation is a restart, so every open `GET /events` ends with the connection and the new process refuses the old key; there is no in-process generation counter to race
- **Client side** - a client that reconnects with the old key gets 401, stops retrying that agent and shows it as unauthorized rather than looping; the operator replaces the line in the keys file and restarts the client, and live reload is not designed
- **Independent of the certificate** - rotating the key keeps the pin; replacing the certificate keeps the key

### 3.7 Failure modes

This table lists what each bearer-key failure looks like and what catches it.

| Failure | Effect | Control |
|---|---|---|
| Replay of a captured request | Full telemetry read | TLS on every request; the key is the only authenticator and adding request signing would need clocks on both sides for no gain over TLS |
| Key in a log | Permanent disclosure to whoever reads logs | Never logged, never printed to stdout or stderr, redacted `Debug`, no header logging, and trace spans carry the path without the query |
| Key in a URL | Truncated and stored by proxies, browser history | The header is the only carrier; a `key` query parameter is ignored, the request gets 401, and the span logs the path without the query |
| Proxy headers | A spoofed `X-Forwarded-For` must never change the decision | The agent reads no forwarding header; the key decides every request |
| TLS-terminating proxy in front | The pin protects the proxy's certificate | Not supported and documented: the proxy holds the access key, sees plaintext fragments, and the agent's identity stops at the proxy |
| Backup or archive of `$STATE` | Key disclosed | Documented: the state directory is a secret, and backups of it are too; a restored file that lost its 0600 mode is refused at startup |
| Core dump or swap | Key disclosed | `zeroize` on the buffer, no swap control; a host-root attacker wins this and the rest |
| `/proc/PID/environ` | Passphrase disclosed to the same user and to root | The environment form exists only for the passphrase, is named as the weaker option, and the file and fd forms are documented first |

### 3.8 Rejected alternatives

- **Mutual TLS** - a client certificate is a second key of the same strength with a second file and a second pinning problem; the bearer key reaches the same result with one secret, and the client is not a browser so it needs no PKCS#12 story
- **A shared PSK configured on both sides** - that is the bearer key with worse generation: a value a person invents, not 256 bits from the CSPRNG, and rotating it means editing both ends before either restarts
- **No auth on loopback** - the agent binds a network port by definition, and the owner asked for a key; a loopback-only agent cannot serve a client on another host, which is the point of the mode

## 4. Transport

This section specifies TLS on the agent listener, the self-signed certificate, and how the client trusts an agent without a certificate authority.

### 4.1 TLS 1.3 through rustls

- **Version** - TLS 1.3 only, forced with `ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])` and the same on the client; TLS 1.2 is not offered, so a downgrade has no path
- **Provider** - the `ring` provider, chosen over `aws-lc-rs` because `aws-lc-sys` carries OpenSSL-derived licence files that `make deny` is likely to flag, and because the static build then needs no cmake
- **Listener** - `axum-server` with `bind_rustls`, given a prebuilt `rustls::ServerConfig`, keeps `axum::serve`'s shutdown behaviour; the fallback is the tokio-rustls acceptor with `hyper_util`'s connection builder, about 25 lines, using `hyper-util 0.1.21` and `http-body-util 0.1.5` already in the cache
- **ALPN** - `http/1.1` only, the protocol the router is tested against
- **Browser surface** - no agent route is reachable over plain HTTP; the listener has no unencrypted branch at all

### 4.2 The self-signed certificate

When `--tls-cert` and `--tls-key` are both absent, the agent generates its own pair with `rcgen` on first start and reuses it afterwards.

- **Key** - ECDSA P-256, rcgen's default, valid for TLS 1.3
- **SANs** - `gethostname` from `nix` (already a dependency) as a DNS name, plus every local address the net collector's `getifaddrs` path can see as an IP SAN
- **Persistence** - `$STATE/agent.crt` and `$STATE/agent.key`, written once and loaded on later starts; regenerating on every start would change the fingerprint and lock out every pinned client
- **Validity** - ten years. The pinned verifier does not read the validity window, so the dates only keep other tools quiet
- **Fingerprint** - SHA-256 over the DER as `sha256:` plus 64 lowercase hex characters, logged at startup and on `--print-fingerprint`, which reads the certificate file and reveals no secret
- **Files given** - `--tls-cert` and `--tls-key` replace generation; nothing is written to the state directory
- **Passphrase** - an encrypted private key is decrypted with the passphrase from section 5 before rustls sees it, because rustls reads unencrypted PEM only

### 4.3 Pinning

The client verifies the agent's certificate by its fingerprint, with no certificate authority in the path.

- **First connect** - the verifier holds the pin when `$STATE/known_agents` has an entry. With no entry it accepts the end-entity certificate the handshake presents and records its fingerprint, and the connection writes the pin before its first request; a pin file that cannot be written fails the connection rather than trusting an agent it cannot remember
- **Later connects** - a matching pin passes; a different fingerprint stops the connection with an error that carries both the pinned and the presented value
- **Fails closed** - an unreadable or unparseable `known_agents` stops the client; it is never read as an empty file, which would re-pin every agent silently
- **A fingerprint is public** - knowing it grants nothing, because the handshake still needs the private key; the client's trust is its own pin file, not the number
- **No fallback** - there is no `--insecure` flag. Re-pinning takes an explicit `--forget-pin NAME`, so a changed certificate is a deliberate act
- **Verifier** - a `rustls::client::danger::ServerCertVerifier` computes SHA-256 over the end-entity DER, compares it to the pin in constant time, and verifies the handshake signature with the provider's algorithms. It deliberately skips chain, hostname and expiry, which is the meaning of a pin
- **Signature check** - the verifier still verifies the transcript signature, so a pin alone cannot be replayed by an attacker who cannot sign; `supported_verify_schemes` returns the provider's suites, or the handshake fails
- **No unsafe** - the trait is named `danger` but carries no unsafe code, so `unsafe_code = "forbid"` stands

### 4.4 Operational cost

- **Regeneration breaks every client** - a lost state directory, a moved host or a deliberate new pair means re-pinning on each client. The failure is closed, which is the right direction
- **No CA can rescue it** - pinning ignores public trust, so a corporate proxy or an internal CA cannot reissue the certificate; that is the property being bought
- **Renewal is manual** - there is no expiry-driven rotation. A pair is replaced by deleting it and restarting, or by pointing the flags at a new pair
- **A pinned print** - the fingerprint is public and printed in the log at startup, so an operator can compare the client's pin with the agent's print without copying a file
- **One more first-run step** - the first client to reach an agent records the pin; every later client records its own on its own first connect

### 4.5 Crates

The cargo registry cache here holds 133 crates, the current build tree, and no TLS or secret crate at all. The first implementation step therefore needs crates.io access, or a primed cache; this is a gate, not a detail.

| Crate | Purpose | In cache now | Target version |
|---|---|:-:|---|
| `hyper` | server and client core | yes, 1.12.0 | 1.12 |
| `hyper-util` | connection builders, client | yes, 0.1.21 | 0.1.21 |
| `http-body-util` | body collection | yes, 0.1.5 | 0.1.5 |
| `tokio-util` | byte stream helpers | yes, 0.7.20 | 0.7.20 |
| `sha2` | certificate fingerprint | yes, 0.11.1 | 0.11 |
| `nix` | `gethostname` | yes, 0.30.1 | 0.30 |
| `rustls` | TLS 1.3 server and client | no | 0.23.x |
| `tokio-rustls` | TLS over tokio streams | no | 0.26.x |
| `hyper-rustls` | client connector | no | 0.27.x |
| `axum-server` | TLS listener for axum | no | 0.7.x |
| `rustls-pemfile` | PEM parsing for given files | no | 2.x |
| `rustls-pki-types` | DER and PEM key types | no | 1.x |
| `rcgen` | self-signed pair generation | no | 0.13.x |
| `getrandom` | 32 bytes from the CSPRNG | no | 0.3.x |
| `base64` | base64url without padding | no | 0.22.x |
| `subtle` | constant-time compare | no | 2.6.x |
| `zeroize` | wiping key buffers | no | 1.8.x |
| `pkcs8` | encrypted private key decryption | no | 0.11.x, feature `encryption` |
| `eventsource-stream` | SSE parsing on the client | no | 0.2.x |

Versions are targets from the crates' current major lines, not facts confirmed here. `make deny` must pass on the fetched set; `ring` and the RustCrypto crates are ISC, MIT or Apache-2.0, which `deny.toml` allows.

### 4.6 What implementation must confirm

- **`rustls`** - the exact `builder_with_protocol_versions` signature, the `danger::ServerCertVerifier` methods that must be implemented (`verify_server_cert`, `verify_tls12_signature`, `verify_tls13_signature`, `supported_verify_schemes`) and the supported way to reuse the provider's signature verification
- **`rcgen`** - the SAN API (`SanType::DnsName`, `SanType::IpAddress`), `KeyPair::generate` and `self_signed`, and whether the released major is 0.13 or newer, because the constructor moved once already
- **`getrandom`** - `fill` in 0.3 against `getrandom` in 0.2, and whether the crate needs an explicit backend feature
- **`pkcs8`** - that `EncryptedPrivateKeyInfo::decrypt` opens what openssl writes for `-aes256` (PBES2, PBKDF2 HMAC-SHA256, AES-256-CBC)
- **`axum-server`** - that 0.7 builds against axum 0.8 and accepts a prebuilt `ServerConfig`; if not, take the `hyper_util` fallback
- **`eventsource-stream`** - that it accepts a hyper byte stream and is maintained; if not, write the 30-line splitter
- **`hyper-rustls`** - the connector API for a custom `ClientConfig`, and that it does not add its own root store over the pinned verifier
- **Static build** - `make build-static` with `ring` under the pinned toolchain, because the Dockerfile ships a static glibc binary

## 5. Secret input

This section specifies how every secret reaches the process, and what the process may print.

### 5.1 Sources

| Secret | Flags | Notes |
|---|---|---|
| Agent access key | `--access-key-file`, `--access-key-fd` | optional; the agent generates one when neither is given |
| Client agent keys | `connect_key = NAME=/PATH` or `NAME=fd:N` | one line per agent in the configuration file |
| TLS certificate | `--tls-cert PATH`, `--tls-cert-fd N` | a path or an inherited descriptor |
| TLS private key | `--tls-key PATH`, `--tls-key-fd N` | a path or an inherited descriptor |
| Key passphrase | `--tls-key-passphrase-file`, `--tls-key-passphrase-fd`, `--tls-key-passphrase-env NAME` | the environment form is the weakest and is documented last |

- **File** - read whole; a passphrase drops its trailing line ending
- **Descriptor** - read through the `filedescriptor` crate (MIT), which owns the descriptor and closes it after the single read; `File::from_raw_fd` would need unsafe, and `unsafe_code = "forbid"` stands
- **Environment** - `std::env::var_os(name)` for the passphrase only, and the value never enters a `Debug`, a log or an error

### 5.2 Refusing argv

There is no flag that takes a secret value. `--access-key`, `--access-key=VALUE`, `--tls-key-passphrase` and `--tls-key-passphrase=VALUE` are matched by the flag name before the generic unknown-argument arm and exit 2 with a message that names the file and descriptor forms, so a person following an old example gets the correction rather than a value in their shell history. The generic arm never quotes an argument that begins with either name, so a mistyped `=` form cannot echo the secret either.

Any other unknown argument keeps today's refusal and usage text.

### 5.3 Printing the key once

- **When** - only on the start that created the key, and only on `--rotate-key`
- **Where** - the controlling terminal opened as `/dev/tty`; a failed open logs the key file path and prints nothing, so a headless start puts the secret in the file alone
- **Format** - one line with the host name and the 43 characters, preceded by a sentence saying it is shown once and stored in the named file
- **Never** - not through `tracing`, so no `EnvFilter`, log file or journal captures it, and not into the file more than once

### 5.4 Headless client

A client with no terminal reads its keys from `--agent-keys-file` or `--agent-keys-fd`. For systemd, `LoadCredential=` places each agent key as a file under `$CREDENTIALS_DIRECTORY`, and the unit passes `--agent-keys-file "$CREDENTIALS_DIRECTORY/agents"`; no key appears in the unit file, the environment or the process list. A descriptor is passed by shell redirection (`3<keyfile`) or by socket activation, not by `LoadCredential=`.

The agent under systemd reads the same two forms, so a provisioned key replaces the generated one and nothing is printed.

### 5.5 Errors and memory

- **Names the secret** - a missing, unreadable or wrongly permissioned secret stops the process with `<flag> cannot read <path>: <reason>`; a wrong passphrase stops with a line naming the passphrase, as ACC-NET-146-SECRET-INPUT requires
- **Redacted types** - the key lives in a newtype whose `Debug` prints `AccessKey(<redacted>)`, so a panic message or a `tracing` field cannot print it
- **Line numbers, not content** - a malformed line in the client keys file is reported by file and line number, never by the text, which holds the key
- **Zeroized** - `Zeroizing<Vec<u8>>` for the read buffer, the decoded key, the passphrase and the decrypted PKCS#8, so the bytes are wiped when the last owner drops
- **No derive** - the key type has no `Serialize`, no `Display` and no `AsRef<str>`

## 6. Threat model

This section lists what each control stops, what it does not stop, and the decision left to the owner.

| Control | Stops | Does not stop |
|---|---|---|
| Bearer key | Anyone without the key reading telemetry; a wrong or missing key gets 401 before the sampler is touched | A key holder reading everything: authorisation is all or nothing, and a stolen client host holds the key |
| TLS 1.3 on the agent | Reading and changing frames on the wire; a captured request replayed from another host without the key | Traffic volume and timing; compromise of either endpoint |
| Self-signed pair and pin | A party with any public CA certificate impersonating the agent; a silent replacement of the certificate | A man in the middle on the very first connect, which trust on first use accepts; and anyone who can write `$STATE/known_agents`, which is the trust anchor itself |
| 0600 key file and 0700 state directory | Other local users reading the key | Root on the host; a backup taken while the mode was correct still holds the key |
| Terminal-only print and redacted `Debug` | The key in logs, journald, a `tee` pipeline, a panic message, a `/proc/PID/cmdline` | A core dump or swap holding the decoded key; a terminal recorder (`script`, a multiplexer pane log) that copies the one-time print |
| Constant-time compare | A remote timing oracle on the key | Nothing else; it is cheap insurance, not a boundary |
| No forwarding-header trust | A spoofed `X-Forwarded-For` changing a decision | A TLS-terminating proxy, which moves the pin's subject |
| Passphrase by file or fd | The passphrase in `ps`, the shell history or the environment block | The environment form, which is visible in `/proc/PID/environ` |

### 6.1 The owner's decision

**Do standalone and client keep serving an unauthenticated dashboard on a non-loopback address?**

Today `BTOP_WEB_LISTEN=0.0.0.0:40290` serves the page and the event stream to anyone who can reach the port. The new agent mode is protected, and standalone and client are not.

- **Keep as is** - no deployment breaks; two postures coexist, and the documentation says so
- **Refuse at startup** - standalone or client with a non-loopback `listen` exits 2; standalone points at `--agent`, and the client points at the operator's own access layer
- **Warn** - one startup log line; nobody reads logs before the fact

I recommend the refusal, because a dashboard that quietly exposes process names and network counters is the failure this criterion family exists to stop, and the owner's own deployments can be moved. The recommendation is his call because it changes how existing installations start.

The same question applies to client mode: `--client --listen 0.0.0.0:40290` serves every connected agent's telemetry to anyone who can reach the port, with no key at all, so a refusal rule that covers only standalone is defeated by the client. The recommendation covers both modes: a non-loopback `listen` exits 2 and points at the operator's own access layer.

## 7. Implementation plan

This section lists the build order, the files each step touches and the test it carries.

| Step | Files | Tests | Gate |
|---:|---|---|---|
| 0 | `Cargo.toml` and `Cargo.lock` only | none | crates fetch, `make deny`, `make build-static` |
| 1 | `src/secret.rs`, `src/main.rs` (argv refusal) | unit: file, fd, env, missing names the item, permission too wide, argv refused | `make ci` |
| 2 | `src/access.rs` | unit: 32 bytes, 43 characters, 1000 keys distinct, mode 0600, `create_new` fails on a second, a wider mode is refused on load, an invalid length is refused, `ct_eq` equal and differing byte, `Debug` redacted, rotation writes through a temp file and leaves one key | `make ci` |
| 3 | `src/pins.rs` | unit: first connect pins, same fingerprint passes, changed fingerprint reports both, forget re-pins, an unwritable pin fails the connect, a corrupt pin file stops the client | `make ci` |
| 4 | `src/tls.rs` | unit: generated SANs hold the host name, the pair reloads with the same fingerprint, the generated private key file is 0600, an encrypted key without a passphrase names it | `make ci` |
| 5 | `src/routes/agent.rs`, `src/routes/mod.rs` | HTTP: no key, wrong key and malformed key give 401 with an empty body; the right key gives a fragment; a refused request leaves the wake count at zero; a `?key=` request is refused and its span carries the path only | `make ci` |
| 6 | `src/main.rs` (modes, TLS listener, prints) | e2e: an agent starts on TLS with a generated pair, a raw TLS client with the pin reads a frame, a replaced pair is refused | `make ci`, `make deny` |
| 7 | `src/client.rs`, `src/routes/dashboard.rs`, `templates/index.html`, `templates/partials/agents.html`, `assets/app.css` | unit: the relay forwards each event verbatim in order and sends `proc` only on a new slow sequence; e2e: agent plus client, the page shows a panel, the agent name and an age, an unconfigured `?agent=` answers 404, and a `container` value carrying markup arrives escaped | `make ci` |
| 8 | `README.md`, `docs/acc-crit.md` evidence, `INTENT.md` open list | none | `make ci` |

Order and blast radius:

- **Steps 1 to 4 are new files** - no existing module changes, so `graphify affected` has nothing to report until step 5
- **Step 5** - `routes/mod.rs` gains the agent router; `routes::router` keeps its current body for standalone, and the existing routes are untouched
- **Step 6** - `main.rs` gains mode parsing and the listener branch; the standalone branch is today's code moved into a function, not rewritten
- **Step 7** - `dashboard::render` gains the client branch that fills `host` and `update_ms` from `/agent`, builds the container tip from the runtime token, and renders the default `Snapshot`; `index.html` gains the strip include, which renders nothing when the client list is empty, and its `sse-connect` carries `?agent=NAME` in client mode
- **The full suite runs at every step**, so a regression in the standalone path fails at the step that caused it

## 8. Open questions

- **Agent page** - should an agent also serve the dashboard to a browser, so one host is usable without a client? I recommend no: one renderer and one page, and a browser at an agent cannot select a second host
- **Agent strip age** - read literally, ACC-NET-143 asks the strip to show each agent's sample age, but only the selected agent has a stream, so the others read `idle`; a live age for every configured agent costs one permanent stream per agent, which keeps every agent awake and turns the client into a poller. I read the criterion as the selected agent's age and recommend the current rule; if the owner reads it as every chip, the cost is one stream per agent
- **Multi-agent page** - one selected agent per page. Showing two hosts at once needs a second layout and breaks the panel ids; I recommend against it inside this criterion family
- **Client deployment** - the client's own listener is plain HTTP; if it must serve beyond loopback, that is a second TLS listener and a second design
- **State directory in a container** - the distroless image has no writable home, so `BTOP_WEB_STATE_DIR` or `--state-dir` is required; the Dockerfile note belongs to the implementation

# btop-web

![The page](docs/design/shipped-dark.png)

## Contents

- [1. Overview](#1.-Overview)
- [2. Quick start](#2.-Quick-start)
  - [2.1 Container](#2.1-Container)
- [3. Configuration](#3.-Configuration)
  - [3.1 Config menu](#3.1-Config-menu)
- [4. Endpoints](#4.-Endpoints)
- [5. Smooth graph scrolling](#5.-Smooth-graph-scrolling)
- [6. Development](#6.-Development)
- [7. Repository layout](#7.-Repository-layout)
- [8. Exposure](#8.-Exposure)
- [9. Licence and attribution](#9.-Licence-and-attribution)

## 1. Overview

btop-web serves [btop](https://github.com/aristocratos/btop) 1.3.0's telemetry to a browser, from `/proc`, `/sys` and device interfaces.

- **Page** - ten panels, one viewport, desktop or phone; only the process table scrolls
- **Metrics** - btop's source, formula, units, field name; a missing value reads `unavailable`, not 0
- **Plots** - server-built SVG histories, 1 s EMA, scrolling; a lane per GPU and per network direction, split by a 1 px `--border` rule at 55 %; grid lines and corner total labelled in a 27 px gutter (GPU 0/25/50/75/100 %, network 0B, half, ceiling)
- **Readouts** - gauge numbers on the gauge axis, every other live value reserving its widest form; every stat shows value, unit and source on hover, a plot none
- **Processes** - views `top`, `all`, `tree`; sortable; filterable; a CPU history on active rows
- **Display** - eight themes, light and dark, auto/light/dark mode, config menu; GPU legend edge tab, LOAD, MEM and PWR mini bars

## 2. Quick start

```sh
make env
make tools
make run
```

Open <http://127.0.0.1:40290>.

Port 40290 is `btop-web` folded (`v = (v * 256 + byte) mod 64512`) plus 1024; `--config` or `BTOP_WEB_LISTEN` overrides it.

### 2.1 Container

Runs the image; a container shows only its own telemetry.

```sh
make docker
target/release/btop-web --default-config > btop-web.conf
docker run --rm -p 40290:40290 -v "$PWD/btop-web.conf:/etc/btop-web.conf:ro" \
  btop-web:dev --config /etc/btop-web.conf
```

- **Image** - distroless, user `nonroot`, port 40290
- **Bind** - loopback by default; set `listen = 0.0.0.0:40290`
- **Scope** - cgroup v2 CPU, memory, swap, processes, cpuset cores, passed-in GPUs
- **Unavailable** - CPU frequency, temperature, disk IO, network on a shared host namespace
- **Detection** - the log prints `scope=container` with runtime, signal, cgroup and network, or `scope=host`; a CONTAINER chip names them

### 2.2 Agent and client

- **Standalone** - no mode flag: samples this host and serves the page. The default
- **Agent** - `--agent` samples this host and serves `/events`, `/healthz` and `/readyz` only, over TLS 1.3 alone; no page, no assets
- **Agent certificate** - generated self-signed with a random key on the first start at `$XDG_CONFIG_HOME/btop-web/agent.crt` and `agent.key` (mode 0600), reused afterwards; `--tls-cert` and `--tls-key` replace it, and their `-fd` forms read it once from an inherited descriptor; an encrypted PKCS#8 key takes its passphrase from `--tls-key-passphrase-env`, `--tls-key-passphrase-file` or `--tls-key-passphrase-fd`
- **Secrets** - a certificate, key, passphrase or access key is named by a path, a descriptor number or an environment variable name; no secret value is ever a flag
- **Client** - `--client --connect NAME=http://HOST:PORT` samples nothing and serves the page from the named agents over TLS 1.3; `--connect` repeats
- **Pinning** - the client records the agent certificate's `sha256:` fingerprint on the first connect in `$XDG_CONFIG_HOME/btop-web/known_agents` and refuses any other certificate, naming both fingerprints
- **Config keys** - `mode = standalone | agent | client` and one `connect = NAME=http://HOST:PORT` line per agent; the flags outrank both
- **Naming** - `NAME` matches `[A-Za-z0-9._-]` and is unique; the top bar names every agent beside its sample age
- **Selection** - the first configured agent is selected; a chip switches the page with `/?agent=NAME`, and an unknown name answers 404
- **State** - an agent that stops answering reads `unavailable` in its chip, with its last frames held for the page's existing STALE mark; never zeros
- **Streams** - one upstream connection per agent, shared by every viewer: two browsers cost one collection per agent tick, not one per viewer

## 3. Configuration

| Key | Default | Variable | Effect |
|---|---|---|---|
| `listen` | `127.0.0.1:40290` | `BTOP_WEB_LISTEN` | bind |
| `mode` | `standalone` | - | `standalone`, `agent` or `client`; `--agent` / `--client` outrank it |
| `connect` | - | - | `NAME=http://HOST:PORT`, one line per agent; client mode |
| `connect_key` | - | - | `NAME=/PATH` or `NAME=fd:N`, the key source for each client agent |
| `tls_cert`, `tls_key` | generated pair | - | the agent listener's PEM certificate and key; both or neither |
| `theme` | `0` | - | first-paint theme |
| `update_ms` | `250` | `BTOP_WEB_UPDATE_MS` | interval in ms; btop's is 2000 |
| `proc_root` | `/proc` | `BTOP_WEB_PROC_ROOT` | collector root |
| `net_floor` | `1` | - | ceiling floor, Mbit/s |
| `net_fall` | `300` | - | ceiling fall window, s |

Unknown keys are ignored; an invalid value stops startup with file and line. Other keys keep btop's names, defaults and validation.

| Flag | Effect |
|---|---|
| `-c, --config PATH` | read PATH after defaults |
| `-d, --debug`; `-f, --filter TEXT`; `-u, --update MS` | debug log; initial filter; interval 100 to 86400000 ms |
| `--theme N` | 0 Default, 1 Everforest Medium, 2 Flexoki, 3 Gruvbox, 4 Solarized, 5 Synthwave, 6 Terracotta, 7 Ultraviolet; other value stops startup |
| `--proc-view VIEW` | `top`, `all`, `tree`; a stored choice outranks both; else `tree` when `proc_tree`, else `top`; invalid or missing exits 2 |
| `--title TEXT` | tab title, default `btop-web`; blank or missing exits 2 |
| `--readonly`, `--no-readonly` | refuse mutations (default), or allow changes |
| `--agent`; `--client` | telemetry only, or the dashboard from `--connect` agents; the two are exclusive |
| `--connect NAME=URL` | `NAME=http://HOST:PORT`; repeats; client mode; a bad name or URL stops startup |
| `--tls-cert PATH`; `--tls-key PATH`; `--tls-cert-fd N`; `--tls-key-fd N` | the agent listener's PEM pair, from a path or an inherited descriptor; both or neither; without them the agent generates and reuses its own |
| `--access-key-file PATH`; `--access-key-fd N`; `--tls-key-passphrase-env VAR`; `--tls-key-passphrase-file PATH`; `--tls-key-passphrase-fd N`; `--known-agents PATH` | the agent's access key, the encrypted key's passphrase and the client's pin file; a secret value is never a flag value |
| `--default-config`; `-h, --help`; `-v, -V, --version` | print embedded config; usage; version |
| `-b/--boxes`, `-l/--low-color`, `-p/--preset`, `-t/--tty_on`, `--utf-force` | btop's terminal-only flags; exit 2 with `btop-web: <flag> is terminal-only and not applicable` |

### 3.1 Config menu

CONFIG opens seven pages: general, cpu, gpu, mem, net, proc, layout.

- **Display options** - theme, mode, pause, motion, GPU, process view, tree depth, sort, layout; per browser; layout resets
- **Server options** - 41 keys; read-only shows text
- **`--no-readonly`** - each becomes a form, validated, applied in two ticks
- **File write** - one line rewritten in `$XDG_CONFIG_HOME/btop-web/btop-web.conf`, or `~/.config/btop-web/btop-web.conf` when unset; temp file and rename; without a resolvable home a change ends at restart
- **Origin** - a mismatched `Origin` gets 403
- **btop file** - `btop.conf` is the baseline and is never written
- **Precedence** - `-u`, `--theme`, `--proc-view` and `BTOP_WEB_*` outrank both files

## 4. Endpoints

No SPA, no JSON API.

| Request | Response |
|---|---|
| `GET /` | dashboard page; client mode `/?agent=NAME` selects the agent |
| `GET /events` | SSE: `cpu`, `mem`, `net`, `proc`, `gpu`, five per snapshot; client mode `/events?agent=NAME` relays that agent's fragments |
| `GET /agents?agent=NAME` | client mode: the agent strip fragment for the one-second htmx poll; 404 in the server modes |
| `POST /config` | one option as HTML |
| `GET /healthz` | `ok` |
| `GET /readyz` | 200 `ready` once the listener serves |
| `GET /assets/*` | CSS, JavaScript, htmx, icon; sha256 `ETag`, `Cache-Control: no-cache`, 304 on `If-None-Match` match |

## 5. Smooth graph scrolling

- **Sampler** - one sampler publishes a `Snapshot` per `update_ms` on a watch channel; all clients share it
- **History** - `chart.rs` keeps 240 s (960 samples at 250 ms) and smooths each series with a 1 s EMA, alpha = 1 - exp(-update_ms / 1000); readouts stay raw
- **Animation clock** - one `requestAnimationFrame` loop in `app.js` slides each drawn series left by elapsed / `update_ms` x slot; the next frame lands one slot on
- **Path** - each bucket's largest sample is a vertex; the server extends the newest knot one slot at the right edge; PCHIP from `spline1d` adds vertices only when a slot is at least 2 viewBox units wide, which 250 ms does not reach
- **Setting** - CONFIG, general, `motion` on/off; PAUSED, STALE and a hidden tab stop the loop, never the OS reduced-motion query
- **Rescale** - a network plot or the Processes count graph moves to a new ceiling over at least 1 s (`chart.rs` `Scale`); `net_floor`, `net_fall` and `net_sync` set the thresholds
- **SSE** - `routes/events.rs` sends five named fragments per snapshot; a slow client skips ahead

Measured: the sampler costs 8.3-8.8 ms per tick; at 60 fps the animation clock costs 33.1 % of one core against 15.0 % idle.

## 6. Development

Build commands and gates; `make help` lists all.

| Command | Effect |
|---|---|
| `make env`, `make tools` | Rust 1.99.1, edition 2024, rustfmt, clippy; cargo-deny, cargo-llvm-cov, cargo-release, cargo-edit |
| `make build`, `make run` | release binary at `target/release/btop-web`; serves 127.0.0.1:40290; bumps the patch version first (`cargo set-version`) |
| `make build SKIP_VERSION_INCREMENT=1` | skip the bump; also `build-static`, `run`, `docker` |
| `make build-static` | static (`STATIC=1`) glibc binary at `target/x86_64-unknown-linux-gnu/release/btop-web`; `STRIP=1` strips it |
| `make fmt-check`, `clippy`, `test`, `ci` | format check, clippy `-D warnings`, tests, all three |
| `make deny`, `coverage`, `doctor`, `version`, `upgrade`, `clean` | cargo-deny and cargo-llvm-cov (skip when absent; `deny.toml` ignores RUSTSEC-2026-0374); environment report; workspace version; upgrade; clean |
| `make release LEVEL=patch` | `cargo release` dry run; `EXECUTE=1` acts; `none` refuses |

- **Static build** - the NVIDIA panel needs a host glibc equal to the build host's; NVML loads at runtime
- **AMD** - DRM sysfs by default; `FEATURES=rocm` adds ROCm SMI, needs `librocm_smi64`
- **CI** - ubuntu-latest runs fmt-check, clippy, test, cargo-deny, coverage, zizmor, Docker build
- **Tests** - `make ci` runs 332 tests: 233 library, 6 end-to-end, 73 HTTP, 6 performance, 1 font, 13 theme contrast
- **Fixtures** - collector tests use recorded roots; mount paths and the interface list resolve inside the tree; e2e sets `BTOP_WEB_PROC_ROOT`
- **Filter placeholder** - a yellow block on the placeholder is Chromium's find-in-page highlight, not the page's paint

## 7. Repository layout

```text
crates/btop-web/src/collector/
crates/btop-web/templates/
crates/btop-web/assets/
crates/btop-web/themes/
crates/btop-web/config/btop-web.conf
crates/btop-web/tests/fixtures/
```

## 8. Exposure

> [!WARNING]
> btop-web has no authentication and binds loopback by default. The page shows process names, command lines, users, endpoints. Remote access needs a TLS proxy with access control. Read-only blocks mutations but adds no authentication; `--no-readonly` lets anyone change options and the btop-web.conf file.

## 9. Licence and attribution

MIT ([LICENSE](LICENSE)). Borrowed parts keep their own licences and are listed in [NOTICE.md](NOTICE.md):

- **btop** - Aristocratos and contributors, Apache-2.0; the theme files in `crates/btop-web/themes/` are adapted from its `themes/`
- **htmx** - 2.0.11 and htmx-ext-sse 2.2.2, BSD-2-Clause; `crates/btop-web/assets/htmx.min.js` and `htmx-sse.min.js`
- **GalaxaLabsTron Narrow** - OFL 1.1, derived from Orbitron; `crates/btop-web/assets/fonts/`

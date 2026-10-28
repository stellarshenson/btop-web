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
- **Plots** - server-built SVG histories with a 1 s exponential moving average (EMA), scrolling; a load plot per GPU; network down, up
- **Readouts** - gauge numbers centred on the gauge axis, every other live value with its widest form reserved; every stat shows value, unit and source on hover, a plot none
- **Processes** - views `top`, `all`, `tree`; sortable; filterable; a CPU history on active rows
- **Display** - eight themes, light and dark, auto/light/dark mode, panel config menu; GPU legend edge tab, LOAD, MEM and PWR mini bars

## 2. Quick start

```sh
make env
make tools
make run
```

Open <http://127.0.0.1:40290>.

Port 40290 is 1024 plus `btop-web` folded as `v = (v * 256 + byte) mod 64512`; `--config` or `BTOP_WEB_LISTEN` overrides it.

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
- **Detection** - the log prints `scope=container` with runtime, signal, cgroup and network, or `scope=host`; a CONTAINER chip names the three

## 3. Configuration

| Key | Default | Variable | Effect |
|---|---|---|---|
| `listen` | `127.0.0.1:40290` | `BTOP_WEB_LISTEN` | bind |
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
| `--default-config`; `-h, --help`; `-v, -V, --version` | print embedded config; usage; version |
| `-b/--boxes`, `-l/--low-color`, `-p/--preset`, `-t/--tty_on`, `--utf-force` | btop's terminal-only flags; exit 2 with `btop-web: <flag> is terminal-only and not applicable` |

### 3.1 Config menu

CONFIG opens seven pages: general, cpu, gpu, mem, net, proc, layout.

- **Display options** - theme, mode, pause, motion, GPU, process view, tree nodes, sort, layout; per browser; layout resets to defaults
- **Server options** - 36 keys; read-only shows text
- **`--no-readonly`** - each becomes a form, validated and applied in two ticks
- **File write** - the line is rewritten by temp file and rename; without `-c` it ends at restart
- **Origin** - `POST /config` with a mismatched `Origin` gets 403
- **btop file** - `~/.config/btop/btop.conf` is untouched
- **Precedence** - `-u`, `--theme`, `--proc-view` and `BTOP_WEB_*` outrank the file

## 4. Endpoints

No SPA, no JSON API.

| Request | Response |
|---|---|
| `GET /` | dashboard page |
| `GET /events` | SSE: `cpu`, `mem`, `net`, `proc`, `gpu`, five per snapshot |
| `POST /config` | one option as HTML |
| `GET /healthz` | `ok` |
| `GET /readyz` | 200 `ready` after first snapshot, else 503 |
| `GET /assets/*` | CSS, JavaScript, htmx, icon; sha256 `ETag`, `Cache-Control: no-cache`, 304 on `If-None-Match` match |

## 5. Smooth graph scrolling

- **Sampler** - one sampler in `collector/mod.rs` publishes a `Snapshot` per `update_ms` on a watch channel; all clients share it
- **History** - `chart.rs` keeps 240 s (960 samples at 250 ms) and smooths each series with a fixed 1 s EMA, alpha = 1 - exp(-update_ms / 1000); readouts stay raw
- **Animation clock** - one `requestAnimationFrame` loop in `app.js` slides each drawn series left by elapsed / `update_ms` x slot; the next frame lands one slot on
- **Path** - each bucket's largest sample is a vertex; the server extends the newest knot one slot at the right edge; PCHIP from `spline1d` adds vertices only when a slot is at least 2 viewBox units wide, which 250 ms does not reach
- **Setting** - CONFIG, general, `motion` on/off, per browser, on by default; PAUSED, STALE, a hidden tab and reduced motion stop the loop
- **Rescale** - a network plot or the Processes count graph moves to a new ceiling over at least 1 s (`chart.rs` `Scale`); the network ceiling rises after five consecutive ticks, falls after `net_fall` seconds below a tenth, never below `net_floor` Mbit/s, shared under `net_sync`
- **SSE** - `routes/events.rs` sends five named fragments per snapshot; a slow client skips ahead

Measured: the sampler costs 8.3-8.8 ms per tick; at 60 fps the animation clock costs 33.1 % of one core against 15.0 % without motion.

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
- **Tests** - `make ci` runs 263 tests: 198 library, 5 end-to-end, 50 HTTP, 10 theme contrast
- **Fixtures** - collector tests use recorded roots; mount paths and the interface list resolve inside the tree; e2e sets `BTOP_WEB_PROC_ROOT` on the binary

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
> btop-web has no authentication and binds loopback by default. The page shows process names, command lines, users, endpoints. Remote access needs a TLS proxy with access control. Read-only blocks mutations but adds no authentication; `--no-readonly` lets anyone change options and the `-c` file.

## 9. Licence and attribution

Apache-2.0 ([LICENSE](LICENSE)).

- **btop** - Aristocratos and contributors, Apache-2.0; theme files adapted from its `themes/`
- **htmx** - 2.0.11 and its SSE extension, 0BSD

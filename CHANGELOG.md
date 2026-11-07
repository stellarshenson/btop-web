# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.5] - 2026-11-07

### Added

- Panels move while one is dragged by its header: the panels between its old and new place shift one place over before the drop, Escape puts every panel back, and mouse and touch both work
- The CONFIG smoothing setting S, M or L (1, 2 or 4 s, default M, the `smoothing` key in `btop-web.conf`) sets the average every live readout shows, except the process table and the Processes tile; the plots keep their own 1 s average and `/events.json` keeps the raw samples
- `docs/usage.md` holds the usage detail the README no longer carries, including the `/events.json` schema and the performance figures

### Changed

- Tooltips say in one line, two at most, what an element is, with no source, formula or mechanism; the tooltip on the Processes header count names its two numbers, the processes in the list now and all processes
- Live byte and rate readouts show one decimal at a fixed decimal point and change unit up at 1000 and down below 800 of the lower unit, so network rates no longer jump between units every 250 ms
- Mount and GPU memory values reserve the width of `000.0 Byte`, so the percent beside them stays in place while a value falls to 0
- The README states in a few lines that btop-web is for live monitoring of critical systems, as btop is, in a web browser, and keeps only install, run and the main flags; the crates.io page shows it
- In band 3 the GPU and Network panels are equally wide, and storage keeps its width
- Tooltip texts that never change come with the page instead of with every cpu and memory update, a few hundred bytes less per memory update for each client; a 1.0.4 page in client mode still reads every value from a 1.0.5 agent
- Seven dependencies move to their latest patch releases: cc, ctutils, flate2, inout, mime, smallvec and winnow

### Fixed

- On a phone each band takes its height and minimum height from the panels it holds, so a panel dragged into another band is no longer clipped
- The host label's tooltip names the full host name, so a long name cut in the top bar can still be read on windows 1101 px and wider

## [1.0.4] - 2026-11-02

### Added

- The cpu and memory panels are fed as compact data records the page applies in place, instead of out-of-band HTML fragments, using about 0.67 s less renderer CPU per minute with the same pixels, values and cadence
- The page listener serves TLS 1.3 when started with `--tls-cert` and `--tls-key`, in standalone and client mode; without the pair it keeps serving plain HTTP
- The `--port` flag sets the listening port alone; `0` binds an ephemeral port, and the flag outranks the `listen` key's port

### Changed

- Continuous animations advance in steps sized from their own duration, so the page renders at most 25 frames per second of motion, using about 1.0 s less renderer CPU per minute with the resting frame unchanged

### Removed

- The `BTOP_WEB_LISTEN` environment variable; the bind address comes from the `listen` key and the port from `--port`

## [1.0.3] - 2026-11-01

### Added

- The ten-panel dashboard: CPU, memory, processes, cores, GPU, network and storage plus the process table in one viewport, rendered with Askama and updated over SSE with htmx
- btop-parity collectors for CPU, memory, network, processes, storage and GPU, each reading the same `/proc`, `/sys` and device interfaces as btop 1.3.0 with its formula, units, rounding and field name
- Ten themes, each with a light and a dark form, and auto, light and dark modes
- The config menu on seven pages (general, cpu, gpu, mem, net, proc and layout), with browser-stored display options and 41 server options written to `btop-web.conf`
- Agent and client modes over TLS 1.3, with an access key on every agent request and certificate pinning against `known_agents`
- The `/events.json` newline-delimited JSON stream, built from the same snapshot `/events` renders
- The companion Python client `btop-web-client` on the standard library, with `frames()` and the JSON-lines CLI
- The container image: distroless, user `nonroot`, port 40290, with the Docker workflow publishing release tags to `ghcr.io/stellarshenson/btop-web`

### Changed

- The page follows `docs/design/concepts/05.06-network-paired.html`: a fixed top bar, ten panels in four bands, and the network shown once
- The server starts read-write, and `--readonly` refuses every mutating request
- The binary accepts btop's config, update, filter, debug and theme flags; btop's terminal-only flags exit 2
- The default listen address is `127.0.0.1:40290`, derived from the binary name; `--config` or `BTOP_WEB_LISTEN` overrides it
- A metric the collector cannot read renders as `unavailable`, never as zero

### Fixed

- Container mode reports the container's own GPUs, cpuset cores and load, instead of `unavailable`
- Container memory uses docker's working set, `memory.current` minus `inactive_file`, and the panel states one scope, its unit and the host total
- Displayed byte and rate values truncate to three significant digits like btop's `floating_humanizer`
- The process popup names a process that ended instead of freezing its last figures
- Asset URLs carry the build stamp and assets are served with a validator, so a reload or an open tab never mixes old CSS or JavaScript with new fragments
- Charts follow the page's motion toggle alone, so a browser that asks for reduced motion still animates when the toggle is on

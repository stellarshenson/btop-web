# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

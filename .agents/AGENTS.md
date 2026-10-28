<!-- @import /home/lab/.claude/CLAUDE.md -->

# Project-Specific Configuration

This file imports the workstation-wide configuration from `/home/lab/.claude/CLAUDE.md`.
All rules there apply. Project-specific rules below strengthen or extend them.

Depth lives in skills under `~/.claude/skills/` and in `references/` beside that file.

## Mandatory Bans (Reinforced)

The following workspace rules are STRICTLY ENFORCED for this project:

- **No automatic git tags** - only create tags when user explicitly requests
- **No automatic version changes** - only modify version in package.json/pyproject.toml/etc. when user explicitly requests
- **No automatic publishing** - never run `make publish`, `npm publish`, `twine upload`, or similar without explicit user request
- **No manual package installs if Makefile exists** - use `make install` or equivalent Makefile targets, not direct `pip install`/`uv install`/`npm install`
- **No automatic git commits or pushes** - only when user explicitly requests

## Project Context

**btop-web** is a web port of [btop](https://github.com/aristocratos/btop)'s system telemetry.
It reads the same `/proc`, `/sys` and device interfaces as btop and serves the result as a
dashboard for a desktop or a phone. The project lives at `/home/lab/workspace/private/btop-web`.

`INTENT.md` is the handover document. Its non-negotiables outrank completeness, speed and feature
count; read it before collector or UI work.

**Technology stack**

- Rust 1.99.1, edition 2024, pinned by `rust-toolchain.toml`
- Cargo workspace, one member: `crates/btop-web`
- axum 0.8.9, tokio, askama 0.16 for templates, rust-embed for assets
- htmx 2.0.11 with `htmx-ext-sse` 2.2.2, both vendored; no SPA and no JSON API
- CSS custom properties are the only theme layer; eight btop theme files in four light/dark
  pairs (Gruvbox, Solarized, Everforest, Flexoki) plus Default
- `nix` for statvfs and getifaddrs, because `unsafe_code = "forbid"` rules out `libc` FFI

**Layout**

- `crates/btop-web/src/collector/` holds one module per btop collector. Each module documents
  btop's source file and function, the raw source it reads, its formula, units, rounding and
  update period
- `/events` carries named `cpu`, `mem`, `net`, `proc` and `gpu` SSE fragments
- `crates/btop-web/tests/fixtures/` holds recorded `/proc` and `/sys` trees, selected by
  `BTOP_WEB_PROC_ROOT`; collector tests never read the live host
- The Makefile owns every command: `make`, `make ci`, `make deny`, `make coverage`,
  `make docker`, `make run`. `make ci` runs fmt-check, clippy with `-D warnings`, and tests

**State, 2026-10-26**

- `make ci` (fmt-check, clippy `-D warnings`, test) passes: 141 tests - 114 library, 2 end-to-end,
  19 HTTP, 6 theme contrast. `FEATURES` defaults to empty and adds `--features` to clippy, test and
  coverage; rocm is off unless `FEATURES=rocm` (needs `librocm_smi64` and libclang)
- The collectors are implemented against btop 1.3.0: the NVIDIA NVML path is live, and AMD reads DRM
  sysfs by default with the ROCm SMI path behind `FEATURES=rocm`
- The page follows the chosen `docs/design/concepts/05.06-network-paired.html`: a fixed top bar
  (btop-web, version, host, interval, LIVE/PAUSED/STALE with sample age, READ-ONLY, clock, CONFIG),
  ten panels in four bands with network shown once, a GPU load plot with one line and one legend
  entry per GPU in band 3 (then cores, then storage), server-built PCHIP charts over a 1 s EMA, and
  gauges with ticks. The config popover holds five numbered themes (0 Default, then Everforest
  Medium, Flexoki, Gruvbox, Solarized) and removes, adds back and resizes panels; the read-only
  guard is on by default. It serves on the derived port 40290, samples every 250 ms by default and
  accepts btop's config, update, filter, debug and theme flags; btop's terminal-only flags exit 2
- Every text token reaches 4.5:1 on the layers it sits on, composed (chip tints, zebra and hover rows
  included), and `tests/theme_contrast.rs` enforces it for all five pairs in dark and light
- No commit and nothing pushed; origin is set, no workspace submodule entry
- Open: CI on a pushed checkout

## Journal Rules (Project-Specific)

- **APPEND ONLY**: New journal entries MUST be appended at the end of the file, never inserted between existing entries
- Entries maintain strict chronological order by position - the last entry in the file is always the most recent work
- Never reorder, move, or insert entries out of sequence
- The Stellars **journal plugin** is the canonical tool for this file: create via `/journal:create`, append via `/journal:update`, archive via `/journal:archive`. The `journal:journal` skill auto-triggers on any mention of "journal" and runs `journal-tools check` after every write
- Direct edits to `JOURNAL.md` are a last resort - prefer the plugin so modus secundis format, continuous numbering and append-only order are enforced automatically

## Acceptance Criteria, Defects and Edge Cases (Project-Specific)

- **Record before working** - when the user states a new acceptance criterion or a new defect, or a change to one, record it through `/project-management:project-management` in the same turn, before working on it
- **Trackers through `pm-tools` only** - every write to `docs/acc-crit*.md` and `docs/defects*.md` goes through `pm-tools`, never a hand edit
- **Real edge cases** - handle the edge cases a real user or a real input reaches; the ones that are cheap to resolve are handled together, in the smallest change, without debating each one
- **No extreme edge cases** - do not chase extreme or contrived edge cases in code, tests or reviews; each one costs tokens and review rounds and changes nothing a user gets

## Strengthened Rules

- **Collection parity outranks output volume** - every metric uses the same source, method, delta
  window, formula, units, rounding and btop field name as btop. Do not substitute `sysinfo` or
  another generic crate where btop computes the value itself
- **No invented data** - a metric the collector cannot read renders as unavailable, never as zero
- **One module per btop collector** - new collection logic goes in that collector's module and
  records the btop file, function, source, formula and period it mirrors
- **Keep `unsafe` forbidden** - use `nix` or a safe crate for kernel interfaces; never weaken the
  workspace lint
- **The Makefile is the only entry point** - use `make ci` before reporting a code change
  complete; CI runs the same format, clippy and test commands
- **Version bumps live in Cargo manifests** - `[workspace.package].version` in `Cargo.toml` is the
  equivalent of the banned automatic `package.json` and `pyproject.toml` edits
- **Fixtures, not the live host** - collector tests set `BTOP_WEB_PROC_ROOT` to a recorded tree;
  never assert against the kernel that runs the test
- **No JSON API and no SPA** - the server renders Askama templates and htmx consumes the SSE
  fragments; adding either is a design change, not an implementation detail
- **Libraries before own code** - standard functionality comes from a maintained crate or a
  vendored library: interpolation, colour maths and contrast, parsing, formatting, protocols.
  Write our own only when no appropriate solution exists, and record why beside the code
- **One sampler, many clients** - one collection tick per interval is shared by every SSE client;
  the dashboard reports the host's load and never adds to it (ACC-PERF-16-NO-OVERHEAD)
- **House chips, no pills** - every indicator, badge and mode control is the JupyterLab
  device-bridge chip: 1 px border, 2 px radius, uppercase, compact, a 12% tint behind the text.
  Rounded pills are banned, badges and panels included (ACC-VIEW-18-HOUSE-CHIP)
- **One viewport, dense by default** - the whole dashboard fits one screen with no page scroll,
  shows nothing it does not need, and minimises what it cannot drop; empty space is a defect,
  not breathing room. Smaller viewports shrink graphs, then step type and padding down, then drop
  lower-priority fields by a stated order, never by clipping; the page never scrolls
  (DEF-LAYOUT-15-ONE-PAGE, ACC-LAYOUT-20-ADAPTIVE, ACC-LAYOUT-21-PRIORITY, ACC-VIEW-23-RELEVANT)
- **Configuration lives in a config menu** - theme pair, mode and settings are not dashboard
  content; the page body carries telemetry only (ACC-VIEW-19-CONFIG-MENU)
- **Plots stay unobstructed** - never draw a readout, label or control over a chart; the newest
  samples arrive at the right edge and stay visible (ACC-GRAPH-24-UNOBSCURED,
  DEF-GRAPH-18-OBSCURED-PLOT)
- **Reserved width, nothing moves** - a live value with text after it, or in an item its neighbours
  place, reserves the width of its widest form (`data-w` in `app.css`: `100%`, three-digit counts),
  so no label, value or control beside it moves when it changes; a value with nothing after it on
  its line needs none (ACC-LAYOUT-74-RESERVED-WIDTH)

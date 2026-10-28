# Intent

Why btop-web exists, what a good result looks like, and the rules that outrank output volume. Read before executing any scaffold or collector work.

## Purpose

btop is a terminal resource monitor. btop-web serves the same telemetry to a browser, so the same numbers are reachable from any device on the network without a terminal session on the monitored machine.

## The problem

btop's data is visible only inside a terminal on the machine it monitors. A web surface makes that data remote and embeddable, but only if the numbers stay identical. A web port that drifts from btop's own collection is a different product, and a worse one.

## What good looks like

- **Same numbers** - every panel shows what btop shows, for the same interval, with the same units
- **Modern and readable** - a light and a dark theme from day one, a mix of selectable colour themes, typography and spacing that hold up on a desktop browser and on a phone
- **btop's standards** - the repository mirrors the original's practice: a Makefile that owns the build, a README that explains and screenshots, a CONTRIBUTING guide, CI on every push
- **Simple to run** - one binary plus static assets, no database required for live monitoring

## Non-negotiables

These outrank completeness, speed and feature count. Work that cannot satisfy them stops and reports.

- **Collection parity** - collect exactly like btop: the same sources, the same methods. Every metric reads the same `/proc`, `/sys`, ioctl or library call btop reads, with the same delta window, the same formula, the same units and the same rounding, and carries btop's own field names. No generic `sysinfo` substitution where btop computes the value itself
- **One collector module per btop collector** - each module names btop's source file and function it mirrors, the raw source it reads, its formula and its update period
- **Dual theme** - light and dark variants of every theme, driven by CSS custom properties as the single theme layer
- **A mix of colour themes** - at least four selectable themes adapted from btop's own theme collection, each with a light and a dark variant, selected at runtime and persisted
- **Legibility in every combination** - every component is readable in every theme and variant
- **No invented data** - a metric the collector cannot read renders as unavailable, never as zero

## Decisions

- **Name** - `btop-web`, crate and binary `btop-web`, module `btop_web`. Chosen 2026-10-25 over the coinage `bwop`
- **Naming note** - the `t` in `top` is not terminal. `top` is the Unix utility's name; William LeFebvre's original README and the FreeBSD top(1) NAME line describe it as the top CPU processes, and "table of processes" is an uncited backronym. `btop` continues bashtop and bpytop, so its `top` comes from that utility; btop's separate `-t` flag is the one that means TTY
- **Not a generic dashboard** - the product is a web port of btop, held to btop's own collection semantics, not a `sysinfo` demo

## Open

- **Framework** - axum is the working recommendation; the research pass is confirming it against actix-web and the rest
- **Frontend approach** - server-rendered templates with htmx, or a SPA; the research pass is deciding
- **Licence** - match btop's licence unless the Star Colonel decides otherwise
- **First release scope** - proposed: Linux only, one host, live metrics, btop theme files; cross-platform and multi-host monitoring are later
- **Repository** - its own git repository plus a workspace submodule entry and a remote are not created yet

## Handover

- **Research** - workflow run `wf_de3f21b0-ba7`, script `/home/lab/workspace/tmp/workflows/btop-web-research/script.js`, checkpoints in `/home/lab/workspace/tmp/workflows/btop-web-research/checkpoints/` (`r1-repo.json`, `r2-framework.json`, `r3-practices.json`, `r4-prior-art.json`, `plan.json`)
- **Scope of this handover** - scaffold only: repository layout, build, CI, docs, server skeleton, one stub module per btop collector, and the theme token layer. The scaffold must build and pass its own gates; it does not monitor anything yet
- **Next** - the scaffold is handed to the next agent, who fills the collectors and the UI against this document

## Done, first release

- `make` builds one binary and serves the UI on one port
- Every metric matches btop's value on the same host interval by interval, proved against btop 1.3.0 running beside it
- Light and dark variants of at least four colour themes, selectable and persisted, legible throughout
- CI green: fmt, clippy with warnings denied, tests
- README with a screenshot in both light and dark

## Out of scope, first release

- macOS, BSD and Windows collectors
- Multi-host agents, history storage and alerting
- Authentication beyond the lab's own access layer
- Full keyboard and mouse parity with btop's terminal UI

# Acceptance Criteria

First-release criteria for btop-web: the build, collection parity with btop 1.3.0, one module per collector, the theme layer and the README.

## Authors

- `@kj` Konrad Jelen

## Build and release `BUILD`

The release build and the CI gate that admits it

- log: 2026-10-25T02:25:08Z @kj category created

- [x] `ACC-BUILD-1-BINARY` **One release binary** - HIGH; make produces target/release/btop-web as the only file needed to run the dashboard
  - evidence: make exit 0; release binary served /healthz 200, /readyz 200 and 15 SSE frames on 127.0.0.1:18082
  - test: run make, then ./target/release/btop-web, and open the listen address
  - test-tags: E2E
  - mechanism: 2026-10-25T02:25:08Z @kj The release build embeds templates and assets; the binary needs no external files
  - log: 2026-10-25T02:25:08Z @kj added
  - log: 2026-10-25T03:14:13Z @kj closed: verified in the collectors verification, 2026-10-25
- [x] `ACC-BUILD-2-PORT` **One port serves the dashboard** - HIGH; the binary serves the dashboard at the configured listen address; no second port and no sidecar process
  - evidence: one axum listener serves /, /events, /healthz, /readyz and /assets; 15 SSE frames in 5 s on 127.0.0.1:18082
  - test: run the binary and open only the configured listen address
  - test-tags: E2E
  - mechanism: 2026-10-25T02:25:08Z @kj axum binds one listener; the page and the SSE stream share it
  - log: 2026-10-25T02:25:08Z @kj added
  - log: 2026-10-25T03:14:13Z @kj closed: verified in the collectors verification, 2026-10-25
- [ ] `ACC-BUILD-3-CI` **CI gate green** - HIGH; make ci and the GitHub CI job pass format, clippy with warnings denied, and tests
  - test: run make ci and the CI workflow on a clean checkout
  - test-tags: E2E
  - mechanism: 2026-10-25T02:25:08Z @kj The Makefile targets and the GitHub workflow run the same commands
  - log: 2026-10-25T02:25:08Z @kj added
- [x] `ACC-BUILD-30-REPO` **The project lives in a private GitHub repository** - HIGH; the repository exists under the house account and the local tree has it as origin; every push and tag needs the Star Colonel's explicit approval first
  - evidence: git remote -v shows origin https://github.com/stellarshenson/btop-web.git for fetch and push; repository created private under stellarshenson via the GitHub API in the repo workflow
  - test: git remote -v shows the GitHub origin; the repository page exists at github.com/stellarshenson/btop-web
  - test-tags: MANUAL
  - mechanism: 2026-10-25T15:15:34Z @kj created through the GitHub API; origin set on the local repository
  - log: 2026-10-25T15:15:34Z @kj added
  - log: 2026-10-25T22:12:30Z @kj closed
- [ ] `ACC-BUILD-32-RELEASE` **Release work goes through Makefile targets** - MEDIUM; the Makefile prints the version, upgrades dependencies, installs the toolchain and tools, dry-runs a release that acts only with EXECUTE=1, and reports a doctor check; no release, tag or publish happens without explicit approval
  - test: run make help, make version, make doctor and make release; assert the dry run changes nothing
  - test-tags: MANUAL
  - mechanism: 2026-10-25T15:15:34Z @kj cargo-release behind a dry-run default; the house target set
  - log: 2026-10-25T15:15:34Z @kj added
  - log: 2026-10-27T09:52:31Z @kj owner approved 2026-10-27: run make tools to install cargo-release from crates.io; make the GitHub release job manual-only (workflow_dispatch) so no push tags or publishes
- [x] `ACC-BUILD-35-FEATURES` **CI never enables the rocm feature** - HIGH; the GitHub CI workflow never enables the rocm feature: it passes --features only when a FEATURES variable is set, and FEATURES defaults to empty; make build, run and docker honour the same FEATURES value, and the default commands are unchanged when FEATURES is empty
  - evidence: ci.yml has no --all-features; FEATURES empty by default and expands to --features only when set; YAML parses; make ci 110 tests pass; gate.json wf_2458c472-170
  - test: grep .github/workflows/ci.yml for --all-features (absent) and for FEATURES; parse the YAML; run make ci with FEATURES empty and confirm the commands match the previous ones
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T15:17:13Z @kj one FEATURES variable defaults to empty in the workflow and the Makefile; the feature flag is added only when FEATURES has a value, so a runner without librocm_smi64 never enables rocm
  - log: 2026-10-25T15:17:13Z @kj added
  - log: 2026-10-25T16:24:23Z @kj closed

## Collection parity `PARITY`

Every metric matches btop 1.3.0 from the same source; an unreadable metric is never invented

- log: 2026-10-25T02:25:11Z @kj category created

- [x] `ACC-PARITY-4-SOURCES` **Same source and formula as btop** - CRITICAL; every metric reads the same /proc, /sys, ioctl or library call as btop, with the same delta window, formula, units, rounding and field name
  - evidence: 70-field audit vs btop 1.3.0: 0 missing; 35 divergences all resolved in divergences.json, 0 unresolved; R02 and R34 fixed with tests; AMD DRM rows recorded as no btop counterpart, RSMI behind FEATURES=rocm; make ci 119 tests
  - test: compare each collector read with btop's source for that field, then diff values on a live host
  - test-tags: UNIT, INTEGRATION
  - mechanism: 2026-10-25T02:25:11Z @kj Each collector module mirrors one btop file and records the source, formula and period
  - log: 2026-10-25T02:25:11Z @kj added
  - log: 2026-10-25T20:58:08Z @kj closed
  - log: 2026-10-27T11:51:31Z @kj owner chose 2026-10-27 (review round 2 deferral): keep btop's default use_fstab = true; this host's empty /etc/fstab lists no disks, as btop 1.3.0 does; the live instance runs without a disk config
- [x] `ACC-PARITY-5-MATCH` **Values match btop 1.3.0** - CRITICAL; on the same host and interval, every displayed metric equals btop 1.3.0's value under btop's rounding
  - evidence: Live 306 string comparisons after the humanizer port: 0 formatting differences, 0 static mismatches; GPU totals 51/51, memory total and swap 51/51; residual differences are sampling-window artefacts with measured offsets; verify gate PASS
  - test: run btop 1.3.0 beside btop-web and compare each panel interval by interval
  - test-tags: MANUAL
  - mechanism: 2026-10-25T02:25:12Z @kj Same sources, delta window and rounding as btop, so values agree without conversion
  - log: 2026-10-25T02:25:12Z @kj added
  - log: 2026-10-25T16:17:11Z @kj Live measurement 2026-10-25: 45 phase-aligned main pairs (offsets -26..+111 ms), 14 GPU pairs (offsets -794..+596 ms); CPU/load/mem percent/swap percent/process count match; display rounding divergence in human() vs btop floating_humanizer; GPU lane sampling artefacts; checkpoint tmp/workflows/btop-web-parity6/checkpoints/live.json
  - log: 2026-10-25T20:58:08Z @kj closed
- [x] `ACC-PARITY-6-UNAVAILABLE` **Unreadable metric shows unavailable** - CRITICAL; a metric the collector cannot read renders as unavailable, never as zero
  - evidence: fixture run renders battery, temperature, frequency, CPU, RAM, swap, disk, network and GPU as 'unavailable', never zero; verify.json gate PASS
  - test: point a collector at a missing fixture file and assert the panel says unavailable
  - test-tags: UNIT, INTEGRATION
  - mechanism: 2026-10-25T02:25:12Z @kj Collector fields are Option<T>; the template renders None as unavailable
  - log: 2026-10-25T02:25:12Z @kj added
  - log: 2026-10-25T03:14:13Z @kj closed: verified against the unavailable fixture tree, 2026-10-25

## Collector structure `COLL`

One module per btop collector, with its provenance recorded

- log: 2026-10-25T02:25:16Z @kj category created

- [x] `ACC-COLL-7-MODULE` **One module per btop collector** - HIGH; each btop collector has one module naming btop's source file and function, raw source, formula and update period
  - evidence: cpu.rs, mem.rs, net.rs, proc.rs and gpu.rs match btop's five collector namespaces and each names its btop functions, source, formula and period
  - test: list btop's collectors and assert one module each with its provenance header
  - test-tags: MANUAL
  - mechanism: 2026-10-25T02:25:16Z @kj The module doc comment carries the provenance; one collector returns one snapshot section
  - log: 2026-10-25T02:25:16Z @kj added
  - log: 2026-10-25T03:14:13Z @kj closed: verified against the five btop collector namespaces, 2026-10-25
- [-] `ACC-COLL-51-OVERSAMPLE` **Sample faster than the display updates** - LOW; owner proposed 2026-10-26: collect samples faster than the page updates and show each update as the average of its samples; counters (CPU jiffies, network and disk bytes) already give the interval average in one delta; open until the owner decides
  - test: unit: an update equals the mean of its sub-samples for each instant gauge; perf: one shared sampler, collection cost per update measured
  - test-tags: UNIT
  - log: 2026-10-26T21:28:50Z @kj added
  - log: 2026-10-26T21:29:29Z @kj rejected: owner dropped it 2026-10-26: CPU, network and disk values are counter deltas that already equal the interval average, NVML and loadavg are averaged at the source, plots show a 1 s EMA; extra reads would only add host load

## Theme `THEME`

Selectable light and dark theme pairs, applied only through CSS custom properties

- log: 2026-10-25T02:25:16Z @kj category created

- [x] `ACC-THEME-8-PAIRS` **Four selectable theme pairs** - HIGH; at least four themes from btop's collection, each with light and dark variants, selected at runtime and persisted
  - evidence: headless Chrome against the release binary: 5 options (Default plus Everforest Medium, Flexoki, Gruvbox, Solarized) each applied, survived a reload and rendered distinct light and dark palettes; the mode toggle flips and persists after reload; 8 of 8 checks passed
  - test: select each theme pair, reload, and assert the choice persists with both variants
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T02:25:16Z @kj Theme files are embedded btop palettes; the selection is stored and applied to custom properties
  - log: 2026-10-25T02:25:16Z @kj added
  - log: 2026-10-25T07:08:38Z @kj closed
- [x] `ACC-THEME-9-TOKENS` **CSS custom properties are the only theme layer** - MEDIUM; every themed value comes from a CSS custom property; no colour literal sits outside the token layer
  - evidence: grep of templates and assets for hex, rgb() and hsl() literals returns no match; the only colour function is color-mix(var(--warn) 12%, transparent); themed values come from the :root token blocks render_css emits, and the contrast tests read the same tokens
  - test: grep the stylesheet for colour literals outside the token definitions
  - test-tags: UNIT
  - mechanism: 2026-10-25T02:25:16Z @kj One render step writes the palette into :root; components read tokens only
  - log: 2026-10-25T02:25:16Z @kj added
  - log: 2026-10-25T07:06:15Z @kj closed
- [x] `ACC-THEME-10-CONTRAST` **Every component legible in every theme** - HIGH; text and controls meet WCAG AA contrast in all four theme pairs and both variants
  - evidence: ten theme and mode combinations at 1440x900 with the popover open and closed: 0 below-floor runs; worst 4.512:1 page and 4.539:1 popover; composed-layer test passes
  - test: run the theme contrast test and inspect every panel in each pair
  - test-tags: UNIT, MANUAL
  - mechanism: 2026-10-25T02:25:16Z @kj Semantic tokens are derived to the contrast floor; raw btop palette values stay verbatim
  - log: 2026-10-25T02:25:16Z @kj added
  - log: 2026-10-25T13:49:42Z @kj kept open: the page passes at 4.512:1 worst case, the config popover still fails below 4.5:1, tracked as DEF-THEME-22-POPOVER-CONTRAST
  - log: 2026-10-25T14:01:48Z @kj closed
- [x] `ACC-THEME-27-NUMBERED` **Themes are numbered from zero** - HIGH; every shipped theme has a stable number from 0, where 0 is Default; --theme N and a theme key select the startup theme; a stored browser choice wins; out-of-range stops startup and lists the range
  - evidence: --theme 0..4 render default, everforest-medium, flexoki, gruvbox, solarized in raw HTML and a fresh browser; --theme 9 exits 1 listing 0..=4; stored pick wins
  - test: start with --theme 0 to N and assert the page data-theme; start with an out-of-range number and assert refusal with the valid range
  - test-tags: UNIT, E2E
  - mechanism: 2026-10-25T08:01:10Z @kj one ordered theme list shared by the picker, the config and the flag; 0 is Default
  - log: 2026-10-25T08:01:10Z @kj added
  - log: 2026-10-25T08:32:59Z @kj edited text "every shipped theme has a stable number, indexed from 0, where 0 is Default; the number selects the startup theme through --theme N and a theme key in the config, and the page starts in it; a number outside the shipped range stops startup and lists the valid range" -> "every shipped theme has a stable number from 0, where 0 is Default; --theme N and a theme key select the startup theme; a stored browser choice wins; out-of-range stops startup and lists the range"
  - log: 2026-10-25T08:41:40Z @kj closed
- [x] `ACC-THEME-65-STEEL-DEFAULT` **Steel colours for the default theme** - HIGH; default theme is not black and fits JupyterLab: dark mode uses Steel Dark surfaces, page background #272b31 (Steel Dark --jp-layout-color0 #252b32); light mode mostly the standard JupyterLab light theme with Steel Light hints; system light/dark picks the mode; other colours modernised a little; text keeps 4.5:1
  - evidence: dark body rgb(39, 43, 49) = #272b31, light white JupyterLab-light page, saturation capped at 70%; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - test: fresh viewer, system dark: body background #272b31; system light: white JupyterLab-light page with Steel Light neutrals; tests/theme_contrast.rs green
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T13:32:22Z @kj added
  - log: 2026-10-27T13:33:09Z @kj amended text "default theme background is not black: dark mode page background #272b31 (Galaxa Steel Dark --jp-layout-color0 is #252b32), light mode takes Steel Light surfaces, chosen by the system light/dark setting; other default colours modernised a little toward Steel; every text token keeps 4.5:1" -> "default theme is not black and fits JupyterLab: dark mode uses Steel Dark surfaces, page background #272b31 (Steel Dark --jp-layout-color0 #252b32); light mode mostly the standard JupyterLab light theme with Steel Light hints; system light/dark picks the mode; other colours modernised a little; text keeps 4.5:1"
  - log: 2026-10-27T20:29:20Z @kj edited test "fresh viewer, system dark: body background #272b31; system light: Steel Light background; tests/theme_contrast.rs green" -> "fresh viewer, system dark: body background #272b31; system light: white JupyterLab-light page with Steel Light neutrals; tests/theme_contrast.rs green"
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint
- [x] `ACC-THEME-66-BOX-BORDERS` **Muted panel borders in the default theme** - HIGH; default theme panel borders use no neon or saturated colour; each box border is the Steel border colour with a small hue shift per panel, so panels stay distinguishable
  - evidence: Default borders: Steel border at 20% saturation, hue shifts -30/-10/+10/+30 deg, no 3:1 lift; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - related: ACC-THEME-65-STEEL-DEFAULT
  - test: fresh viewer, both modes: every panel border colour within a small hue and saturation distance of the Steel border colour; screenshot reviewed
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T13:32:30Z @kj added
  - log: 2026-10-27T20:29:20Z @kj steel-theme spec C1: Default panel borders carry no 3:1 floor (1.65-2.42:1), per the owner's no-neon, small-hue-shift rule; panels keep their titles; other theme pairs keep the 3:1 lift
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint
- [x] `ACC-THEME-69-MODE-AUTO` **Mode setting: dark, light, auto** - HIGH; the config menu mode control offers dark, light and auto; auto follows the system light/dark setting and switches live when it changes; auto is the default for a fresh viewer; the choice is stored per viewer
  - evidence: mode row dark, light, auto; auto default and live on a scheme change; choice kept on reload; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - related: ACC-THEME-65-STEEL-DEFAULT
  - test: config menu shows three mode options; auto with emulated prefers-color-scheme dark then light switches the page without reload; reload keeps the choice
  - test-tags: FUNCTIONAL
  - log: 2026-10-27T18:31:54Z @kj added
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint

## Documentation `DOCS`

The README explains the product and shows both themes

- log: 2026-10-25T02:25:16Z @kj category created

- [x] `ACC-DOCS-11-SHOTS` **README screenshots in light and dark** - MEDIUM; the README shows the dashboard in both the light and the dark variant
  - evidence: README Fig 1 and Fig 2: docs/design/shipped-dark.png and shipped-light.png at 1440x900 with the GPU load panel; tmp/workflows/btop-web-gpu-load/checkpoints/final-1.json check 5
  - test: open the README and assert both images render
  - test-tags: MANUAL
  - mechanism: 2026-10-25T02:25:16Z @kj Screenshots are committed and referenced by relative path from the README
  - log: 2026-10-25T02:25:16Z @kj added
  - log: 2026-10-26T18:59:43Z @kj closed
- [x] `ACC-DOCS-31-README` **The README explains the product truthfully** - HIGH; the README says what btop-web is, how to run it, what it configures and what it refuses, in the house technical style, with no claim the tree contradicts: test counts, port, flags, themes, endpoints and licence all match
  - evidence: every command, port, count, flag, endpoint and licence in README.md matches the tree; make help claim true; tmp/workflows/btop-web-implement05/checkpoints/final.json check 2, tmp/workflows/btop-web-gpu-load/checkpoints/final-1.json check 5
  - test: open README.md and check each stated command, port and count against the tree
  - test-tags: MANUAL
  - mechanism: 2026-10-25T15:15:34Z @kj numbered sections, a commands table and a status line refreshed whenever a gate or a count moves
  - log: 2026-10-25T15:15:34Z @kj added
  - log: 2026-10-26T18:59:43Z @kj closed

## Safety and controls `SAFE`

Read-only operation and the actions that change the system

- log: 2026-10-25T02:29:16Z @kj category created

- [x] `ACC-SAFE-12-READONLY` **Read-only forbids system changes** - CRITICAL; with --readonly, the server refuses kill, signal, priority and every other mutating request, whatever the client sends
  - evidence: release binary with --readonly refused 12/12 POST, PUT, PATCH and DELETE requests to /, /events and /processes/1/kill with 403 and the refusal body; GET / and the five SSE events still 200; --no-readonly returns 405/404 instead of 403; 7 http tests and 88 total green
  - test: start with --readonly, send each mutating request, assert refusal and unchanged process state
  - test-tags: INTEGRATION, E2E
  - mechanism: 2026-10-25T02:29:16Z @kj A server-side guard rejects every mutating route when the flag is set; the UI is not the boundary
  - log: 2026-10-25T02:29:16Z @kj added
  - log: 2026-10-25T07:01:09Z @kj closed
- [x] `ACC-SAFE-13-READONLY-UI` **Read-only hides mutating controls** - MEDIUM; with --readonly, the UI hides or disables every control that would change the system and states why
  - evidence: index passes AppState.readonly; the header renders the READ-ONLY house chip with aria-describedby and the visible reason, absent under --no-readonly; http test and live release binary on 127.0.0.1:18092 prove both bodies; make ci 90 tests green
  - test: start with --readonly and inspect each process row and panel control
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T02:29:16Z @kj Templates receive the read-only flag and render the controls disabled with a reason
  - log: 2026-10-25T02:29:16Z @kj added
  - log: 2026-10-25T07:05:58Z @kj closed

## GPU display `GPU`

Per-device GPU telemetry and the device selection

- log: 2026-10-25T03:24:30Z @kj category created

- [x] `ACC-GPU-14-ALL-DEVICES` **Every GPU shown by default** - HIGH; the GPU panel shows load, memory, temperature and power for every GPU the collector reports; the default selection is all devices, and a changed device count appears without a restart
  - evidence: the live host renders 3 GPU boxes by default; the template loops the collector's devices; a tree without a GPU renders the one-line unavailable panel instead
  - test: with the three NVIDIA GPUs present, open the GPU panel and assert one row per device; remove a device from the fixture and assert the row disappears
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T03:24:30Z @kj The collector returns one entry per NVML index; the template iterates every entry and defaults the selection to all
  - log: 2026-10-25T03:24:30Z @kj added
  - log: 2026-10-25T07:54:57Z @kj closed
- [x] `ACC-GPU-15-SELECTION` **GPU selection persists** - MEDIUM; a control narrows the GPU panel to the selected devices and the choice survives a reload
  - evidence: clicking GPU 1 hides it, writes [0,2] to btop-web-gpu-selection, and 2 boxes remain after reload; the last visible GPU cannot be hidden
  - test: select GPU0 and GPU2, reload, assert only those two rows are drawn
  - test-tags: E2E
  - mechanism: 2026-10-25T03:24:30Z @kj The selection is stored with the theme in guarded localStorage and applied to the panel
  - log: 2026-10-25T03:24:30Z @kj added
  - log: 2026-10-25T07:54:57Z @kj closed
- [x] `ACC-GPU-70-ASYNC-READ` **GPU readings never delay the update** - HIGH; GPU readings are fetched asynchronously; when a reading is not ready at a tick, the tick goes out on time with the newest completed reading and the late one is used when it arrives
  - evidence: built already: collector/gpu.rs:41-46, one NVML sweep in flight off the tick, each snapshot takes the newest completed sweep (a sweep of 3 cards takes 0.2-1.3 s); round-2 final gate 4 with the sweep running: 80 of 80 ticks in 20 s for 1 and 10 clients, tmp/workflows/btop-web-round2-fix/checkpoints/final.json
  - test: at 250 ms with the NVML sweep running, 20 s gives 80 ticks for 1 and 10 clients
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T18:58:50Z @kj added
  - log: 2026-10-27T18:58:51Z @kj closed

## Performance `PERF`

Sampling cost, render cost, and the dashboard's effect on the host it measures

- log: 2026-10-25T03:38:07Z @kj category created

- [x] `ACC-PERF-16-NO-OVERHEAD` **The dashboard does not load the host it measures** - CRITICAL; collecting and rendering must stay negligible: one sampler tick per interval, shared by every client; static facts cached; no busy loops; N clients cost no extra collection. The reported CPU, memory and GPU values do not move because the dashboard runs. The measured baseline and the bound are recorded
  - evidence: One watch channel sampler (mod.rs:486-500, main.rs:118); /events clones the receiver only. Idle, 1 and 10 clients: 120 ticks each in 60 s, delta 0; collection 1.92 ms/tick; added CPU 0.26 s vs 2.30 s for ten collectors; verify run reproduced 40/40 ticks
  - test: run the host idle, then with btop-web and 10 SSE clients; compare the host CPU time used by btop-web and the reported CPU total; the sampler stays under the recorded bound and reported values agree within noise
  - test-tags: FUNCTIONAL, E2E, MANUAL
  - mechanism: 2026-10-25T03:38:07Z @kj one tokio interval drives one collect per tick; the snapshot is built once and fanned out to every SSE subscriber; static data is read once and refreshed slowly; rendering stays in the browser; a repeatable harness records the overhead
  - log: 2026-10-25T03:38:07Z @kj added
  - log: 2026-10-25T22:11:15Z @kj closed

## Graphs and sparklines `GRAPH`

Line graphs, sparklines and history charts: interpolation, marks and scale

- log: 2026-10-25T03:38:23Z @kj category created

- [x] `ACC-GRAPH-17-PCHIP-SMOOTH` **Graphs are smoothed with PCHIP** - HIGH; every line graph, sparkline and history chart interpolates between samples with monotone piecewise cubic Hermite (PCHIP): knots land exactly on their timestamps, a monotone run stays monotone, and the curve never overshoots the sample extremes. No jagged segment joins at the 2 s sample rate. The curve comes from the spline1d crate's PCHIP (MIT or Apache-2.0), never hand-written interpolation; whichever of PCHIP and a bicubic scheme measures cheaper on a 120-knot rebuild wins, a tie goes to PCHIP
  - evidence: every chart is an SVG path built by chart.rs from spline1d's PCHIP; unit tests pin exact knots, monotone data and no overshoot; live frames carry 34 paths; no hand-written interpolation
  - test: unit-test the interpolator on rising, falling, flat and alternating series: knots exact, no overshoot outside the sample range, monotone data stays monotone; then compare a rendered sparkline with a polyline at the same samples
  - test-tags: UNIT, FUNCTIONAL, E2E
  - mechanism: 2026-10-25T03:38:23Z @kj PCHIP slopes from the Fritsch-Carlson limiter; the curve is emitted as an SVG path sampled per pixel column; knots keep their exact x and y
  - log: 2026-10-25T03:38:23Z @kj added
  - log: 2026-10-25T03:38:57Z @kj edited text "every line graph, sparkline and history chart interpolates between samples with monotone piecewise cubic Hermite (PCHIP): knots land exactly on their timestamps, a monotone run stays monotone, and the curve never overshoots the sample extremes. No jagged segment joins at the 2 s sample rate" -> "every line graph, sparkline and history chart interpolates between samples with monotone piecewise cubic Hermite (PCHIP): knots land exactly on their timestamps, a monotone run stays monotone, and the curve never overshoots the sample extremes. No jagged segment joins at the 2 s sample rate. Whichever of PCHIP and a bicubic or natural cubic scheme measures cheaper on a 120-knot rebuild wins; a tie goes to PCHIP"; reason: owner's tie-break rule 2026-10-25
  - log: 2026-10-25T03:51:34Z @kj edited text "every line graph, sparkline and history chart interpolates between samples with monotone piecewise cubic Hermite (PCHIP): knots land exactly on their timestamps, a monotone run stays monotone, and the curve never overshoots the sample extremes. No jagged segment joins at the 2 s sample rate. Whichever of PCHIP and a bicubic or natural cubic scheme measures cheaper on a 120-knot rebuild wins; a tie goes to PCHIP" -> "every line graph, sparkline and history chart interpolates between samples with monotone piecewise cubic Hermite (PCHIP): knots land exactly on their timestamps, a monotone run stays monotone, and the curve never overshoots the sample extremes. No jagged segment joins at the 2 s sample rate. The curve comes from the spline1d crate's PCHIP (MIT or Apache-2.0), never hand-written interpolation; whichever of PCHIP and a bicubic scheme measures cheaper on a 120-knot rebuild wins, a tie goes to PCHIP"; reason: library-first policy and tie-break from the owner 2026-10-25
  - log: 2026-10-25T07:54:57Z @kj closed
- [x] `ACC-GRAPH-24-UNOBSCURED` **The newest samples stay visible** - HIGH; no label, readout, legend or control is drawn over a plot. The right edge, where new samples enter, stays clear; values sit in a line above the chart or in a column beside it. No overlay follows the pointer
  - evidence: net readouts sit in header lines; elementFromPoint at the right edge of each plot returns the chart path, so nothing covers the newest samples
  - test: open every chart at 1440 and 390; assert no element overlaps the plot area and the last sample is drawn at full height at the right edge
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T06:08:37Z @kj readouts are header rows outside the plot; the sparkline and history components render an empty right margin when a value must sit beside the curve
  - log: 2026-10-25T06:08:37Z @kj added
  - log: 2026-10-25T07:54:57Z @kj closed
  - log: 2026-10-26T15:01:47Z @kj 2026-10-26 scope: a gauge readout may sit in the arc's empty centre (ACC-GRAPH-38-GAUGE-TICKS); time-series plots keep no text over them
  - log: 2026-10-26T15:01:52Z @kj correction: the gauge criterion is ACC-GRAPH-42-GAUGE-TICKS; the previous log named ACC-GRAPH-38 in error
  - log: 2026-10-28T10:10:23Z @kj overridden-by ACC-GRAPH-73-STAT-OVERLAY removed - override removed, ACC-GRAPH-73 rejected
- [x] `ACC-GRAPH-38-TEMPORAL` **Temporal smoothing on every graph** - HIGH; Every time-series graph and sparkline plots an exponentially smoothed series, time constant 1 s, applied to the samples before the PCHIP curve; readout numbers stay btop's raw values; owner asked 2026-10-26
  - evidence: wf_c29f79db-a67 verify-2 gate 11: EMA step tests pass in make ci; live plotted point equals EMA of raw readouts within 0.019 over 77 frames; tmp/workflows/btop-web-implement05/checkpoints/verify-2.json
  - test: unit-test the smoothing on a step input (63 percent of the step after 1 s); compare a plotted path with the raw series on a noisy fixture
  - test-tags: UNIT, E2E
  - mechanism: 2026-10-26T14:40:35Z @kj one EMA per history buffer in chart.rs, alpha from the update interval and the 1 s constant
  - log: 2026-10-26T14:40:35Z @kj added
  - log: 2026-10-26T17:09:27Z @kj closed
- [x] `ACC-GRAPH-42-GAUGE-TICKS` **Gauge readout inside the arc, with ticks** - HIGH; Each radial gauge shows its value, unit and label inside the arc's empty centre, legible and touching neither the arc, its fill nor the ticks; tick marks every 10 percent, longer at 0, 50 and 100 with small labels outside the arc, replace the range text; owner asked 2026-10-26
  - evidence: wf_c29f79db-a67 verify-2 gate 2b: readouts inside inner radius, 5.4 px min clearance, 11 ticks with 3 major, 4.5:1; tmp/workflows/btop-web-implement05/checkpoints/verify-2.json
  - test: render each gauge; assert the readout box lies inside the arc's inner radius and intersects no arc, fill or tick path; count 11 ticks with 3 major; assert the readout text reaches 4.5:1
  - test-tags: UNIT, E2E
  - mechanism: 2026-10-26T15:01:47Z @kj server-built SVG arc with tick paths; the readout is HTML centred over the hollow, with the arc's inner radius as its bound
  - log: 2026-10-26T15:01:47Z @kj added
  - log: 2026-10-26T17:09:27Z @kj closed
- [ ] `ACC-GRAPH-43-GPU-LOAD` **GPU load graph in the lower band** - HIGH; the lower band's left slot holds a GPU load graph: each GPU in its own lane, the lanes on one shared time axis, smoothed and PCHIP like the other plots, no text over the plot; owner asked 2026-10-26, lanes 2026-10-28
  - test: render at 1440x900: band 3 left panel is the GPU load plot, one series per GPU, no text inside its data-chart box
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-26T15:51:13Z @kj added
  - log: 2026-10-26T16:56:38Z @kj owner chose: GPU load plot with a per-GPU legend (name, util, VRAM, temp, power) like the network legend; the GPU cards panel goes; cores move to band 3 middle c6-9, storage c10-12
  - log: 2026-10-26T18:59:43Z @kj closed
  - log: 2026-10-28T11:14:28Z @kj reopened: amended: one lane per GPU (owner 2026-10-28, DEF-GRAPH-46-GPU-LANES); evidence retired: band 3 left: one data-chart plot, one series per GPU, legend per GPU; plot and network gain 80 samples in the same 960-slot window over 80 ticks; no text over plots; 135 tests; wf_46141fe3-f77 tmp/workflows/btop-web-gpu-load/checkpoints/verify-3.json gates 1-8, tmp/workflows/btop-web-gpu-load/checkpoints/final-1.json check 2
  - log: 2026-10-28T11:14:34Z @kj amended text "the lower band's left slot, where 05 had the network graph, holds a GPU load graph: utilisation history of each GPU on one shared time axis, smoothed and PCHIP like the other plots, no text over the plot; owner asked 2026-10-26" -> "HIGH; the lower band's left slot holds a GPU load graph: each GPU in its own lane, the lanes on one shared time axis, smoothed and PCHIP like the other plots, no text over the plot; owner asked 2026-10-26, lanes 2026-10-28"
  - log: 2026-10-28T11:14:34Z @kj correction: the reopen line's defect id is DEF-GRAPH-51-GPU-LANES
  - log: 2026-10-28T11:44:20Z @kj owner 2026-10-28: one separate graph per GPU, not one shared graph; legend keeps all fields at 1024x768
- [-] `ACC-GRAPH-45-MOTION-PREDICT` **Plots and gauges move between samples with a fast predictor** - MEDIUM; between samples every plot slides and every gauge arc glides toward a one-interval-ahead estimate from a fast Rust predictor (Kalman or similar), corrected when the sample arrives; numbers stay raw; used only if it costs next to nothing; a CONFIG toggle turns it off; owner asked 2026-10-26
  - test: fps sweep: client CPU under 1 pp of one core at the chosen capped fps, fps above the 4 per s update rate; predictor beats hold-last on CPU; toggle off gives today's page
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-26T18:53:09Z @kj added
  - log: 2026-10-26T18:53:09Z @kj owner chose a predictor over the one-interval delay and slide, for plots and gauges; told the predicted value is not measured; estimate covers one interval only, none for unavailable series
  - log: 2026-10-26T18:54:32Z @kj edited test "recorded trace: predictor beats hold-last on mean correction; browser: paths and arcs move between frames, still when paused, STALE or reduced motion" -> "trace: beats hold-last; cost: predictor under 10 us per tick, SSE bytes +1 percent, client CPU +2 points of a core; toggle off gives today's page; still when paused, STALE, reduced motion"
  - log: 2026-10-26T18:54:35Z @kj amended text "between two samples every plot slides and every gauge arc glides toward a one-interval-ahead estimate from a fast Rust predictor (Kalman filter or similar), corrected when the measured sample arrives; numbers stay btop's raw values; owner asked 2026-10-26" -> "MEDIUM; between samples every plot slides and every gauge arc glides toward a one-interval-ahead estimate from a fast Rust predictor (Kalman or similar), corrected when the sample arrives; numbers stay raw; used only if it costs next to nothing; a CONFIG toggle turns it off; owner asked 2026-10-26"
  - log: 2026-10-26T18:54:36Z @kj owner added: predictor only if it costs next to nothing, and toggleable; bar: server under 10 us per tick, SSE bytes under +1 percent, client CPU under +2 points of one core; toggle in the CONFIG popover, persisted in the browser, on by default
  - log: 2026-10-26T20:18:33Z @kj measured: full design +51.8 pp client CPU, cheapest slide +5.7 pp, server +1 ns, bytes +0.1 percent; owner: reduce the animation fps so the client cost falls to a fraction of a percent; bar now under 1 pp of one core; gauge glide dropped, no target beat no glide
  - log: 2026-10-26T20:18:33Z @kj edited test "trace: beats hold-last; cost: predictor under 10 us per tick, SSE bytes +1 percent, client CPU +2 points of a core; toggle off gives today's page; still when paused, STALE, reduced motion" -> "fps sweep: client CPU under 1 pp of one core at the chosen capped fps, fps above the 4 per s update rate; predictor beats hold-last on CPU; toggle off gives today's page"
  - log: 2026-10-26T20:19:36Z @kj owner: keep the browser animation capped at the performance budget; the fps is the highest under 1 pp of one core, built when it exceeds the 4 per s update rate
  - log: 2026-10-26T21:41:42Z @kj owner accepted ema-step over the damped Kalman: on the live trace ema-step 0.203 px vs Kalman 0.233 px CPU correction, 0.7 ns vs 6.8-27.5 ns per tick; Kalman equals hold-last on network and GPU
  - log: 2026-10-26T21:47:32Z @kj owner questioned the Kalman result; finding: the tested Kalman took the 1 s EMA output as its measurement while ema-step used the raw reading, so the comparison was not like for like
  - log: 2026-10-26T21:58:25Z @kj fair rerun wf_d07b7d6d-180 with raw readings: ema-step wins, CPU 0.203 px vs one-state 0.237 and two-state Kalman 0.238; rates 3.57 vs 7.28 px; ema-step is a one-state Kalman with gain 1; best hindsight Kalman equals it; verified independently
  - log: 2026-10-26T22:05:48Z @kj rejected: owner's rule (slide only under a fraction of a percent, capped at performance) not met: cheapest slide +1.48 pp of one core at 5 fps over 9 paired runs, CPU and memory only +1.73 pp; bar 1 pp; headless Chrome, software compositing; wf_ec680a42-95f spec.json
- [x] `ACC-GRAPH-47-GAUGE-SMOOTH` **Gauge arcs follow the smoothed series** - MEDIUM; each gauge arc is drawn from the same 1 s EMA as the plots, so it steps about a third as far per update; the number inside the arc stays btop's raw value; no animation; owner chose 2026-10-26
  - evidence: CPU arc within 0.003 points of the 1 s EMA over 41 live frames, readout raw in every frame; arc step per frame 1.27 vs 3.76 percent raw; tmp/workflows/btop-web-motion2/checkpoints/verify-1.json gate 1, tmp/workflows/btop-web-motion2/checkpoints/final-2.json check 2
  - test: unit: arc fill from the EMA, readout raw; live: CPU arc step per frame drops from about 3.5 to about 1.2 percent
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-26T20:18:33Z @kj added
  - log: 2026-10-26T23:16:50Z @kj closed
- [x] `ACC-GRAPH-67-SMOOTH-SCALE` **Smooth scale changes** - HIGH; when an auto-scaled graph changes its scale, the lines and the scale label move to the new scale smoothly over several frames, never in one frame; applies to the network graph and every other auto-scaled graph
  - evidence: scale change animated over 1.0-1.2 s, 0 of 43 changes above 4 px per frame, label steps 4-5 values; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - related: DEF-GRAPH-39-NET-JUMP
  - test: force a rescale on the fixture or live traffic: no vertex moves more than a few px between consecutive frames during the change; label ends at the new value
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T18:20:46Z @kj added
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint
- [x] `ACC-GRAPH-68-PLOT-GRID` **Grid on the top graphs, horizontal grid on all** - MEDIUM; the top-band graphs (CPU, Memory, Processes) get the vertical time grid the Network and GPU graphs have, and every graph gets horizontal grid lines, except the small CPU graph in each process row (owner 2026-10-28: noisy there); grid lines very faint, under the lines, never over a value
  - evidence: process-row graphs: computed background-image none in 20 of 20 cases (3 viewports x top/all/tree x dark/light); cpu, mem, proc-count, net-down, net-up, gpu keep 6 grid gradients (1.31:1 dark, 1.26:1 light); make ci 242 green; round3-prep wf_ea69fac5-1f4 verify-1 and final checks 1-4, docs recheck; tmp/workflows/btop-web-round3-prep/checkpoints/
  - test: 1440x900: each [data-chart] box has vertical and horizontal grid lines under the plot, muted colour, no text over the plot
  - test-tags: FUNCTIONAL
  - log: 2026-10-27T18:29:16Z @kj added
  - log: 2026-10-27T18:29:18Z @kj amended text "the top-band graphs (CPU, Memory, Processes) get the vertical time grid the Network and GPU graphs have, and every graph also gets horizontal grid lines; grid lines are muted, drawn under the lines, never over a value" -> "the top-band graphs (CPU, Memory, Processes) get the vertical time grid the Network and GPU graphs have, and every graph also gets horizontal grid lines; grid lines are very faint, drawn under the lines, never over a value"
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint
  - log: 2026-10-28T09:41:47Z @kj reopened; evidence retired: quarter grid under every [data-chart] plot, 1.16-1.38:1 in all ten variants, still during rescales; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - log: 2026-10-28T09:41:51Z @kj amended text "the top-band graphs (CPU, Memory, Processes) get the vertical time grid the Network and GPU graphs have, and every graph also gets horizontal grid lines; grid lines are very faint, drawn under the lines, never over a value" -> "the top-band graphs (CPU, Memory, Processes) get the vertical time grid the Network and GPU graphs have, and every graph gets horizontal grid lines, except the small CPU graph in each process row (owner 2026-10-28: noisy there); grid lines very faint, under the lines, never over a value"
  - log: 2026-10-28T10:22:59Z @kj closed; reason: evidence names the measured result and the checkpoint
- [x] `ACC-GRAPH-71-SCALE-FLOOR` **Auto scale has a floor** - HIGH; the auto scale changes smoothly over time and never goes below a floor, 1 MiB/s by default, so idle traffic draws low and calm instead of filling the box; btop's own floor is 10 KiB
  - evidence: 60 s under 1 MiB/s: label never below 1.0M; bursts rescale smoothly and return to the floor; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - related: ACC-GRAPH-67-SMOOTH-SCALE
  - test: idle interface under 1 MiB/s: scale label stays at the floor, no rescale below it; traffic above it rescales smoothly
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T19:40:40Z @kj added
  - log: 2026-10-27T20:01:57Z @kj owner 2026-10-27 21:00: never scale network traffic down to a trickle; the user must see when something interesting happens; microtraffic is not interesting
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint
- [x] `ACC-GRAPH-72-NET-COLOURS` **Download and upload colours clearly differ** - MEDIUM; in the Default theme, dark and light, the download and upload lines, areas and legend markers use clearly different hues (CIEDE2000 at least 30 between the two line colours) instead of btop's two violets; muted, no neon; 3:1 chart floor kept
  - evidence: Default download #86acd6 and upload #d9a978: CIEDE2000 35.86 dark, 38.34 light (was 11.2); merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - related: ACC-THEME-65-STEEL-DEFAULT
  - test: Default dark and light: computed download and upload line colours differ by CIEDE2000 30 or more; each reaches 3:1 on the plot surface
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T19:50:38Z @kj added
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint
- [-] `ACC-GRAPH-73-STAT-OVERLAY` **Tall top-tile charts under the big number** - MEDIUM; in the CPU, Memory and Processes tiles the chart also takes the height of the big-number row; the big number sits over the chart's top-left on a semitransparent backing, text 4.5:1 composed over the plot; no other text over a chart; the right half stays clear
  - test: 1440x900, 1024x768, 390x844: chart height before and after, number box inside the chart's left half, contrast in all themes dark and light
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-28T10:07:54Z @kj added
  - log: 2026-10-28T10:10:23Z @kj overrides ACC-GRAPH-24-UNOBSCURED removed - override removed, ACC-GRAPH-73 rejected
  - log: 2026-10-28T10:10:23Z @kj rejected: rejected by the owner on 2026-10-28 before any change was made; ACC-GRAPH-24-UNOBSCURED applies unchanged

## Dashboard display `VIEW`

What the panels render and how unavailable values are drawn

- log: 2026-10-25T03:54:53Z @kj category created

- [x] `ACC-VIEW-18-HOUSE-CHIP` **Indicators use house chips** - HIGH; every status indicator, badge and mode control uses the house chip of the JupyterLab extensions (device-bridge reference): compact, bordered, square or hairline radius, uppercase label, text beside colour. No rounded pills anywhere, badges and panels included
  - evidence: chips use a 1 px border, 2 px radius, uppercase and a 12% tint; max computed radius across the live page is 2 px; LIVE, READ-ONLY, CONFIG and the config options are chips
  - test: open the dashboard and the design system; assert every chip matches the house anatomy and that no element anywhere uses a pill radius
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T03:54:53Z @kj one chip component (Askama partial plus one CSS class) shared by READ-ONLY, connection, pause and the mode control; radius tokens allow 0 to 2 px only
  - log: 2026-10-25T03:54:53Z @kj added
  - log: 2026-10-25T07:54:57Z @kj closed
- [x] `ACC-VIEW-19-CONFIG-MENU` **Configuration lives in a config menu** - HIGH; theme pair, light and dark mode and every other setting sit in a config menu opened from the page chrome. The dashboard body carries telemetry only; the menu is keyboard reachable, closes on Escape and writes its choices through theme.js so they survive a reload
  - evidence: theme select, mode buttons, GPU selection and pause live only in #config; the body has no theme control; it opens with focus on the select, Escape closes it and returns focus to the trigger
  - test: open the dashboard and assert no theme or mode control in the content; open the config menu, change the theme pair and the mode, reload, and assert both persisted
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T03:57:42Z @kj one config menu component (Askama partial plus CSS) anchored in the header; the inline head script reads storage before first paint
  - log: 2026-10-25T03:57:42Z @kj added
  - log: 2026-10-25T07:54:57Z @kj closed
- [x] `ACC-VIEW-23-RELEVANT` **Only present, relevant telemetry is shown** - HIGH; the page shows only telemetry this host has and the operator needs. No battery, no absent sensor, no unavailable row, no uptime or address that adds nothing; a field the host cannot provide is not drawn at all, not drawn as unavailable
  - evidence: the live page contains no battery, uptime, unavailable or n/a element; a host without a GPU renders the one-line unavailable panel; no placeholder field is drawn
  - test: open the page on this host; assert no battery, sensor or unavailable row appears, and that every rendered field has a real value from the collector
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T05:19:38Z @kj the template omits a field when its value is None and the host reports no device; unavailable is reserved for a device that exists but failed to read
  - log: 2026-10-25T05:19:38Z @kj added
  - log: 2026-10-25T07:54:57Z @kj closed
- [x] `ACC-VIEW-40-GADGETS` **Panels resizable, removable and addable, good default** - HIGH; In the config menu every panel can be removed, added back and resized; choices persist in the browser and a reset restores the default; every layout keeps one viewport with no page scroll; the default layout passes the fit, space and overlap gates; owner asked 2026-10-26
  - evidence: wf_c29f79db-a67 verify-2 gate 1b: remove, add back, S/M/L, reload persists, reset restores; fits 1440x900 and 390x844 at each state; tmp/workflows/btop-web-implement05/checkpoints/verify-2.json
  - test: in the config menu remove a panel, add it back, change its size; reload and assert the layout persists; reset; assert each state fits 1440x900 and 390x844 with no scroll
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-26T14:46:42Z @kj layout state in localStorage next to the theme choice; panels hidden or spanned by CSS grid; server rendering unchanged
  - log: 2026-10-26T14:46:42Z @kj added
  - log: 2026-10-26T17:09:27Z @kj closed
- [x] `ACC-VIEW-41-TOP-BAR` **Top bar carries the app status** - HIGH; The top bar shows btop-web and its version, the host, the sampling interval, the live state with the last sample's age (LIVE, PAUSED, or STALE when the stream stops), READ-ONLY and the clock, CONFIG at the right end; none of these repeat elsewhere; owner asked 2026-10-26
  - evidence: wf_c29f79db-a67 verify-2 gate 1c: bar reads btop-web 0.1.0, host, 250 ms, LIVE with age, READ-ONLY, clock, CONFIG last; STALE within two intervals; tmp/workflows/btop-web-implement05/checkpoints/verify-2.json
  - test: render the page; assert the bar holds btop-web, the Cargo version, the host, the configured interval and LIVE; stop the server and assert STALE within two intervals; pause in the config menu and assert PAUSED
  - test-tags: E2E, UNIT
  - mechanism: 2026-10-26T14:50:26Z @kj server renders name, version, host and interval in the bar; app.js sets the live state from SSE frame arrival
  - log: 2026-10-26T14:50:26Z @kj added
  - log: 2026-10-26T17:09:27Z @kj closed
- [x] `ACC-VIEW-48-CORE-BARS` **Cores as btop rows with left-to-right bars** - HIGH; the cores panel shows one row per core like btop: label, a meter filling left to right, the percent on its right, smaller type; cells no longer light up or change colour; per-core motion is reduced so the panel stops jumping; owner asked 2026-10-26
  - evidence: 64 rows in 6 columns: label, left-to-right meter, smoothed number; no tint; number change 2.07 vs 7.63 per frame raw; bars within 0.07 points of the per-core EMA; tmp/workflows/btop-web-motion2/checkpoints/verify-1.json gate 2c
  - test: render 64 cores: bar width proportional to the value, number right of the bar, no tinted cell background; live: mean bar change per frame lower than raw
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-26T21:18:37Z @kj added
  - log: 2026-10-26T21:19:07Z @kj owner chose: bar and number both follow the 1 s EMA of each core, so the number is no longer btop's instant value
  - log: 2026-10-26T23:16:50Z @kj closed
- [x] `ACC-VIEW-49-TOTAL-LABEL` **CPU gauge says TOTAL, not BUSY** - LOW; the CPU gauge readout label reads TOTAL, btop's term for overall CPU usage, instead of BUSY, which came from the 05 mock; owner chose 2026-10-26
  - evidence: CPU gauge readout label TOTAL, no BUSY text; tmp/workflows/btop-web-motion2/checkpoints/verify-1.json gate 2d
  - test: render: the CPU gauge readout label is TOTAL; no BUSY text on the page
  - test-tags: UNIT
  - log: 2026-10-26T21:19:11Z @kj added
  - log: 2026-10-26T23:16:50Z @kj closed
- [x] `ACC-VIEW-50-CPU-LOAD` **CPU shows normalised load and aggregate core load** - MEDIUM; the CPU panel shows the normalised load (load average divided by the core count) and below it the aggregate load of the cores; owner asked 2026-10-26
  - evidence: load per core equals /proc/loadavg / cores, cores busy equals total x cores / 100, labels and tooltip present, fits at four viewports; tmp/workflows/btop-web-motion2/checkpoints/verify-1.json gate 2e; live 2026-10-27 00:16
  - test: render on the fixture: normalised load equals loadavg / cores; the aggregate core load equals the per-core values combined
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-26T21:19:48Z @kj added
  - log: 2026-10-26T21:25:46Z @kj owner chose facts beside the arc: arc stays TOTAL; load per core for 1, 5 and 15 min (loadavg / cores; 1.00 = one task per core); cores busy N.N of 64 (CPU total x cores / 100); label spelled out with a tooltip, load/core was unclear
  - log: 2026-10-26T23:16:50Z @kj closed
- [x] `ACC-VIEW-52-PROC-GRAPH` **Small CPU graph per process** - MEDIUM; the process table shows a small CPU load graph per process beside Cpu%, as btop's proc_cpu_graphs does, only if it costs btop-web next to nothing: server under 1 percent more CPU per tick, SSE bytes under +3 percent, client CPU under +0.5 points of one core; owner asked 2026-10-26
  - evidence: wf_8a79cbc6-eaa verify-3 4 of 4: graph from a row's first update above 0, btop's 10-update drop, 0 of 2,275 knots moved; server +0.13 percent, SSE bytes +0.37-0.71 percent, Chrome +0.25 pp; final checks 1-4 pass; tmp/workflows/btop-web-proc-graphs/checkpoints/verify-3.json, tmp/workflows/btop-web-proc-graphs/checkpoints/final.json
  - test: render: rows with CPU above 0 carry a graph left of Cpu%; cost measured against the three limits; over the limit it is not shipped
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-26T21:31:47Z @kj added
  - log: 2026-10-26T21:31:47Z @kj btop 1.3.0 reference: proc_cpu_graphs default true; btop_draw.cpp:1793-1806 keeps a 5-column graph per pid with CPU above 0 and drops it after 10 updates below 0.1 percent; btop's 10 samples span 20 s at its 2000 ms default
  - log: 2026-10-27T05:55:59Z @kj closed
- [x] `ACC-VIEW-53-PROC-TILE` **Process count graph and process facts** - MEDIUM; the Processes tile shows a graph of the process count over the plot window beside the current count, and these facts: running, sleeping, waiting and zombie counts; threads; the top process by CPU with its percent; how many processes use 80 percent of total process CPU and of memory; how many processes use a GPU, unavailable when NVML cannot list them; owner chose 2026-10-27
  - evidence: tile: count graph, running/sleeping/waiting/zombie, threads, top process, 80% cpu and mem counts; gpu count unavailable in a container (NVML pids are host pids); merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - test: render: the tile carries a [data-chart] count graph and the five facts; live: the graph rises when processes start; GPU count matches nvidia-smi or shows unavailable
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T11:55:18Z @kj added
  - log: 2026-10-27T12:06:02Z @kj edited text "the Processes tile shows a graph of the process count over the plot window beside the current count, plus more process facts; owner asked 2026-10-27; facts chosen by the owner from: per-state counts, forks per second, threads, top process" -> "the Processes tile shows a graph of the process count over the plot window beside the current count, and these facts: running, sleeping, waiting and zombie counts; threads; the top process by CPU with its percent; how many processes use 80 percent of total process CPU and of memory; how many processes use a GPU, unavailable when NVML cannot list them; owner chose 2026-10-27"; test "render: the tile carries a [data-chart] count graph and the chosen facts; live: the graph rises when processes start and falls when they end" -> "render: the tile carries a [data-chart] count graph and the five facts; live: the graph rises when processes start; GPU count matches nvidia-smi or shows unavailable"; reason: lists the five facts the owner chose
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint
- [x] `ACC-VIEW-54-CORE-LABEL` **Core label and percent on every row** - HIGH; every cores row shows its core name with two digits, C00 to C63, and its value with the percent sign, as 50%, at every tier and window size; core names leave the drop table; owner asked 2026-10-27 and clarified: core name as C01, C08, value with the % sign
  - evidence: C00-C63 and value with % at every tier, 88 of 88 sweep sizes; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - related: DEF-VIEW-37-CORE-LABEL,ACC-VIEW-48-CORE-BARS,ACC-LAYOUT-46-ADAPTIVE-PRIORITY
  - test: sweep all window sizes: 64 rows each show C00-C63 and a value ending in %, nothing clipped, no page scroll
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T12:07:45Z @kj added
  - log: 2026-10-27T12:19:32Z @kj edited text "every cores row shows its label C0-C63 and its value with the percent sign, as 50%, at every tier and window size; core labels leave the drop table; owner asked 2026-10-27" -> "every cores row shows its core name with two digits, C00 to C63, and its value with the percent sign, as 50%, at every tier and window size; core names leave the drop table; owner asked 2026-10-27 and clarified: core name as C01, C08, value with the % sign"; test "sweep all window sizes: 64 rows each show C<n> and a value ending in %, nothing clipped, no page scroll" -> "sweep all window sizes: 64 rows each show C00-C63 and a value ending in %, nothing clipped, no page scroll"
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint
- [ ] `ACC-VIEW-55-TAB-TITLE` **Page tab shows a title** - MEDIUM; the browser tab and the JupyterLab tab that show the page read btop-web; owner asked 2026-10-27 after the JupyterLab tab opened with an empty label
  - test: live: document.title is btop-web; the JupyterLab open link passes title btop-web and the tab label reads it
  - test-tags: FUNCTIONAL, MANUAL
  - log: 2026-10-27T12:09:02Z @kj added
  - log: 2026-10-27T12:09:06Z @kj cause: page has <title>btop-web</title> (templates/index.html:7, read live in Chrome); the JupyterLab tab label is args.title of server-proxy:open (jupyter-server-proxy static 724.js), and the open link sent 2026-10-27 omitted title
- [x] `ACC-VIEW-56-CORE-COLOUR` **Core bars and values take btop's CPU gradient colour** - HIGH; as btop's Cpu::draw: each core meter fills with the theme's cpu gradient (cpu_start, cpu_mid, cpu_end) along its length, and the value is drawn in the gradient colour at its value, then % in the main text colour; colour follows the smoothed value; text keeps 4.5:1 composed; replaces the no-colour-change clause of ACC-VIEW-48-CORE-BARS; owner asked 2026-10-27
  - evidence: meter fill linear-gradient(--ok, --warn, --crit) in 640 of 640 rows, value colour at 4.5:1; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - related: ACC-VIEW-48-CORE-BARS,ACC-VIEW-54-CORE-LABEL
  - test: render: meter gradient stops come from the theme's cpu gradient; value colour equals the gradient at its value; contrast 4.5:1 in all ten theme variants
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T12:10:26Z @kj added; reason: names btop's draw rule, the smoothing and the contrast floor
  - log: 2026-10-27T12:10:36Z @kj edited text "as btop's Cpu::draw: each core meter fills with the theme's cpu gradient (cpu_start, cpu_mid, cpu_end) along its length, and the value is drawn in the gradient colour at its value, then % in the main text colour; colour follows the smoothed value; text keeps 4.5:1 composed; replaces the no-colour-change clause of ACC-VIEW-48; owner asked 2026-10-27" -> "as btop's Cpu::draw: each core meter fills with the theme's cpu gradient (cpu_start, cpu_mid, cpu_end) along its length, and the value is drawn in the gradient colour at its value, then % in the main text colour; colour follows the smoothed value; text keeps 4.5:1 composed; replaces the no-colour-change clause of ACC-VIEW-48-CORE-BARS; owner asked 2026-10-27"; reason: names btop's draw rule, the smoothing and the contrast floor
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint
- [x] `ACC-VIEW-57-FAVICON` **Page tab shows the btop-web icon** - MEDIUM; the tab showing the page carries a btop-web icon that reads as a chart; the owner picks it from 10 proposals (2026-10-27); served as assets/icon.svg; a JupyterLab proxy tab shows no icon (jupyter-server-proxy sets none), so the owner opens a browser tab or picks a JupyterLab-side change
  - evidence: owner 2026-10-28: the browser tab is enough; assets/icon.svg serves 06-stacked-bars byte-equal (200 image/svg+xml), linked from the page; the JupyterLab proxy tab sets no icon; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - test: browser tab: icon shown at 16 px; GET assets/icon.svg 200 image/svg+xml through the proxy
  - test-tags: FUNCTIONAL, MANUAL
  - log: 2026-10-27T12:10:30Z @kj added; reason: states where the icon shows and why the JupyterLab tab cannot
  - log: 2026-10-27T19:41:23Z @kj owner 2026-10-27: change icon to be more like a chart; propose 10 icons
  - log: 2026-10-27T19:41:25Z @kj amended text "the tab showing the page carries the btop-web icon; the page links assets/icon.svg, adapted from btop's Img/icon.svg; a JupyterLab proxy tab shows no icon because jupyter-server-proxy sets none, so the owner opens the page in a browser tab or picks a JupyterLab-side change; owner asked 2026-10-27" -> "the tab showing the page carries a btop-web icon that reads as a chart; the owner picks it from 10 proposals (2026-10-27); served as assets/icon.svg; a JupyterLab proxy tab shows no icon (jupyter-server-proxy sets none), so the owner opens a browser tab or picks a JupyterLab-side change"
  - log: 2026-10-27T19:44:49Z @kj 10 proposals ready for the owner's pick: tmp/workflows/btop-web-favicon/sheet.png, icons/01-line-grid to 10-chart-b, each at 16, 32, 64 px on dark and light tab bars
  - log: 2026-10-27T19:48:45Z @kj owner picked 06 stacked-bars (tmp/workflows/btop-web-favicon/icons/06-stacked-bars.svg); applied as change acc-57 in btop-web-merge-1
  - log: 2026-10-28T09:41:47Z @kj closed
- [x] `ACC-VIEW-58-PROC-VIEW` **Process list modes: top-level, all, tree** - HIGH; the config menu sets the process list to top-level only, all (flat, today's list) or tree; a viewer with no stored choice starts in top-level only, or in tree when btop's proc_tree is True; tree indents children under parents as btop's proc_tree; an expanded row shows its own CPU and memory; a collapsed row shows its own plus its hidden descendants', as btop's proc_aggregate; top-level only lists the roots (parent init or none), each collapsed; a config option sets tree nodes collapsed or expanded; a triangle marker on each parent row, pointing right when collapsed and down when expanded, toggles that node on click; owner chose 2026-10-27
  - evidence: top, all and tree views equal two independent fixture models; collapsed rows show own plus descendants; triangles U+25B8/U+25BE with aria-expanded toggle one node by click and Enter; tree nodes option collapsed or expanded; proc-view wf_d3dbd6ab-57d verify-3 7 of 7 and final 5 of 5, make ci 228 green, live 40290 2eac7608; tmp/workflows/btop-web-proc-view/checkpoints/verify-3.json
  - test: render on the fixture tree: each mode shows the expected pids in order; tree rows carry depth and collapse state; switching mode needs no reload
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T12:12:04Z @kj added; reason: lists the three modes, the tree option and btop's key
  - log: 2026-10-27T12:17:37Z @kj edited text "the config menu sets the process list to top-level only, all (flat, today's list) or tree; tree view indents children under parents as btop's proc_tree does, with a config option for nodes collapsed or expanded; btop's proc_tree config key sets the starting mode; owner asked 2026-10-27; top-level definition and per-row toggling chosen by the owner" -> "the config menu sets the process list to top-level only, all (flat, today's list) or tree; tree indents children under parents as btop's proc_tree; an expanded row shows its own CPU and memory; a collapsed row shows its own plus its hidden descendants', as btop's proc_aggregate; top-level only lists the roots (parent init or none), each collapsed; a config option sets tree nodes collapsed or expanded, and clicking a row's [+]/[-] toggles that node; btop's proc_tree key sets the starting mode; owner chose 2026-10-27"; reason: states the three modes, the aggregation rule the owner chose and the toggle
  - log: 2026-10-27T12:17:42Z @kj edited text "the config menu sets the process list to top-level only, all (flat, today's list) or tree; tree indents children under parents as btop's proc_tree; an expanded row shows its own CPU and memory; a collapsed row shows its own plus its hidden descendants', as btop's proc_aggregate; top-level only lists the roots (parent init or none), each collapsed; a config option sets tree nodes collapsed or expanded, and clicking a row's [+]/[-] toggles that node; btop's proc_tree key sets the starting mode; owner chose 2026-10-27" -> "the config menu sets the process list to top-level only, all (flat, today's list) or tree; tree indents children under parents as btop's proc_tree; an expanded row shows its own CPU and memory; a collapsed row shows its own plus its hidden descendants', as btop's proc_aggregate; top-level only lists the roots (parent init or none), each collapsed; a config option sets tree nodes collapsed or expanded; a triangle marker on each parent row, pointing right when collapsed and down when expanded, toggles that node on click; btop's proc_tree key sets the starting mode; owner chose 2026-10-27"; reason: states the three modes, the aggregation rule, the triangle toggle the owner chose
  - log: 2026-10-27T12:19:39Z @kj edited text "the config menu sets the process list to top-level only, all (flat, today's list) or tree; tree indents children under parents as btop's proc_tree; an expanded row shows its own CPU and memory; a collapsed row shows its own plus its hidden descendants', as btop's proc_aggregate; top-level only lists the roots (parent init or none), each collapsed; a config option sets tree nodes collapsed or expanded; a triangle marker on each parent row, pointing right when collapsed and down when expanded, toggles that node on click; btop's proc_tree key sets the starting mode; owner chose 2026-10-27" -> "the config menu sets the process list to top-level only, all (flat, today's list) or tree; a viewer with no stored choice starts in top-level only, or in tree when btop's proc_tree is True; tree indents children under parents as btop's proc_tree; an expanded row shows its own CPU and memory; a collapsed row shows its own plus its hidden descendants', as btop's proc_aggregate; top-level only lists the roots (parent init or none), each collapsed; a config option sets tree nodes collapsed or expanded; a triangle marker on each parent row, pointing right when collapsed and down when expanded, toggles that node on click; owner chose 2026-10-27"; reason: states the modes, the top-level default, the aggregation rule and the triangle toggle the owner chose
  - log: 2026-10-28T07:31:15Z @kj closed; reason: evidence names the measured result and the checkpoint
- [x] `ACC-VIEW-59-PROC-SORT` **Sort the process list by clicking a column** - HIGH; clicking a process column header sorts the list by that column, clicking it again reverses the order; the starting sort is memory, largest first, for a viewer with no stored choice, even though this host's btop.conf says proc_sorting cpu direct; the active column shows a marker with the direction; per viewer; tree mode sorts siblings by it; owner asked 2026-10-27
  - evidence: fresh viewer sorts MemB descending with the marker on MemB only; 7 headers x 2 clicks per view match raw values; proc-view wf_d3dbd6ab-57d verify-3 7 of 7 and final 5 of 5, make ci 228 green, live 40290 2eac7608; tmp/workflows/btop-web-proc-view/checkpoints/verify-3.json
  - related: ACC-VIEW-58-PROC-VIEW
  - test: click each header: rows ordered by it, second click reverses; fresh viewer sorted by memory; active header carries the marker; reload keeps the choice
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T12:19:32Z @kj added; reason: states the click rule, the memory default over btop.conf and the marker
  - log: 2026-10-28T07:31:15Z @kj closed; reason: evidence names the measured result and the checkpoint
- [ ] `ACC-VIEW-75-PANEL-MENU` **Right-click a panel to set its size** - HIGH; right-click on any panel opens a menu: Off, S, M, L; the choice applies at once, shares the Layout page state and persists in the browser; Off removes the panel, which the Layout page adds back; the keyboard context-menu key opens it on a focused panel
  - related: ACC-VIEW-40-GADGETS
  - test: right-click each panel at 1440x900: menu shows Off/S/M/L with the current size marked; each choice changes the panel and the Layout page and survives a reload
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T11:06:35Z @kj added

## Layout and density `LAYOUT`

Page fit, density and responsive behaviour across viewport sizes

- log: 2026-10-25T03:59:19Z @kj category created

- [x] `ACC-LAYOUT-20-ADAPTIVE` **Smaller screens shrink the graphs, not the page** - HIGH; as the viewport shrinks the layout adapts: graph heights and per-core cells shrink in steps, type and padding step down, panels reflow, and the page still fits one screen at 1440x900, 1280x800, 1024x768 and 390x844 with no page scroll. The process table scrolls inside its panel
  - evidence: scrollHeight equals innerHeight at 1440x900, 1280x800, 1024x768 and 390x844; CPU history 168/128/112 px and network 95.4/85.4/64.2 px (verify.json)
  - test: render the page in headless Chrome at 1440x900, 1280x800, 1024x768 and 390x844; assert document.scrollHeight does not exceed innerHeight at each size and that graph heights decrease step by step
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T03:59:19Z @kj media queries in three steps driving height and padding tokens; SVG sparklines sized by viewBox and a height token; the process table keeps its own scroll area
  - log: 2026-10-25T03:59:19Z @kj added
  - log: 2026-10-25T08:41:41Z @kj closed
- [x] `ACC-LAYOUT-21-PRIORITY` **Small screens keep the highest-priority data** - HIGH; each field has a stated priority; as space shrinks the layout keeps the higher-priority fields and drops or summarises the rest. CPU total, memory used, network rate, the three GPUs and the process table outrank per-core cells, load, disks, IP and detail columns; what is dropped is named, never clipped
  - evidence: notes.md 9.4 drop-order table matches measured visibility at 1440x900, 1280x800, 1024x768 and 390x844 in all 15 rows (verify.json)
  - test: resize through 1440x900, 1280x800, 1024x768 and 390x844; at each size assert every visible field is on the priority list for that size and every hidden field is named in the design's drop order
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T04:00:44Z @kj one priority table in the design system drives the breakpoints; lower-priority panels collapse to one summary line; notes.md lists the order
  - log: 2026-10-25T04:00:44Z @kj added
  - log: 2026-10-25T13:49:42Z @kj closed
- [x] `ACC-LAYOUT-37-SPACE` **Space rearranged, no empty bands** - HIGH; Panel space follows content: at 1440x900 and 1024x768 no panel body holds an empty band of 24 px or more with no text, control or plot; freed height goes to the plots; owner asked 2026-10-26 to rearrange the space
  - evidence: wf_c29f79db-a67 verify-2 gate 12: live host largest empty band 19.5 px at 1440x900 and 17.5 px at 1024x768, none reaches 24 px; tmp/workflows/btop-web-implement05/checkpoints/verify-2.json
  - test: render at 1440x900 and 1024x768; scan each panel body for horizontal bands of 24 px or more holding no element; expect none
  - test-tags: E2E
  - mechanism: 2026-10-26T14:40:35Z @kj panels size to content in the 05 grid; plots take the remaining height
  - log: 2026-10-26T14:40:35Z @kj added
  - log: 2026-10-26T17:09:27Z @kj closed
- [x] `ACC-LAYOUT-46-ADAPTIVE-PRIORITY` **Shipped page adapts by priority drops** - HIGH; below desktop size the shipped page follows docs/design/concepts/05.06-adaptive-1-priority.html: one grid, plots and bands shrink, then type and padding step down, then fields drop P4, P3, P2 at named breakpoints with the tier chip; no page scroll at any size; owner chose 2026-10-26
  - evidence: wf_7eb06481-928 verify-1 5 of 5: 88 of 88 sweep sizes pass fresh and on live resize, live and fixture; drop table matches at the 8 named sizes incl. 844x390, 667x375, 915x412; 1440x900 unchanged; contrast 4.51 or more; final-1 5 of 5; tmp/workflows/btop-web-adaptive-ship/checkpoints/verify-1.json, tmp/workflows/btop-web-adaptive-ship/checkpoints/final-1.json
  - test: sweep widths 360-1920 and heights 600-1200 plus 844x390 and 667x375: no scroll, no text over a plot, nothing clipped, drops match the table
  - test-tags: FUNCTIONAL
  - log: 2026-10-26T19:10:55Z @kj added
  - log: 2026-10-27T07:29:40Z @kj closed
- [x] `ACC-LAYOUT-62-VERSION` **Version stays in the top bar** - MEDIUM; the top bar shows the btop-web version next to the name at every tier and window size; today the drop-p4 tier (width up to 1100 px or height up to 640 px) hides it with the host and clock; owner asked 2026-10-27 where the version was
  - evidence: version shown next to btop-web at every tier of the sweep; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - related: ACC-LAYOUT-46-ADAPTIVE-PRIORITY
  - test: sweep all window sizes: the version text is visible next to btop-web, nothing clipped, no page scroll
  - test-tags: FUNCTIONAL
  - log: 2026-10-27T12:21:51Z @kj added
  - log: 2026-10-28T02:00:21Z @kj closed; reason: evidence names the measured result and the checkpoint
- [ ] `ACC-LAYOUT-74-RESERVED-WIDTH` **Reserved space, nothing moves** - HIGH; general rule: every live value reserves the width of its widest form (100%, three-digit counts, longest unit), tabular figures, so no label, value or control beside it moves when the value changes; text longer than its slot (process and model names) is cut by an ellipsis inside the slot
  - test: each viewport and theme: set every live value to its narrowest and widest form and watch 20 s of updates; no other text box moves more than 0.5 px
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T10:11:50Z @kj added
  - log: 2026-10-28T10:12:14Z @kj owner: applies everywhere it makes sense - every panel, top bar, legends, process table - wherever a width change moves other text; a value with nothing after it on its line needs no reservation
  - log: 2026-10-28T11:06:10Z @kj scope note: the units after the big numbers (CPU %, Memory GiB) are in scope; the ACC-GRAPH-73 rejection excludes only the overlay and taller charts

## Design and presentation `DESIGN`

Where the design derives from and how its sources are recorded

- log: 2026-10-25T04:01:57Z @kj category created

- [x] `ACC-DESIGN-22-BORROW-REFS` **Design borrows from btop and the best monitors** - HIGH; the design and presentation derive from btop's frame and the best current monitors: btop supplies the information architecture, panel anatomy and density; the researched references supply typography, colour and chart presentation. Each panel and component names its source in the design system; borrowing without a named source is not accepted
  - evidence: docs/design/notes.md section 15 names the btop 1.3.0 source and the researched reference for all 10 panels, GPU load plot included; tmp/workflows/btop-web-implement05/checkpoints/final.json check 3, tmp/workflows/btop-web-gpu-load/checkpoints/docs.json
  - test: open the design system's borrowed-from section; every panel and component names btop or a researched reference; compare index.html's layout with btop's frame and the named references
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T04:01:57Z @kj the revision records one line per element in notes.md and system.html; the verifier rejects a claim without a source
  - log: 2026-10-25T04:01:57Z @kj added
  - log: 2026-10-26T18:59:43Z @kj closed
- [x] `ACC-DESIGN-29-CONCEPTS` **Ten design options are produced and the owner picks one** - HIGH; after the owner approved the 1:1 render, ten modernised one-page concepts were to be produced for comparison, each comparable at 1440x900, and the owner picks one; the pick and its reason are recorded and the shipped page follows it until the owner says otherwise
  - evidence: ten concepts in docs/design/concepts/ 01-10; owner picked 05-grafana-panels, then 05.06-network-paired ('this is probably the best'), recorded on ACC-DESIGN-33-RENDER; shipped page follows it
  - test: open docs/design/concepts/index.html and its ten pages; confirm each renders at 1440x900; confirm the recorded pick matches the shipped page
  - test-tags: MANUAL
  - mechanism: 2026-10-25T15:03:06Z @kj one concept page and one screenshot per option under docs/design/concepts/, indexed by index.html; the approved 1:1 render stays shipped until a pick is recorded
  - log: 2026-10-25T15:03:06Z @kj added
  - log: 2026-10-25T15:18:05Z @kj owner picked 05-grafana-panels on 2026-10-25, with the constraint that a displayed number must not sit on top of a graph; ten variations 05.01-05.10 are wanted before the shipped page follows it, tracked as ACC-DESIGN-35-VARIANTS
  - log: 2026-10-25T15:18:11Z @kj correction: the variations are tracked as ACC-DESIGN-36-VARIANTS; the earlier log named 35 in error
  - log: 2026-10-26T14:38:43Z @kj 2026-10-26 owner: continue with the implementation of the page I selected; implementing 05-grafana-panels with no number over a plot and CONFIG in fixed page chrome
  - log: 2026-10-26T14:46:42Z @kj 2026-10-26 owner chose docs/design/concepts/variants-r2-legibility/05.06-charts.html as the base: network moved from the lower row to the upper row, arrangement refined toward the original btop layout, gauge meters allowed
  - log: 2026-10-26T14:59:56Z @kj 2026-10-26 owner: docs/design/concepts/05.06-network-paired.html is probably the best, so something around this; it replaces 05.06-charts as the base
  - log: 2026-10-26T18:59:43Z @kj closed
- [x] `ACC-DESIGN-33-RENDER` **The shipped page renders the chosen 05.06-network-paired design** - HIGH; the shipped page stays close to docs/design/concepts/05.06-network-paired.html - network once, as paired download and upload plots on one time axis in the upper area, gauge meters allowed; live telemetry, no number over a plot, the top bar per ACC-VIEW-41-TOP-BAR, three NVIDIA GPUs
  - evidence: shipped page follows 05.06-network-paired with the owner's GPU load change; tmp/workflows/btop-web-implement05/checkpoints/verify-2.json gates 1 and 9, tmp/workflows/btop-web-gpu-load/checkpoints/verify-3.json gate 1; live on port 40290 checked through the lab proxy 2026-10-26
  - test: render the page at 1440x900 beside 05.06-network-paired.html; assert one network panel with paired plots in the upper area, three GPU cards, no text over a plot, the top bar contents
  - test-tags: E2E, MANUAL
  - mechanism: 2026-10-25T15:15:34Z @kj the approved mock is the reference; the Askama templates reproduce its geometry
  - log: 2026-10-25T15:15:34Z @kj added
  - log: 2026-10-26T14:38:43Z @kj amended title "The page is a 1:1 modernised btop render" -> "The shipped page renders the picked 05 Grafana-panels design"; text "the shipped page keeps btop's panel anatomy, border labels and density, shows this host's three NVIDIA GPUs by default, and drops terminal artefacts: no braille, no box drawing, no hotkey letters" -> "HIGH; the shipped page follows docs/design/concepts/05-grafana-panels.html - its panel set, grid and chart types - with live telemetry, no number or label over a plot, and one CONFIG button in fixed page chrome holding theme and mode; three NVIDIA GPUs by default; no terminal artefacts"
  - log: 2026-10-26T14:38:53Z @kj edited test "compare the page at 1440x900 with docs/design/onepage.html and the btop screenshots; assert three GPU boxes and no terminal artefact" -> "render the page at 1440x900 beside docs/design/concepts/05-grafana-panels.html; assert its panel set and chart types, three GPU cards, no text over a plot, CONFIG in the top bar"; test-tags "MANUAL" -> "E2E, MANUAL"
  - log: 2026-10-26T14:46:42Z @kj amended title "The shipped page renders the picked 05 Grafana-panels design" -> "The shipped page renders the chosen 05.06-charts design"; text "the shipped page follows docs/design/concepts/05-grafana-panels.html - its panel set, grid and chart types - with live telemetry, no number or label over a plot, and one CONFIG button in fixed page chrome holding theme and mode; three NVIDIA GPUs by default; no terminal artefacts" -> "HIGH; the shipped page follows docs/design/concepts/variants-r2-legibility/05.06-charts.html with network shown once, in the upper area instead of the Network in and Network out tiles; the arrangement is refined toward btop 1.3.0's original layout; gauge meters allowed; live telemetry, no number over a plot, CONFIG in the top bar, three NVIDIA GPUs"
  - log: 2026-10-26T14:46:42Z @kj edited test "render the page at 1440x900 beside docs/design/concepts/05-grafana-panels.html; assert its panel set and chart types, three GPU cards, no text over a plot, CONFIG in the top bar" -> "render the page at 1440x900 beside 05.06-charts.html; assert one network panel in the upper area, three GPU cards, no text over a plot, CONFIG in the top bar"
  - log: 2026-10-26T14:59:56Z @kj amended title "The shipped page renders the chosen 05.06-charts design" -> "The shipped page renders the chosen 05.06-network-paired design"; text "the shipped page follows docs/design/concepts/variants-r2-legibility/05.06-charts.html with network shown once, in the upper area instead of the Network in and Network out tiles; the arrangement is refined toward btop 1.3.0's original layout; gauge meters allowed; live telemetry, no number over a plot, CONFIG in the top bar, three NVIDIA GPUs" -> "HIGH; the shipped page stays close to docs/design/concepts/05.06-network-paired.html - network once, as paired download and upload plots on one time axis in the upper area, gauge meters allowed; live telemetry, no number over a plot, the top bar per ACC-VIEW-41-TOP-BAR, three NVIDIA GPUs"
  - log: 2026-10-26T14:59:56Z @kj edited test "render the page at 1440x900 beside 05.06-charts.html; assert one network panel in the upper area, three GPU cards, no text over a plot, CONFIG in the top bar" -> "render the page at 1440x900 beside 05.06-network-paired.html; assert one network panel with paired plots in the upper area, three GPU cards, no text over a plot, the top bar contents"
  - log: 2026-10-26T18:59:43Z @kj closed
- [x] `ACC-DESIGN-36-VARIANTS` **Ten small imaginative variations of the accepted 05 concept** - HIGH; Ten small variations 05.01-05.10 of the accepted 05-grafana-panels page; each is one visible imaginative idea over 05 - a design move a reader sees at a glance - while keeping 05's identity, data and three GPUs; no page is a near-clone of 05 or of another, and no page is a new concept; each fits 1440x900, draws no number, label or legend over a graph, and carries one visible config button in fixed page chrome with theme and mode only in its menu; the index lists all ten and the owner picks one
  - evidence: ten round-2 variations 05.01-05.10 pass all ten gates, 14.5-18.8 percent pixel difference from 05; tmp/workflows/btop-web-r2-variations/checkpoints/verify.json; owner picked 05.06-network-paired
  - test: open each 05.01-05.10 page at 1440x900; assert no page scroll, three GPU boxes, and zero elements overlapping a chart box
  - test-tags: MANUAL
  - mechanism: 2026-10-25T15:18:08Z @kj each variation keeps 05's panel anatomy and Grafana density and varies one presentation axis; readouts live in stat tiles and header lines outside the plot area
  - log: 2026-10-25T15:18:08Z @kj added
  - log: 2026-10-25T15:19:45Z @kj owner refinement 2026-10-25: mix in the readout insights from earlier rounds - move numbers into the panel header or another arrangement outside the plot - and vary the arrangement across the ten so they can be compared
  - log: 2026-10-25T20:37:16Z @kj amended title "Ten variations of the chosen Grafana-panels design" -> "Ten legibility variations of the accepted 05 concept"; text "ten variations 05.01-05.10 of 05-grafana-panels as standalone pages under docs/design/concepts; each fits 1440x900 with no page scroll, shows the three GPUs, uses house chips and a config menu, and draws no number, label or legend over a graph; the index lists all ten and the owner picks one" -> "Ten legibility variations 05.01-05.10 of the accepted 05-grafana-panels page; each keeps 05's panel grid, panel set and chart types and varies only legibility - type scale, weight, contrast, spacing, alignment, label density, chart line treatment - never a new chart type or restructured layout; each fits 1440x900, shows the three GPUs, draws no number, label or legend over a graph, and carries one visible config button in fixed page chrome that opens the theme and mode menu, with no theme control in the body; the index lists all ten and the owner picks one"; reason: owner correction 2026-10-25: the ten must stay within the accepted 05 concept and vary legibility only, and every page needs a visible config button
  - log: 2026-10-25T21:10:00Z @kj amended title "Ten legibility variations of the accepted 05 concept" -> "Ten small imaginative variations of the accepted 05 concept"; text "Ten legibility variations 05.01-05.10 of the accepted 05-grafana-panels page; each keeps 05's panel grid, panel set and chart types and varies only legibility - type scale, weight, contrast, spacing, alignment, label density, chart line treatment - never a new chart type or restructured layout; each fits 1440x900, shows the three GPUs, draws no number, label or legend over a graph, and carries one visible config button in fixed page chrome that opens the theme and mode menu, with no theme control in the body; the index lists all ten and the owner picks one" -> "Ten small variations 05.01-05.10 of the accepted 05-grafana-panels page; each is one visible imaginative idea over 05 - a design move a reader sees at a glance - while keeping 05's identity, data and three GPUs; no page is a near-clone of 05 or of another, and no page is a new concept; each fits 1440x900, draws no number, label or legend over a graph, and carries one visible config button in fixed page chrome with theme and mode only in its menu; the index lists all ten and the owner picks one"; reason: owner correction 2026-10-25: the legibility round repeated 05 near 1:1; variations must be small but visible imaginative departures
  - log: 2026-10-26T18:59:44Z @kj closed
- [x] `ACC-DESIGN-44-ADAPTIVE-CONCEPTS` **Concepts for adaptive and responsive design** - HIGH; concept pages in docs/design/concepts/ show how the chosen page adapts across device classes and responds fluidly between them: desktop, laptop, tablet and phone, which panels reflow, shrink or drop at each breakpoint; owner asked 2026-10-26
  - evidence: four strategy pages 05.06-adaptive-1-priority, 2-container, 3-split, 4-tabs plus 05.06-adaptive-base; 9 of 9 gates: 681 sweep renders clean, adaptive-spec matched at 20 named renders, contrast in 10 variants; index section with 25 shots; wf_02b3af5d-270 tmp/workflows/btop-web-adaptive/checkpoints/verify-2.json
  - test: open each concept at 1440x900, 1280x800, 1024x768, 768x1024 and 390x844: no page scroll, the stated layout per breakpoint
  - test-tags: MANUAL
  - log: 2026-10-26T15:51:13Z @kj added
  - log: 2026-10-26T16:56:38Z @kj owner chose: four pages, one adaptive strategy each, each fluid between its breakpoints
  - log: 2026-10-26T19:07:45Z @kj closed
  - log: 2026-10-26T19:10:55Z @kj owner picked 05.06-adaptive-1-priority for the shipped page; recorded as ACC-LAYOUT-46-ADAPTIVE-PRIORITY

## Configuration `CONF`

Startup options, defaults and precedence

- log: 2026-10-25T08:01:10Z @kj category created

- [x] `ACC-CONF-25-PORT-DERIVED` **Default port derives from the name** - HIGH; the default listen port derives from btop-web: fold its UTF-8 bytes as v = (v*256 + byte) mod 64512, then port = 1024 + v, which is 40290. A config file, --config or BTOP_WEB_LISTEN overrides it. The rule and the result are documented in the shipped config
  - evidence: --default-config prints listen = 127.0.0.1:40290; no config binds 40290; --config and BTOP_WEB_LISTEN override (verify.json)
  - test: run with no configuration; assert the server binds 127.0.0.1:40290 and --default-config prints that port; override by env and file and assert precedence
  - test-tags: UNIT, E2E
  - mechanism: 2026-10-25T08:01:10Z @kj one function folds the name bytes at compile time; the config default uses its result
  - log: 2026-10-25T08:01:10Z @kj added
  - log: 2026-10-25T08:41:40Z @kj closed
- [x] `ACC-CONF-26-BTOP-FLAGS` **btop command-line flags are honoured** - HIGH; the binary accepts btop 1.3.0's own flags with btop's effects: -c/--config, -d/--debug, -f/--filter, -h/--help, -u/--update, -v/--version. The terminal-only flags -b/--boxes, -l/--low-color, -p/--preset, -t/--tty_on and --utf-force are recognised and refused with a clear not-applicable message
  - evidence: -v prints btop-web 0.1.0 exit 0; -h usage; -u 5000 gives 5.000 s SSE; -f python rendered; -d logs DEBUG; -c applied; -b/-l/-p/-t/--utf-force exit 2
  - test: run each flag and assert its effect; run each terminal flag and assert the refusal message and exit status
  - test-tags: UNIT, E2E
  - mechanism: 2026-10-25T08:01:10Z @kj the hand-written parser in main.rs maps btop's spellings onto the existing options and rejects the terminal set
  - log: 2026-10-25T08:01:10Z @kj added
  - log: 2026-10-25T08:41:40Z @kj closed
- [-] `ACC-CONF-28-THEME-NUMBER` **Themes are selected by number** - HIGH; One ordered list numbers the shipped themes from 0: Default, Everforest Medium, Flexoki, Gruvbox, Solarized. The theme = N key and --theme N flag select one; out of range stops startup and lists the range. With no stored preference the page starts on the configured theme; a stored choice wins.
  - test: run --theme N for N in 0..=4 and assert data-theme matches THEMES[N]; assert --theme 5 exits non-zero and prints 0..=4; set theme = 2 in a config file and assert flexoki
  - test-tags: UNIT, E2E
  - mechanism: 2026-10-25T08:04:50Z @kj one THEMES list in src/config.rs; the config default and the flag both index it; Config::theme_id feeds the template's data-theme and theme.js prefers a stored choice
  - log: 2026-10-25T08:04:50Z @kj added
  - log: 2026-10-25T08:32:59Z @kj rejected: duplicate of ACC-THEME-27-NUMBERED; reason: same requirement filed twice in one session; 27 carries the tracked text plus the stored-choice clause
- [x] `ACC-CONF-39-SAMPLE-RATE` **Default sampling every 250 ms** - HIGH; The default update interval is 250 ms instead of btop's 2000 ms; update_ms in a config file and -u still override it within 100..86400000; one shared sampler tick stays the only collection (ACC-PERF-16); owner asked 2026-10-26
  - evidence: wf_c29f79db-a67 verify-2 gate 10: no config gives 41 cpu events in 10 s, median gap 250 ms; -u 2000 gives median 2.000 s; config update_ms 500 honoured; tmp/workflows/btop-web-implement05/checkpoints/verify-2.json
  - test: start with no config and assert 4 ticks per second on /events; start with -u 2000 and assert 2000 ms; re-measure ten-client overhead at 250 ms
  - test-tags: UNIT, E2E
  - mechanism: 2026-10-26T14:40:39Z @kj the default constant in config.rs; README and help text state 250 ms
  - log: 2026-10-26T14:40:39Z @kj added
  - log: 2026-10-26T17:09:27Z @kj closed
- [x] `ACC-CONF-60-VIEW-FLAG` **Flag sets the starting process view** - MEDIUM; a command-line flag, --proc-view top, all or tree, sets the starting process view for viewers with no stored choice; precedence: the flag, then btop's proc_tree, then top-level only; the config menu changes it per viewer; --help lists it; a bad value exits 2 with a message; owner asked 2026-10-27
  - evidence: --proc-view top|all|tree sets the start view (34 of 34 flag cases), else tree when proc_tree is True, else top; bad value exits 2; proc-view wf_d3dbd6ab-57d verify-3 7 of 7 and final 5 of 5, make ci 228 green, live 40290 2eac7608; tmp/workflows/btop-web-proc-view/checkpoints/verify-3.json
  - related: ACC-VIEW-58-PROC-VIEW
  - test: start with each value: a fresh viewer opens in that mode; a bad value exits 2; --help shows the flag
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T12:19:56Z @kj added
  - log: 2026-10-28T07:31:15Z @kj closed; reason: evidence names the measured result and the checkpoint
- [x] `ACC-CONF-61-CONFIG-PAGES` **Config menu with pages per aspect** - HIGH; the config menu has pages General, CPU, GPU, Memory, Network and Processes, as btop's options menu groups them, plus Layout for panels, and covers more settings than today; with the read-only guard on it changes display options only, stored in the browser per viewer; with the guard off it also changes server settings and writes them to the btop config file btop-web loaded; built straight into the page; owner chose 2026-10-27
  - evidence: seven tabbed pages general, cpu, gpu, mem, net, proc, layout; display options per browser; 36 server options read-only by default, htmx POST config with Origin check, atomic one-line write, live apply via watch channel with --no-readonly; 896 sweep checks 0 failing; make ci 242 green; live 40290 25f05818; tmp/workflows/btop-web-config-pages/checkpoints/final-1.json
  - related: ACC-VIEW-58-PROC-VIEW,ACC-CONF-60-VIEW-FLAG
  - test: each page opens from the config menu; read-only: display options apply live and persist in the browser, server settings shown but not editable; read-write: a server setting changes for all viewers and lands in the config file
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T12:20:12Z @kj added
  - log: 2026-10-27T12:21:47Z @kj edited text "the config menu grows into pages, each configuring one aspect of the dashboard, and covers more settings than today's theme, mode and panels; owner asked 2026-10-27; page set, setting scope (per viewer or server-wide) and concept-first chosen by the owner" -> "the config menu has pages General, CPU, GPU, Memory, Network and Processes, as btop's options menu groups them, plus Layout for panels, and covers more settings than today; with the read-only guard on it changes display options only, stored in the browser per viewer; with the guard off it also changes server settings and writes them to the btop config file btop-web loaded; built straight into the page; owner chose 2026-10-27"; test "each page opens from the config menu, its settings take effect live, and the menu fits one viewport at every tier" -> "each page opens from the config menu; read-only: display options apply live and persist in the browser, server settings shown but not editable; read-write: a server setting changes for all viewers and lands in the config file"; reason: states the pages, both guard states and where each writes
  - log: 2026-10-28T09:02:56Z @kj closed; reason: evidence names the measured result and the checkpoint
  - log: 2026-10-28T09:41:47Z @kj owner 2026-10-28: the config file is read and written only when -c names it; without -c a read-write change applies live and is not saved
- [x] `ACC-CONF-63-TITLE-FLAG` **Flag sets the page title** - MEDIUM; a command-line flag, --title TEXT, sets the page title shown in the browser tab, default btop-web; the text is HTML-escaped; an empty or missing value exits 2 with a message; --help lists it; owner asked 2026-10-27
  - evidence: --title sets document.title as escaped text; missing, empty or blank value exits 2; --help lists it; proc-view wf_d3dbd6ab-57d verify-3 7 of 7 and final 5 of 5, make ci 228 green, live 40290 2eac7608; tmp/workflows/btop-web-proc-view/checkpoints/verify-3.json
  - related: ACC-VIEW-55-TAB-TITLE,ACC-CONF-60-VIEW-FLAG
  - test: start with --title 'lab 1': document.title reads lab 1; markup in the value renders as text; --title with no value exits 2
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T12:22:30Z @kj added
  - log: 2026-10-28T07:31:15Z @kj closed; reason: evidence names the measured result and the checkpoint

## Libraries `LIB`

Standard functionality comes from maintained crates, not hand-written code

- log: 2026-10-25T15:15:34Z @kj category created

- [x] `ACC-LIB-34-FIRST` **Libraries before own code** - HIGH; standard functionality comes from a maintained crate or a vendored library: interpolation, colour maths and contrast, parsing, formatting, protocols; own code exists only where no appropriate solution does, and each such case states why beside it
  - evidence: wf_55b18032-4c5: asset types from rust-embed mime_guess, blend() via palette Mix (CSS of 10 variants byte-identical), one-line reasons beside the remaining own routines, unused uptime code deleted; verify passed, make ci 145 tests; tmp/workflows/btop-web-round1-fix/patches/lib-1..4.patch, tmp/workflows/btop-web-round1-fix/checkpoints/final.json
  - test: audit src for hand-written routines with a standard equivalent; each survivor names why no crate fits
  - test-tags: UNIT
  - mechanism: 2026-10-25T15:15:34Z @kj the spline, palette and nvml paths already delegate; a new hand-written routine must justify itself in its module doc
  - log: 2026-10-25T15:15:34Z @kj added
  - log: 2026-10-27T10:34:12Z @kj closed

## Container scope `SCOPE`

what btop-web measures when it runs inside a container

- log: 2026-10-27T12:39:53Z @kj category created

- [x] `ACC-SCOPE-64-CONTAINER` **Inside a container, show only this container** - CRITICAL; when btop-web runs inside a container it always limits every panel to that container: processes, CPU, cores, load, memory, disks, network and GPU; a metric with no container-scoped source renders as unavailable, never the host value; detection never misses a Docker or Podman container; outranks btop collection parity inside a container; owner asked 2026-10-27
  - evidence: container detected at startup (/.dockerenv); CPU from cpu.stat 0.8 vs 0.73, memory 6.25 GiB vs cgroup 6.258 (VM 16.8), 109 process rows = 109 cgroup members, 0 outside; load, cores, GPU, IO unavailable, never host values; make ci 172 green; tmp/workflows/btop-web-container-scope/checkpoints/final-1.json
  - test: in this container: each panel's value matches its container source (cgroup v2 files, container namespaces), no host-wide value is shown; outside a container: unchanged btop parity
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-27T12:39:53Z @kj added; reason: names every panel, the unavailable rule, detection and the parity override
  - log: 2026-10-27T18:20:57Z @kj closed
  - log: 2026-10-27T19:30:32Z @kj DEF-GPU-40 and DEF-CPU-41 closed: GPUs passed into the container and the cpuset's cores are shown; load and disk IO stay unavailable; make ci 173


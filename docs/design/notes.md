# btop-web design system

The design system of the shipped page, read from `crates/btop-web/templates/`, `assets/app.css`, `assets/theme.js` and `assets/app.js`. Every colour is a semantic token computed from the btop palettes and measured against its WCAG floor. A section that records the earlier rendered proof, `docs/design/index.html`, says so in its first line.

| Item | Value |
|---|---|
| Page title | `btop-web` |
| Sources | `crates/btop-web/templates/` and `crates/btop-web/assets/` (the page), `docs/design/notes.md` (this system) |
| Source of truth | btop 1.3.0 at tag `v1.3.0`, commit `36842a3bccfb78f5c27642435bff387b42f7d13b`; theme files at btop main `87bb1610` |
| Theme model | Eight families, sixteen variants: fourteen file-backed variants in seven pairs plus the generated Default pair, which carries the JupyterLab Steel surfaces (section 3.3); mode is dark, light or auto (section 8.4) |
| Default state | Read-write; the config menu's server options are forms (section 8.14) and no READ-ONLY chip shows. `--readonly` shows the READ-ONLY chip, the server refuses every mutating request and the page has no mutating control |
| Layout | The `05.06-network-paired` page in section 9; one viewport with no page scroll at 1440x900, 1280x800, 1024x768 and 390x844 |
| Graphs | Server-built PCHIP SVG paths (`spline1d` crate) over a 1 s exponential moving average of a 240 s window (960 samples at the 250 ms default); one knot per bucket of at least one pixel column, fixed in time, and a steady left edge once the window is full; no overshoot; a very faint quarter grid under every plot except the process-row graphs; the network plots show a decade ceiling that a sample above it raises at once, that only a page load lowers, and that is never less than the decade of `net_floor` (section 7) |
| Motion | Two clocks: a compositor animation slides each plot's svg left between readouts and the server's one-slot tail extends the newest smoothed knot (`ACC-GRAPH-98-ANIM-CLOCK`); a scale change moves a network plot or the Processes count graph, and on a network plot its scale label, to the new ceiling over 1 s or more (`ACC-GRAPH-67-SMOOTH-SCALE`). A value changes only when a frame arrives (section 6) |
| Performance budget | One sampler tick per interval, 250 ms by default, fanned out to all clients; fragment-driven DOM; thresholds at 250 ms in section 11.3 |
| Scope | The host; inside a container, only that container, from its cgroup v2 files and namespaces (section 16, `ACC-SCOPE-64-CONTAINER`) |

## 1. Purpose

btop-web serves btop 1.3.0's telemetry to a browser with btop's sources, field names, units, formulas, delta windows and rounding. The design keeps btop's information architecture and drops terminal rendering: no braille, no box-drawing borders, no hotkey letters, no menus. Inside a container the page shows only that container and departs from btop's sources on the owner's order of 2026-10-27 (section 16).

The system exists so the implementation has one answer for every visual decision: which token, which size, which spacing step, which state, which threshold. A reviewer compares the page against btop running beside it; the numbers and the names must match, the pixels need not.

### 1.1 What the dashboard answers

- **Is the host healthy?** CPU, memory and network, in that order, above the fold.
- **What is doing it?** The process table, sorted by memory by default and by any column the viewer clicks.
- **What hardware is present?** Three NVIDIA GPUs, each with its own load plot and legend entry. The page draws no battery.
- **Can I change anything?** A viewer changes what their own browser shows, and the config menu also changes btop's server options for every viewer and writes them to the loaded config file (section 8.14). Under the read-only guard, which `--readonly` turns on, a viewer changes only what their own browser shows.

### 1.2 Audience

- Primary: the operator of one Linux host who knows btop and wants the same numbers on the LAN.
- Secondary: a phone user checking CPU, memory and network without a terminal.
- Tertiary: a reviewer checking collection parity and contrast.
- Not for: fleets, alerting, history storage, authenticated public exposure.

## 2. Principles

1. **Numbers first.** The value, its unit and its precision are the content; decoration is secondary. A number with no unit or no precision is not finished.
2. **Glance, then detail.** Three panels answer "healthy"; the process table answers "why".
3. **One theme layer.** Every colour is a CSS custom property. No colour literal appears outside the theme blocks that `theme.rs` writes into the page.
4. **Semantic over raw.** Text, borders and state use contrast-clamped semantic tokens. Raw btop gradients are confined to chart fills after a 3:1 check.
5. **State is explicit.** Live, paused, stale, read-only and unavailable are words as well as colour.
6. **No invented data.** A metric the collector cannot read is never drawn as `0`: a main value reads `unavailable`; a core number, mount percentage or process cell is left empty; a detail such as temperature is not drawn (section 8.11).
7. **No terminal cosplay.** Modern type, spacing and SVG charts, while keeping btop's panel names, field names, units, per-core list and process table.

## 3. Colour

### 3.1 Two-layer token model

The raw layer is btop's 48 theme keys in btop's order (`crates/btop-web/src/theme.rs:15-64`). Values stay verbatim. Missing keys follow btop's own fallbacks, not a blanket fill from Default:

- `meter_bg` and `graph_text` fall back to `inactive_fg`.
- `process_start`, `process_mid`, `process_end` fall back to `cpu_start`, `cpu_mid`, `cpu_end`.
- An empty gradient midpoint is generated by btop's two-stop rule, not invented.

The semantic layer is the only layer components read. Each semantic token is the raw btop colour moved toward black or white only as far as its WCAG floor requires (`crates/btop-web/src/theme.rs:216-292,496-532`). Raw values remain available for btop-faithful gradients; a chart fill that cannot reach 3:1 against the surface is replaced by its clamped chart token.

### 3.2 Semantic token contract

| Token | Role | Floor | Raw source |
|---|---|---:|---|
| `--surface` | Page and panel background | - | `main_bg` |
| `--surface-raised` | Control background, surface blended 2% toward text | text 4.5:1 on the derived value | derived |
| `--surface-sunken` | Equal to `--surface-raised`; defined in app.css, read by no shipped rule | text 4.5:1 on the derived value | derived |
| `--text` | Body text and values | 4.5:1 | `main_fg` |
| `--text-strong` | Panel titles, primary values and links | 4.5:1 | `title` |
| `--muted` | Labels, units, sample age, secondary text | 4.5:1 | dimmed `main_fg` |
| `--accent` | Focus ring, link underline, interactive affordance | 3:1 | `hi_fg` |
| `--selection-bg` | Selected process row | - | `selected_bg` |
| `--selection-fg` | Selected row text | 4.5:1 on `selection-bg` | `selected_fg` |
| `--border` | Panel and control boundary | 3:1 | `div_line` |
| `--border-subtle` | Table rules, decorative dividers | decorative | `border` blended toward surface |
| `--border-cpu` | Border of cpu-stat, cpu-gauge and cores | 3:1; none in Default | `cpu_box`; Default: Steel border (section 3.3) |
| `--border-mem` | Border of mem-stat, mem-gauge and storage | 3:1; none in Default | `mem_box`; Default: Steel border |
| `--border-net` | Border of net | 3:1; none in Default | `net_box`; Default: Steel border |
| `--border-proc` | Border of proc-stat, gpu and proc | 3:1; none in Default | `proc_box`; Default: Steel border |
| `--border-cores`, `--border-gpu`, `--border-storage` | Default only: border of cores, gpu and storage; the other themes leave them unset and fall back to the family token | none | Steel border |
| `--ok` | Healthy state, low utilization; process percentage text; core gradient start | 4.55:1 text | `cpu_start` |
| `--warn` | Warning state, mid utilization; process percentage text; core gradient middle | 4.55:1 text | `cpu_mid` |
| `--crit` | Critical state, high utilization; process percentage text; core gradient end | 4.55:1 text | `cpu_end` |
| `--chip-fg` | Chip label on its 12% hue tint | 4.5:1 on the composed tint | `text-strong`, clamped against the least-contrasting `--ok`, `--warn` or `--accent` tint |
| `--zebra` | Alternate process row background, 3% away from text | text 4.5:1 on the derived value | derived |
| `--row-hover` | Process row hover background, 1.25:1 against the plain and the zebra row | the cell tokens below at 4.5:1 on the derived value | derived |
| `--row-hover-text`, `--row-hover-muted`, `--row-hover-ok`, `--row-hover-warn`, `--row-hover-crit` | Hovered cell text | 4.5:1 on `--row-hover` | the named token where it holds 4.5:1, otherwise `--tip-text` |
| `--net-down-text` | Download arrow glyph | 4.5:1 | `download_end` clamped |
| `--net-up-text` | Upload arrow glyph | 4.5:1 | `upload_end` clamped |
| `--unavailable` | Unavailable text and dashed border | 4.5:1 text | `muted` |
| `--meter-track` | Unfilled meter and chart track | 3:1 | `--border` |
| `--grid-line` | Plot grid (`ACC-GRAPH-68-PLOT-GRID`); reads `--grid` | 1.1-1.6:1, very faint by design | `--grid`: surface blended 12% toward text |
| `--chart-cpu` | CPU series | 3:1 | `cpu_start` |
| `--chart-cpu-warn` | CPU mid band | 3:1 | `cpu_mid` |
| `--chart-cpu-crit` | CPU high band | 3:1 | `cpu_end` |
| `--chart-mem-used` | Memory used and disk used | 3:1 | `used_end` clamped |
| `--chart-mem-available` | Memory available | 3:1 | `available_end` clamped |
| `--chart-mem-cached` | The other used and other reclaimable segments of the Memory bar and keys, mixed in `app.css` (`ACC-VIEW-190-MEM-RECLAIM`) | 3:1 | of `cached_end`, `cached_mid` and `cached_start` clamped, the one farthest by CIEDE2000 from used, muted and free |
| `--chart-mem-free` | Memory free | 3:1 | `free_end` clamped |
| `--chart-net-down` | Download series | 3:1 | `download_end` clamped |
| `--chart-net-up` | Upload series | 3:1 | `upload_end` clamped |
| `--chart-proc` | Process count plot (`proc-stat`) | 3:1 | `process_mid` clamped |
| `--status-ok` | Alias of `--ok`; defined in app.css, read by no shipped rule | 4.5:1 | `--ok` |
| `--status-warn` | Alias of `--warn`; defined in app.css, read by no shipped rule | 4.5:1 | `--warn` |
| `--status-crit` | Alias of `--crit`; defined in app.css, read by no shipped rule | 4.5:1 | `--crit` |

### 3.3 Semantic values per variant

Sixteen variants. The fourteen file-backed variants are Gruvbox, Solarized, Everforest, Flexoki, Synthwave, Terracotta and Ultraviolet, dark and light. The four original pairs keep btop's theme-file values unchanged, and their exact values are in Appendix A; the three added pairs, 5 Synthwave (futuristic cyan and magenta), 6 Terracotta (warm clay, olive and teal) and 7 Ultraviolet (violet, sky blue and rose), define all 48 keys in their theme files. Default dark is btop's `Default_theme` with the `STEEL_DARK` overrides in `theme.rs`, and Default light is the HSL lightness inversion of Default dark with the `STEEL_LIGHT` overrides (`ACC-THEME-65-STEEL-DEFAULT`). The overrides apply only in `ThemePair::default_theme()`; `DEFAULT_DARK` stays btop's table, because `with_defaults()` fills the keys a theme file omits from it.

**Default palette sources.** Default is chosen to match a JupyterLab tab in either mode. The references are the Galaxa Steel Dark and Steel Light JupyterLab themes and the standard JupyterLab light theme.

- Dark neutrals come from Steel Dark: the page is `#272b31`, the owner's value, 2 RGB units from Steel Dark `--jp-layout-color0` `#252b32`; text is `--jp-ui-font-color0` `#c3c3c3`; `div_line` is `--jp-border-color1` `#404b54`
- Light follows the JupyterLab light theme first and takes Steel Light as a hint: the page is `--jp-layout-color0` white, strong text is `--jp-ui-font-color0` `#000000`, text is Steel Light `--jp-ui-font-color0` `#23262a`, and `div_line` `#dde0e3` is JupyterLab `--jp-border-color2` lightness at Steel Light `--jp-layout-color2` hue and saturation
- Inner lines (`div_line`) sit one step below the panel borders, as in btop and JupyterLab
- The other colours are btop's Default with the saturation capped: every key above 70% HSL saturation drops to 70%, hue and lightness kept. Ten keys change and three are visible: `--chart-mem-used`, `--chart-mem-free` and `--chart-gpu-2`. Apart from the network lines in the next bullet, no hue family changes; the largest hue move over the other chart and meter tokens is 0.64 degrees. The other semantic tokens differ from btop's Default only because `ensure_contrast` measures them against the Steel surface
- The network lines are not btop's two violets, which differ by CIEDE2000 11 (`ACC-GRAPH-72-NET-COLOURS`, the owner's request after the Steel change): `download_end` is a muted steel blue `#86acd6` and `upload_end` a muted amber `#d9a978`, `#294f79` and `#875726` in light, 35.9 apart in dark and 38.3 in light. The plot lines, areas, legend swatches and arrows read them. `download_mid` stays btop's, because GPU series 1 reads it; the other download and upload keys have no reader. `default_download_and_upload_lines_differ` in `theme.rs` holds both modes at 30 or more
- The panel borders are the muted Steel border in both modes, described under the panel border table below

| Variant | surface | text | text-strong | muted | accent | border | selection-bg | selection-fg | ok | warn | crit |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Default Dark | #272b31 | #c3c3c3 | #eeeeee | #909293 | #bc5252 | #6c757c | #6a2f2f | #eeeeee | #77ca9b | #cbc06c | #e36f6f |
| Default Light | #ffffff | #23262a | #000000 | #747679 | #bf4a4a | #929496 | #d09595 | #111111 | #348457 | #80762d | #b32323 |
| Gruvbox Dark | #1d2021 | #a89984 | #ebdbb2 | #918573 | #d79921 | #a89984 | #282828 | #fabd2f | #b8bb26 | #d79921 | #fb4934 |
| Gruvbox Light | #fbf1c7 | #3c3836 | #3c3836 | #736d60 | #cc241d | #968976 | #f2e5bc | #8f3f71 | #407856 | #8e6516 | #cc241d |
| Solarized Dark | #002b36 | #eee8d5 | #fdf6e3 | #80918b | #b58900 | #5f747a | #073642 | #d6a200 | #adc700 | #d6a200 | #e96630 |
| Solarized Light | #fdf6e3 | #586e75 | #002b36 | #60747a | #b48800 | #859191 | #eee8d5 | #836300 | #687700 | #8e6b00 | #c44714 |
| Everforest Dark | #2d353b | #d3c6aa | #d3c6aa | #a39c8a | #e67e80 | #767e81 | #3d484d | #dbbc7f | #a7c080 | #dbbc7f | #f97371 |
| Everforest Light | #fdf6e3 | #5c6a72 | #5c6a72 | #67737a | #d765b3 | #4f585e | #4f585e | #ecc86b | #697701 | #936900 | #c44341 |
| Flexoki Dark | #100f0f | #cecdc3 | #fffcf0 | #7d7c76 | #da702c | #615f5e | #403e3c | #fffcf0 | #3aa99f | #d0a215 | #d25145 |
| Flexoki Light | #fffcf0 | #343331 | #100f0f | #76746f | #bc5215 | #93928b | #e6e4d9 | #100f0f | #238079 | #926e01 | #af3029 |

**Tint and net text tokens.** `--chip-fg` is `--text-strong` clamped against the 12% `--ok`, `--warn` or `--accent` tint that leaves the least contrast, so a chip label stays at 4.5:1 on its own tint. `--zebra` is the surface blended 3% away from the text colour, so the row tint separates rows without spending text contrast. `--row-hover` is the smallest blend of the surface toward black or white that reaches 1.25:1 against the plain and the zebra row, on the side that leaves the text the most contrast (ACC-VIEW-198-ROW-HOVER). Six variants step away from the text: Default, Everforest, Gruvbox, Solarized and Steel dark, and Steel light. The other fourteen have room only toward the text, so a token that falls below 4.5:1 on the step paints as `--tip-text` in the hovered row. `--net-down-text` and `--net-up-text` are the raw graph endpoints clamped to 4.5:1 against the surface. Default light has no room for the away blend, because its surface is pure white: `--zebra` equals the surface, and Default light carries no row tint. Default dark, on the Steel surface, and the other fourteen variants keep the row tint.

| Variant | chip-fg | zebra | row-hover | net-down-text | net-up-text |
|---|---|---|---|---|---|
| Default Dark | #eeeeee | #262a30 | #131518 | #86acd6 | #d9a978 |
| Default Light | #000000 | #ffffff | #e4e4e4 | #294f79 | #875726 |
| Gruvbox Dark | #ebdbb2 | #1c1f20 | #000101 | #a3a8f7 | #d3869b |
| Gruvbox Light | #3c3836 | #fbf1c9 | #e1d8b3 | #75700e | #966211 |
| Solarized Dark | #fdf6e3 | #002a34 | #001116 | #a3a8f7 | #f56caf |
| Solarized Light | #002b36 | #fdf6e4 | #e2dccb | #6a6da1 | #b24e7f |
| Everforest Dark | #d3c6aa | #2c3339 | #1c2125 | #7fbbb3 | #e67e80 |
| Everforest Light | #5b6971 | #fdf6e4 | #e2dccb | #2e78a0 | #ab518f |
| Flexoki Dark | #fffcf0 | #100f0f | #262626 | #3aa99f | #8b7ec8 |
| Flexoki Light | #100f0f | #fffcf0 | #e5e2d8 | #24817a | #5e409d |

**Derived surfaces and decorative rules.** `--surface-raised` and `--surface-sunken` are the surface blended 2% toward text; `--border-subtle` is the border blended 50% toward surface; `--unavailable` is `--muted`. Muted text on the blended surfaces measures 4.31-4.45:1, so muted text never sits there; controls that carry muted text use `--surface`.

| Variant | surface-raised | surface-sunken | border-subtle | unavailable |
|---|---|---|---|---|
| Default Dark | #2a2e34 | #2a2e34 | #404b54 | #909293 |
| Default Light | #fbfbfb | #fbfbfb | #dde0e3 | #747679 |
| Gruvbox Dark | #202223 | #202223 | #625c52 | #918573 |
| Gruvbox Light | #f7edc4 | #f7edc4 | #c8bd9e | #736d60 |
| Solarized Dark | #052f39 | #052f39 | #2f4f58 | #80918b |
| Solarized Light | #faf3e1 | #faf3e1 | #c1c4ba | #60747a |
| Everforest Dark | #30383d | #30383d | #525a5e | #a39c8a |
| Everforest Light | #faf3e1 | #faf3e1 | #a6a7a0 | #67737a |
| Flexoki Dark | #141313 | #141313 | #383736 | #7c7b75 |
| Flexoki Light | #fbf8ec | #fbf8ec | #c9c7be | #76746f |

**Chart and state tokens.** Every chart token reaches 3:1 against its surface. `--chart-cpu` drives the CPU plot; the per-core meters show the `--ok`, `--warn`, `--crit` gradient (section 8.8); `--chart-proc` drives the process count plot; the gauge arcs take `--ok`, `--warn` or `--crit` by band; `--chart-mem-used` drives the memory plot, the storage used meter and the memory bar's used segment, `--chart-mem-free` the storage free meter and the bar's free segment, and `--chart-mem-cached` its cache segment (`--muted` its other segment); the network tokens drive the two network plots; the GPU lines use `--chart-gpu-0` to `--chart-gpu-2` (section 7.3); the live chip uses `--ok`, and `--warn` when paused or stale. app.css defines `--chart-cpu-warn`, `--chart-cpu-crit`, `--chart-mem-available`, `--surface-sunken` and `--status-*`, and no shipped rule reads them.

| Variant | chart-cpu | chart-cpu-warn | chart-cpu-crit | chart-mem-used | chart-mem-available | chart-mem-cached | chart-mem-free | chart-net-down | chart-net-up | chart-proc | meter-track | grid-line | status-ok | status-warn | status-crit |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Default Dark | #77ca9b | #cbc06c | #dc4c4c | #e3637a | #dcaa37 | #86daea | #d4ed97 | #86acd6 | #d9a978 | #dcd179 | #6c757c | #3a3d43 | #77ca9b | #cbc06c | #e36f6f |
| Default Light | #358859 | #938834 | #b32323 | #9c1c33 | #c89623 | #2190b8 | #4f6812 | #294f79 | #875726 | #867b23 | #929496 | #e5e5e5 | #348457 | #80762d | #b32323 |
| Gruvbox Dark | #b8bb26 | #d79921 | #fb4934 | #fb4934 | #fabd2f | #74e6fc | #98971a | #a3a8f7 | #d3869b | #dcd179 | #a89984 | #2e2f2d | #b8bb26 | #d79921 | #fb4934 |
| Gruvbox Light | #427b58 | #d79921 | #cc241d | #cc241d | #b57614 | #076678 | #427b58 | #79740e | #b57614 | #af3a03 | #968976 | #e4dbb6 | #407856 | #8e6516 | #cc241d |
| Solarized Dark | #adc700 | #d6a200 | #e65317 | #e02f30 | #edb400 | #74e6fc | #bad600 | #a3a8f7 | #f56caf | #dcd179 | #5f747a | #1d4249 | #adc700 | #d6a200 | #e96630 |
| Solarized Light | #849700 | #d6a200 | #e65317 | #e02f30 | #edb400 | #114061 | #839700 | #8589c9 | #e063a0 | #989054 | #859191 | #e9e6d6 | #687700 | #8e6b00 | #c44714 |
| Everforest Dark | #a7c080 | #dbbc7f | #f85552 | #f85552 | #a7c080 | #7fbbb3 | #a7c080 | #7fbbb3 | #e67e80 | #e67e80 | #767e81 | #414648 | #a7c080 | #dbbc7f | #f97371 |
| Everforest Light | #849701 | #dfa000 | #f85552 | #f85552 | #8da101 | #33a077 | #849701 | #3994c5 | #d765b3 | #d765b3 | #4f585e | #eae5d5 | #697701 | #936900 | #c44341 |
| Flexoki Dark | #3aa99f | #d0a215 | #d14d41 | #d14d41 | #da702c | #2361a7 | #879a39 | #3aa99f | #8b7ec8 | #d0a215 | #615f5e | #272625 | #3aa99f | #d0a215 | #d25145 |
| Flexoki Light | #24837b | #ad8301 | #af3029 | #af3029 | #bc5215 | #205ea6 | #66800b | #24837b | #5e409d | #ad8301 | #93928b | #e7e4d9 | #238079 | #926e01 | #af3029 |

Panel border tokens equal `--border` in six variants. Default and Flexoki carry distinct panel rules:

| Variant | border-cpu | border-mem | border-net | border-proc |
|---|---|---|---|---|
| Default dark | `#476b69` | `#47606b` | `#47556b` | `#47496b` |
| Default light | `#b0c8ca` | `#b0bfca` | `#b0b6ca` | `#b2b0ca` |
| Flexoki dark | `#4385be` | `#879a39` | `#ce5d97` | `#3aa99f` |
| Flexoki light | `#205ea6` | `#66800b` | `#a02f6f` | `#24837b` |
| Gruvbox, Solarized, Everforest | same as `--border` | same | same | same |

**Default panel borders** (`ACC-THEME-66-BOX-BORDERS`). A Default panel border is the Steel border colour with a small hue shift per panel and no saturated colour. The dark base is Steel Dark `--jp-border-color0` `#4d5a65`; the light base is `#b7bcc3`, JupyterLab light `--jp-border-color1` `#bdbdbd` lightness at Steel Light `--jp-layout-color2` hue and saturation.

- The four family tokens keep the base lightness at HSL saturation 20% and shift the hue by -30 degrees (cpu), -10 (mem), +10 (net) and +30 (proc)
- Default gives cores, gpu and storage their own tokens, because removing a panel lets any of them touch any family. Each keeps the hue of its fallback family (cpu, proc, mem) at saturation 2-7% with a lightness step of about 4 points, cores and gpu lighter, storage darker: dark `#5f6867`, `#5f5f68`, `#4c5051`; light `#c4cacb`, `#c3c2ca`, `#b1b3b4`
- The seven values are `DEFAULT_PANEL_BORDERS` in `theme.rs`. `render_css` writes them as two more Default blocks after the Default theme blocks, which still carry btop's `*_box` keys and their 3:1 lift; `:root` and the other themes get no per-panel token, so `app.css` falls back to the family token there
- Panel borders carry no contrast floor in Default, because each panel is named by its `<h2>` title. They measure 1.65-2.48:1 dark and 1.66-2.11:1 light; JupyterLab's own `#bdbdbd` border on white measures 1.88:1. `--border` (chips, option buttons, meter tracks, the popover) keeps its 3:1 floor
- `default_panel_borders_are_muted_steel` in `tests/theme_contrast.rs` holds every token within 35 degrees of hue, at most 25% saturation and 12 points from the base saturation, and within 5 lightness points of the base. The four families differ by CIEDE2000 4 or more; cores, gpu and storage differ by 5 or more from every other panel border

Selection tokens for the process row:

| Variant | selection-bg | selection-fg | contrast |
|---|---|---|---|
| Default dark | `#6a2f2f` | `#eeeeee` | 8.78:1 |
| Default light | `#d09595` | `#111111` | 7.57:1 |
| Gruvbox dark | `#282828` | `#fabd2f` | 8.69:1 |
| Gruvbox light | `#f2e5bc` | `#8f3f71` | 5.36:1 |
| Solarized dark | `#073642` | `#d6a200` | 5.59:1 |
| Solarized light | `#eee8d5` | `#846400` | 4.51:1 |
| Everforest dark | `#3d484d` | `#dbbc7f` | 5.16:1 |
| Everforest light | `#4f585e` | `#ecc86b` | 4.51:1 |
| Flexoki dark | `#403e3c` | `#fffcf0` | 10.36:1 |
| Flexoki light | `#e6e4d9` | `#100f0f` | 15.00:1 |

### 3.4 Contrast audit

Measured with the WCAG 2.x relative-luminance formula against each variant's own surface. Normal text needs 4.5:1; large text and non-text UI or graphics need 3:1.

| Variant | Body 4.5 | Strong 4.5 | Muted 4.5 | Muted on raised 4.5 | Accent 3.0 | Border 3.0 | Selected 4.5 | Status min 4.5 | Chart min 3.0 | Track 3.0 | Grid 1.1-1.6 |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| Default Dark | 8.07 | 12.26 | 4.55 | 4.37 | 3.03 | 3.03 | 8.78 | 4.59 | 3.03 | 3.03 | 1.31 |
| Default Light | 15.19 | 21.00 | 4.56 | 4.40 | 4.89 | 3.04 | 7.57 | 4.58 | 3.61 | 3.04 | 1.26 |
| Gruvbox Dark | 5.90 | 11.95 | 4.53 | 4.42 | 6.61 | 5.90 | 8.69 | 4.77 | 3.75 | 5.90 | 1.22 |
| Gruvbox Light | 10.22 | 10.22 | 4.53 | 4.37 | 4.82 | 3.02 | 5.36 | 4.58 | 3.02 | 3.02 | 1.23 |
| Solarized Dark | 12.25 | 13.92 | 4.53 | 4.31 | 4.68 | 3.05 | 5.59 | 4.58 | 3.30 | 3.05 | 1.38 |
| Solarized Light | 4.99 | 13.92 | 4.55 | 4.44 | 3.01 | 3.02 | 4.57 | 4.57 | 3.01 | 3.02 | 1.16 |
| Everforest Dark | 7.38 | 7.38 | 4.56 | 4.37 | 4.55 | 3.01 | 5.16 | 4.58 | 3.81 | 3.01 | 1.30 |
| Everforest Light | 5.18 | 5.18 | 4.52 | 4.40 | 3.04 | 6.74 | 4.51 | 4.57 | 3.03 | 6.74 | 1.17 |
| Flexoki Dark | 11.98 | 18.62 | 4.57 | 4.43 | 5.77 | 3.01 | 10.36 | 4.56 | 4.42 | 3.01 | 1.27 |
| Flexoki Light | 12.28 | 18.62 | 4.54 | 4.39 | 4.69 | 3.04 | 15.00 | 4.59 | 3.39 | 3.04 | 1.24 |

Ten variants are tabulated. The three added pairs, 5 Synthwave, 6 Terracotta and 7 Ultraviolet, are enforced by `theme_contrast.rs` against the same floors in both modes: text 4.5:1 composed, graphics and borders 3:1, grid 1.1-1.6:1, GPU series CIEDE2000 above 20, and the core gradient at 4.5:1 at every value 0-100.

Findings that constrain use:

- **All normal-text pairs pass.** Among the ten tabulated, the tightest body text is Solarized light at 4.99:1; the tightest selected-row text is Everforest light at 4.51:1.
- **Muted on `--surface-raised` or `--surface-sunken` fails 4.5:1** (4.31-4.45:1). Muted text therefore never sits on a blended surface. Controls that carry muted text use `--surface`; controls that carry `--text` may use `--surface-raised`. The table header is muted on `--surface`, not on a sunken track. The config popover carries `--text` labels on `--surface-raised`, and its selected option chip reads `--chip-fg` on the 12% `--accent` tint over that surface.
- **`--ok`, `--warn` and `--crit` are text tokens at 4.55:1.** The process table paints them as 12 px percentages and the core values mix them in linear light (section 8.8), so every variant derives them 0.05 above the text floor in both modes, which keeps a mix above 4.5:1 after the browser rounds it to 8 bits; among the ten tabulated, the lowest stop is 4.56:1 (Flexoki dark). Default light `ok` is `#348457` and `warn` is `#80762d`. A status dot or meter may use them. The READ-ONLY badge keeps `--warn` on its border and `--chip-fg` in its label.
- **Text on a tinted layer is measured on the composed layer.** A chip label is clamped against the 12% tint of its hue, the alternate process row and its hover move the surface 3% and 7% away from the text colour, and the network arrows use clamped text variants of the raw graph endpoints. The composed-layer test `text_on_tinted_layers_reaches_4_5_to_1` in `crates/btop-web/tests/theme_contrast.rs` composes each layer and asserts 4.5:1 for every text token that can sit on it, for all eight theme pairs and both modes. Default light has no room for the away blend on its white surface, so it carries no row tint; Default dark and the other fourteen variants do.
- **Every chart token reaches 3:1.** Among the ten tabulated, the lowest is 3.01:1. A raw btop endpoint below 3:1 is replaced by its chart token, never used directly.
- **Every border token reaches 3:1, except the Default panel borders** (`ACC-THEME-66-BOX-BORDERS`, section 3.3). Among the ten tabulated, the lowest is 3.01:1. `--border-subtle` is decorative and never carries a required boundary.
- **The plot grid stays under 3:1 on purpose.** `--grid` is the surface blended 12% toward the text colour, 1.16-1.38:1 in the ten tabulated, so the grid reads as a faint guide under the lines and never competes with a series (`ACC-GRAPH-68-PLOT-GRID`). It carries no information of its own: the axis labels under the plots name the time steps. `plot_grid_is_very_faint_in_every_pair_and_mode` in `tests/theme_contrast.rs` holds every pair and mode inside 1.1-1.6:1.
- **Raw `inactive_fg` fails in the ten tabulated pairs (1.00-2.61:1)** and never carries text. Raw `div_line` fails in several of them (1.33-2.79:1) and never carries a required boundary. `--border-subtle` resolves to the raw `div_line` through `app.css`.
- **Solarized light raw values are the reason the semantic layer exists.** `selected_fg` on `selected_bg` is 2.62:1 raw and `hi_fg` on `main_bg` is 2.98:1 raw; both are clamped above.

### 3.5 Colour usage rules

- Components read semantic tokens only. A raw btop key in a component style is a defect.
- A layer that carries text is composed before it is measured: a chip label is clamped against its 12% hue tint, and the zebra and hover rows blend away from the text colour. A rule that blends a text layer toward the text colour is a defect.
- No colour is the only encoding: status carries a word, read-only carries the words `READ-ONLY`, utilization carries a number, core rows carry a number.
- A gradient fill may use btop's raw stops; a flat meter uses the clamped chart token.
- `forced-colors: active` has no rule in `app.css` and has not been checked (section 10).
- A new theme is added as two blocks, `html[data-theme="<id>"][data-mode="dark"]` and `[data-mode="light"]`, with the same 79 token names. No component changes.

## 4. Typography

Historical: this section records the type scale of the earlier proof `docs/design/index.html`. The shipped sizes are in `app.css` and in sections 9.3 and 9.4.

### 4.1 Stacks

- Sans: `ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif`
- Mono: `ui-monospace, "SFMono-Regular", "Cascadia Mono", "DejaVu Sans Mono", Menlo, Consolas, monospace`
- No web font, no remote font, no icon font. System fonts only; the page must open without a network.

### 4.2 Scale and roles

| Size | Role | Family | Line height |
|---:|---|---|---|
| 11 px | Axis and scale labels, badges, micro labels, units, core cells, table headers | sans for labels, mono for numbers | 1.2 |
| 12 px | Status strip, table captions, subtitles, secondary values | sans | 1.45 |
| 13 px | Table cells, memory values, disk metadata, drawer values | mono for values, sans for metadata | 1.45 |
| 14 px | Body, controls, hero-adjacent values, network rates | sans for body, mono for values | 1.45 |
| 16 px | Panel titles, hero unit, drawer title | sans | 1.2 |
| 20 px | Wordmark | sans | 1.2 |
| 28 px | Phone hero metric | mono | 1.05 |
| 40 px | Desktop hero metric | mono | 1.05 |

Rules:

- Every telemetry value uses the mono stack with `font-variant-numeric: tabular-nums`; labels and prose use the sans stack.
- Table cells and axis numbers are tabular so a value changing from `9.9` to `10.0` does not shift its column.
- A live value with text after it, or in an item its neighbours place, reserves its widest form in `data-w` (`100%`; for a process count the digits of the count graph's ceiling, at least three; the core count for busy cores; `100` and `0000` for the CPU and memory numbers; seven digits for the Pid column while the table sizes its columns to content), so nothing beside it moves; the gauge readouts centre on the gauge axis with their unit as one block, every other reserved value starts at its label's left edge with the reserve after the number, the memory legend's stacked cells keep a right edge, the process facts sit in equal columns and each `load per core` value keeps four characters (`9.99`, `10.0`) (`ACC-LAYOUT-74-RESERVED-WIDTH`, `ACC-VIEW-87-NUMBER-PLACE`).
- Headings use 1.2 line height; numeric heroes use 1.05; body uses 1.45.
- No letter spacing on prose. `0.04em` and uppercase are reserved for 11 px micro labels.
- No font weight above 700. The 20 px wordmark is 700 (large text, 3:1 floor); panel titles are 650, labels 600, values 500, body 400.
- A value and its unit are one readout: the number is `--text-strong` or `--text`, the unit is `--muted` at 11-16 px. The unit never shrinks below 11 px.

## 5. Space, shape and elevation

Historical: this section records the spacing, radii and focus ring of the earlier proof `docs/design/index.html`. The shipped page uses an 8 px page padding and gap at 1440x900, 5-8 px along `--gap` (section 9.3), a 2 px radius on panels, chips and the popover, and a 1 px `--accent` focus outline at 1 px offset (sections 8 and 9).

### 5.1 Spacing scale

The base unit is 4 px. Only these steps are used; a value outside the scale is a defect.

| Step | Use |
|---:|---|
| 2 px | Hairline offsets, unit-to-number gap, label-to-value gap |
| 4 px | Core grid gap, control padding, badge inner gap |
| 8 px | Inside a component: meter-to-label, status items, table cell padding |
| 12 px | Panel inner block gap, control row gap, drawer section gap |
| 16 px | Panel padding, page padding at tablet and phone, header control gap |
| 24 px | Page padding at desktop, CPU inner column gap, footer top padding |
| 32 px | Footer bottom padding, large section separation |
| 48 px | Reserved for page-level separation; unused inside panels |
| 64 px | Header height at desktop and tablet |

### 5.2 Layout gaps and page padding

| Breakpoint | Columns | Gap | Page padding |
|---|---:|---:|---:|
| Desktop >= 1200 px | 12 | 20 px | 24 px |
| Tablet 768-1199 px | 8 | 16 px | 16 px |
| Phone <= 767 px | 1 | 12 px | 16 px |

The page is centered at `max-width: 1600px`. Panels are the only cards; there is no outer page frame. Every grid child has `min-width: 0`.

### 5.3 Radii

| Token | Value | Use |
|---|---:|---|
| `--r-control` | 4 px | Buttons, selects, inputs, core cells, GPU rows, dashed unavailable notes |
| `--r-panel` | 8 px | Panels, drawer, actions group |
| `--r-pill` | 999 px | Badges only |
| Meter radius | 6 px | Meter track and fill |

### 5.4 Borders and focus

- Panel: 1 px `--border` plus a 2 px top rule in the panel token (`--border-cpu`, `--border-mem`, `--border-net`, `--border-proc`). GPU uses `--border-proc`.
- Control: 1 px `--border`, 32 px minimum height (44 px on phone); 28 px in the status strip.
- Table: 1 px horizontal rules in `--border-subtle`; no vertical rules.
- Focus: 2 px `--accent` ring, 2 px offset, on every interactive element through `:focus-visible`.
- The table wrapper takes an inset ring (`outline-offset: -2px`) so the ring is not clipped by the scroll container.
- Disabled controls: `--surface` background, `--border` border, `--muted` text, `cursor: not-allowed`. Disabled is a state, not a colour.

### 5.5 Elevation

No drop shadows. Separation comes from surface, border, the 2 px panel top rule and the panel radius. A drawer uses the native dialog backdrop, which is the only overlay.

## 6. Motion

Two things move between frames, each on its own clock. A compositor animation that `app.js` starts on each plot's svg slides the plot left between readouts (`ACC-GRAPH-98-ANIM-CLOCK`); a network plot or the Processes count graph whose scale changes moves to the new ceiling over 1 s or more (`ACC-GRAPH-67-SMOOTH-SCALE`, section 7.4). Values smooth with a fixed 1 s EMA whatever the interval (section 7.1), and a value changes only when a frame arrives: motion moves geometry, never a sample. `app.css` declares the `rescale` and `chart-shift` `@keyframes` and no `transition`, so hover and a theme change apply at once.

| Property | Shipped |
|---|---|
| Animation clock | One linear CSS animation per chart's svg, restarted once per slow pass: it moves the svg left by one slot, the svg's width x slow period / window, over one slow period; the slow pass that follows lands exactly one slot on. The server draws a one-slot tail past the viewBox that extends the newest smoothed knot, so the curve reaches the right edge; the plot box clips it. Only the svg moves, never the grid the plot box paints, the axes, the labels or the readouts. On the svg, a CSS box, the animation needs no layout per frame (`ACC-PERF-133-PAGE-COST`) |
| Transitions and animations | One: `@keyframes rescale` scales the series group of a network plot or the Processes count graph to its new ceiling over `max(1000, 400 ln ratio)` ms, only while a scale change runs; the scroll moves the svg and the scale change the group inside it, so the two compose |
| Forbidden | A value animated between readouts, gauge glide, pulsing live dot, auto-scrolling table, layout transition, parallax, skeleton shimmer |
| Setting | The CONFIG general page's `motion` on/off chips stop and restart the animation clock without a reload; the choice is stored per browser under `btop-web-motion`, on by default. PAUSED, STALE and a hidden tab also stop it |
| Reduced motion | `app.js` does not consult the query: the CONFIG motion toggle is the only switch, because the owner's machine sets it on every page (`DEF-VIEW-72-MOTION-GATE`); `app.css` has no rule, so the scale change animates for every viewer |
| Updates | A frame writes what changed into its panels' nodes (section 9.2); an interpolated number is a number the host never reported, so the page draws none. The scale change moves the ceiling, never a sample |

Measured 2026-10-28, headless Chromium 156 with software compositing at 1440x900 and 60 fps: the animation clock costs 33.1 % of one core against 15.0 % with it off (+18.1 pp), and the main-thread task is 2.40 ms per frame against 1.25. Evidence: `tmp/workflows/btop-web-batch4-motion/checkpoints/anim-98.json`.

### 6.1 Motion between samples: rejected predictors, then the ordered clock

The owner asked on 2026-10-26 for plots and gauge arcs that move between samples toward a one-interval estimate from a fast Rust predictor, used only if it costs next to nothing and switched by a CONFIG toggle (`ACC-GRAPH-45-MOTION-PREDICT`). The predictor was measured and rejected, and so was a slide capped at 5-30 fps against a bar of 1 percentage point of one core. On 2026-10-28 the owner ordered the animation clock of section 6 (`ACC-GRAPH-98-ANIM-CLOCK`), which runs at the display frame rate and costs what section 6 states; the predictor was not built. The sweep below is the evidence for the rejected capped design.

- Trace: `crates/btop-web/tests/fixtures/traces/live-250ms.csv`, 600 s on the live host at 250 ms, 2401 ticks of 7 series (cpu, mem, net_down, net_up, gpu0-gpu2), integers only. The load was four CPU bursts up to 100 % and 31-56 MiB/s of other traffic on eth0
- Predictor: ema-step, estimate = s + alpha·(x − s), where s is the 1 s EMA and x the newest raw value; it has no state and no parameter. On the trace's test half (ticks 1204-2400) it moves the CPU plot's right edge by 0.203 px on average when the measured frame lands, against 0.310 px for hold-last (no estimate). The one-state and two-state Kalman filters, tuned on the raw readings, reach 0.237 and 0.238 px; ema-step equals a one-state Kalman filter with gain 1. On the network and GPU plots no estimate beats hold-last
- Server and wire cost: ema-step adds 0.7 ns per tick for 11 histories; the drawn estimate adds 0.03 to 0.11 % to the SSE bytes
- Client cost, first run, in headless Chrome 156 with software compositing at 1440x900: the proposed design (an SVG slide on 7 plots plus a gauge glide) added 51.8 percentage points (pp) of one core, and the cheapest composited slide (CPU and memory plots) added 5.7 pp. No glide target made the gauge step smaller than no glide: the CPU arc step was 3.53 % without a glide and 4.88-5.06 % with one
- The owner then set the bar at under 1 pp of one core, with the frame rate capped at the highest value under that bar and above the 4 frames per second the data arrives at

Frame-rate sweep, same browser, 250 ms interval, window full. Each cell is the median of 3 paired 20 s windows: the slide's CPU minus the mean of the windows with the slide off before and after it. The timer variant sets each plot's `translateX` from `setTimeout` after the `gpu` frame; the CSS variant runs `steps(n)` animations.

| fps | Steps after each frame (ms) | Timer, 5 plots (pp) | CSS, 5 plots (pp) | Timer, CPU and memory (pp) | CSS, CPU and memory (pp) |
|---:|---|---:|---:|---:|---:|
| 30 | 33, 67, 100, 133, 167, 200, 233 | +12.18 | +6.62 | +8.23 | +4.25 |
| 20 | 50, 100, 150, 200 | +8.69 | +5.78 | +5.55 | +2.97 |
| 15 | 67, 133, 200 | +5.68 | +5.25 | +5.61 | +3.88 |
| 12 | 83, 167 | +2.38 | +3.53 | +3.88 | +1.82 |
| 10 | 100, 200 | +3.82 | +4.30 | +3.38 | +3.99 |
| 8 | 125 | +2.50 | +3.60 | +1.62 | +2.43 |
| 6 | 167 | +1.50 | +3.60 | +1.78 | +2.94 |
| 5 | 200 | +1.52 | +3.17 | −0.48 | +2.40 |

- A replicate of the one-step points over 9 paired runs gave +1.48 pp for the timer at 5 fps on all five plots, of which the software compositor alone was +0.70 pp; +2.50 pp at 8 fps; and +1.73 pp at 5 fps on the CPU and memory plots, where the sweep's −0.48 did not hold
- The cap held: drawn frames per second were 4 plus 4 per step per interval, and paint and layout counts did not rise with the frame rate
- Decision NO_GO: no capped slide stays under 1 pp. The criterion was rejected on 2026-10-26
- Not measured: a GPU-composited desktop browser, and btop's 2000 ms interval
- Evidence: `tmp/workflows/btop-web-motion/spec.md` (trace, predictors, first cost run), `tmp/workflows/btop-web-kalman-fair/result.md` (Kalman rerun), `tmp/workflows/btop-web-motion2/spec.md` (sweep, replicate, decision)

### 6.2 Gauge arcs follow the EMA

The gauge arc has no animation of its own: each draws the 1 s EMA that its plot already holds (`ACC-GRAPH-47-GAUGE-SMOOTH`). The arc steps about a third as far per frame, and the number stays btop's raw value.

- Source: `History::ema()` in `chart.rs`, alpha = 1 − e^(−update_ms / 1000) = 0.2212 at 250 ms. `CpuSnapshot.gauge_pct` is `total_pct.and(history.ema())` and `MemSnapshot.gauge_pct` is `ram_percent.and(used_history.ema())`
- The `g-fill` path and its `ok`, `warn` or `crit` class read `gauge_pct`, so the arc's colour matches its length. The number in the hollow, the gauge's `aria-label` and the stat tile `#cpu-total` keep the raw value and its band
- An unavailable raw value makes `gauge_pct` `None`: the gauge draws no fill and reads `unavailable`
- Trace, CPU arc step per frame: 3.53 % raw against 1.19 % EMA on the test half (ratio 0.338), 2.57 % against 0.82 % over the whole trace, and a largest step of 90.0 % against 19.9 %. The arc lags the number by a mean of 4.2 % and a p99 of 41 %. Memory stays at 6-7 % on the trace, so its arc is unchanged in practice
- Live, verify round 1: the CPU arc stayed within 0.0034 points of the EMA of the raw readouts over 40 frames, and the number equalled the raw value in every frame. The step ratio was 0.23 on a quiet host; the ratio depends on the load
- No new state, no JavaScript and no CSS; the SSE bytes change only by the digits of the arc's `d`
- Tests: `chart::tests::ema_is_the_newest_smoothed_sample`, `mem::tests::gauge_pct_is_the_smoothed_used_percent`, `cpu::tests::seed_records_the_since_boot_point_and_the_first_tick_is_the_delta`, `http::gauge_arcs_draw_the_ema_and_readouts_stay_raw`

## 7. Data visualisation

### 7.1 PCHIP: the interpolation rule

Every plot is a PCHIP curve over smoothed samples, built on the server in `crates/btop-web/src/chart.rs` and sent as SVG path data in the SSE fragments. PCHIP is monotone piecewise cubic Hermite interpolation; the page draws no curve of its own.

- **Smoothing first** (`ACC-GRAPH-38-TEMPORAL`). `History::push` stores an exponential moving average with a 1 s time constant: each sample moves the average by `alpha = 1 - exp(-update_ms / 1000)`, which is 0.221 at the 250 ms default and 0.632 at 1 s. The constant is 1 s of wall-clock time whatever the interval, so at 250 ms it spans four samples and at 1 s one; a sparse sampler graphs choppier on purpose, and no path stretches the constant (`ACC-GRAPH-97-SMOOTH-REFRESH`). The first sample seeds it. The knots are the smoothed values. The readouts print btop's raw values and never read a history; the gauge arcs (section 6.2) and the core rows (section 8.8) draw the smoothed value.
- **Window.** A history holds `capacity(update_ms) = 240000 / update_ms` samples, clamped to 120-2400: 960 at the 250 ms default and btop's 120 at 2000 ms. Each tick pushes one sample, so every plot spans the same 240 s and the axis labels `240 s` to `now` sit under the network and GPU plots alike.
- **Placement.** The newest sample sits at the right edge. Sample `k` of `n` sits at `x = (slots - n + k) * width / (slots - 1)`, so a partial history starts mid-plot the way btop's graphs fill.
- **Interpolation.** `Spline::pchip` from the `spline1d` crate (0.1.1). Knots are exact, monotone runs stay monotone, and the slope limiter keeps every point inside the range of the samples around it, so the curve never leaves the series minimum or maximum. PCHIP is O(n) with no global solve.
- **Knots fixed in time** (`DEF-GRAPH-32-UNSTABLE-HISTORY`). The samples fall into buckets of `ceil(1 / slot width)` samples, counted from the history's absolute sample 0 rather than from the right edge, and each bucket's largest sample is a knot; the newest sample closes the line at the right edge. A complete bucket keeps its knot, so a peak keeps its height and moves left one sample step per tick. At 250 ms a bucket is 3 samples and each plot draws 320-321 vertices once the window is full. Sampling the curve at fixed pixel columns, as before, drew a spike's top anywhere between y 55.23 and 60.49 over 40 frames
- **Steady left edge** (`DEF-GRAPH-39-NET-JUMP`, the owner's decision of 2026-10-27, every chart). Until the window is full, the oldest sample starts the line and never changes. Once the window is full, the oldest sample is replaced every tick, so the line starts at the first complete bucket's knot; with PCHIP (every sample a knot) it starts one knot later, because PCHIP's end slope at the oldest sample changes when its left neighbour leaves. The first vertex is therefore a vertex the previous frame drew one slot to its right at the same height; when its bucket leaves the window, the next bucket's knot, already drawn, starts the line. The left end lies between x = 0 and one bucket (at most 1.46 units at 250 ms), or one slot with PCHIP (2.8-3.9 units at 2000 ms), and the area closes under the first vertex. Before, the line started at the oldest sample and its first segment, up to 2.04 units wide, changed height every frame. Release binary on a synthetic counter tree after the window filled: 1,520 of 1,520 plot ticks started at a vertex of the previous tick with the same y, against 1,238 new first vertices in 1,920 for the binary before the change
- **Emission.** The knots are written as `M` and `L` commands in the panel's viewBox (section 7.3). When a slot is two or more units wide (at btop's 2000 ms default, for example), every sample is a knot and the PCHIP curve adds `floor(slot width) − 1` vertices at fixed fractions of each interval. Each coordinate is written as Rust's `{:.2}` writes it, byte for byte; whole hundredths go through the `itoa` crate, which cut the server's instructions per tick by 39% at 250 ms (synthetic tree, release binary), and a value within 1e-6 of a rounding tie goes through `{:.2}` itself (test `vertices_print_as_two_decimal_format_does`). The area is the line closed to the baseline. Lines are 1.5 px, 2 px on the network and GPU plots, with `vector-effect: non-scaling-stroke` and round caps and joins; the area has opacity 0.15.
- **Absence.** A sample the collector could not read is skipped, never drawn as zero. A history with fewer than two samples, a non-finite value or a failed evaluation draws no path.
- **Tests.** `chart.rs`: `knots_land_on_the_sampled_values`, `a_monotone_series_stays_monotone`, `no_point_exceeds_the_sample_min_and_max`, `a_partial_history_is_right_aligned`, `a_spike_keeps_its_plotted_height_in_every_frame`, `completed_knots_keep_their_value_and_move_left_one_step_per_tick`, `a_full_window_keeps_every_vertex_but_the_newest` (CPU, network and GPU sizes at 250 and 2000 ms).

### 7.2 Interpolation decision and measurement rule

The choice was made on measured cost, with a preference for PCHIP when the costs are similar.

| Scheme | Median rebuild at 120 knots | p95 | Overshoot points | Global solve |
|---|---:|---:|---:|---|
| PCHIP (Fritsch-Carlson) | 0.0995 ms | 0.1756 ms | 0 | No |
| Natural cubic | 0.1045 ms | 0.1965 ms | 13 | Yes, tridiagonal |

- Measurement: Node v26.12.1, `tmp/workflows/btop-web-design/bench_pchip.js`, 120 knots, 8 subdivisions per interval, 100 warm-up rebuilds then 2,000 measured rebuilds; median and p95 of the path-string build only.
- Result: natural cubic is 5% slower, not faster. It also overshoots. PCHIP stays.
- Rule: a natural cubic or bicubic scheme may replace PCHIP only if its **median rebuild at 120 knots is at least 20% faster** on the target browser **and** it passes the no-overshoot and exact-knot checks. If the times are similar (within 20%), PCHIP wins. Bicubic applies only to a 2D surface, which this dashboard does not draw; the 1D alternative is natural cubic. Re-run the measurement before any replacement and record the numbers here.
- Acceptance criterion: `ACC-GRAPH-17-PCHIP-SMOOTH`.

### 7.3 Chart specifications

Every plot holds the samples of the 240 s window, 960 at the 250 ms default, and draws one knot per bucket of samples (section 7.1). The viewBox is the plot's size at 1440x900 in `chart.rs`; `preserveAspectRatio="none"` stretches it to the panel at other sizes.

| Mark | Series | viewBox | Stroke | Area | Scale and marks |
|---|---|---|---:|---:|---|
| CPU total (`cpu-stat`) | btop's total CPU percent | 467 x 28 | 1.5 px | 0.15 | 0-100 %; the quarter grid |
| Memory used (`mem-stat`) | btop's used RAM percent | 467 x 28 | 1.5 px | 0.15 | 0-100 %; the quarter grid |
| Network download and upload (`net`) | btop's rate per direction, one lane each | 391 x 77 each | 2 px | 0.15 | 0 to the shown ceiling per direction: a decade of bytes per second, rising at once and lowered only by a page load, never below the decade of `net_floor` (1 MiB/s for 1 Mbit/s), shared under `net_sync`, moving to a new ceiling over 1 s or more (section 7.4); the quarter grid; the scale in a 54 px gutter left of each box, `0 B/s` at the floor, half the shown ceiling on the middle line and the ceiling at the top corner, each with its unit (`DEF-GRAPH-102-NET-SCALE`); the two lanes sit 10 px apart with one 1 px `--border` rule at 55 % over the surface centred in the gap, each box inset 3 px, so no grid line or stroke lies on the boundary (`DEF-GRAPH-61-NET-LANES`, `ACC-GRAPH-99-LANE-BORDER`, `DEF-GRAPH-63-GRID-UNITS`); one shared axis, labels `240 s` to `now` below the plots |
| GPU load (`gpu`) | btop's utilisation percent per device, one lane per GPU with its own plot (`ACC-GRAPH-43-GPU-LOAD`, `DEF-GRAPH-51-GPU-LANES`) | 334 x 135 split into lanes with 4 px gaps: 135, 65, 42, 30 and 13 high for 1, 2, 3, 4 and 8 GPUs (`chart::gpu_lane`); each pair of adjacent shown lanes has one full-width 1 px `--border` rule at 55 % over the surface, centred in the 4 px gap (n-1 rules for n shown lanes; a hidden lane's own rule is hidden with it), and each plot box, grid included, is inset 3 px top and bottom (1 px under 25 px), so no stroke or grid line lies on a lane edge (`DEF-GRAPH-54-LANES-MERGED`, `ACC-GRAPH-78-GPU-LANES`, `ACC-GRAPH-99-LANE-BORDER`) | 2 px | 0.15 | 0-100 %; the quarter grid; `100%` at the plot's top corner, `75%`, `50%` and `25%` on the three painted lines and `0%` at the floor, right-aligned in the 27 px scale gutter left of the box (`DEF-GRAPH-63-GRID-UNITS`); the network plot's axis labels; device `i` uses `--chart-gpu-{i mod 3}` |
| Gauge arc (`cpu-gauge`, `mem-gauge`) | the value's 1 s EMA (section 6.2); the number in the hollow is the raw value | 108 x 114, radius 40, band 8 | - | - | 270° sweep over 0-100 %, ticks every 10 %, labels 0/50/100 (`ACC-GRAPH-42-GAUGE-TICKS`) |
| Process count (`proc-stat`, `ACC-VIEW-53-PROC-TILE`) | the number of process rows each tick | 30 x 28 | 1.5 px | 0.15 | 0 to the smallest 1, 2 or 5 x 10^k at or above both 10 and the largest drawn sample (133 → 200), moving to a new ceiling over 1 s or more (section 7.4); no scale label; the quarter grid; `--chart-proc`, `process_mid` clamped to 3:1 |
| Core row (`cores`) | each core's 1 s EMA (section 8.8) | 3 px meter | - | - | 0-100 %; the meter shows btop's cpu gradient up to the value; the integer and `%` are printed right of the meter, the integer in the gradient colour at its value |
| Process row graph (`proc`, `ACC-VIEW-52-PROC-GRAPH`) | btop's cpu_p per process: 5 for 0.1 to 5 %, otherwise rounded | 10 x 12, drawn at 36 x 12 px | 1.5 px | - | 0-100 %; a 20 s window (80 samples at 250 ms) with about ten knots; no grid; the Cpu% cell's colour; section 8.9 |
| Meter (`storage`) | used and free percent per mount | track and fill | - | value width | the percent and the size are printed beside it |

Rules:

- No chart border, no axis line and no point markers.
- The quarter grid (`ACC-GRAPH-68-PLOT-GRID`): every `[data-chart]` box except the process-row `proc` graphs paints six 1 px `--grid-line` lines as its CSS background, at 25, 50 and 75 % across and down. Across, they are the 60 s steps of the 240 s window. No box edge gets a line. The grid lies under the svg, carries no text and belongs to no series group, so a scale change never moves it. `--grid-line` is 1.16-1.38:1 against the surface (section 3.4). The SVG grid paths and `chart::x_grid()` are gone, which saves about 75 bytes per plot per frame.
- The scale gutter (`DEF-GRAPH-63-GRID-UNITS`): each GPU lane and each network lane prints its scale left of the `[data-chart]` box, never over it. Labels right-align on the fixed 27 px `--scale-w` gutter with tabular figures, so a changing ceiling moves no plot and no label beside it (`ACC-LAYOUT-74-RESERVED-WIDTH`). GPU lanes show `100%` at the top corner, `75%`, `50%` and `25%` on the three painted lines and `0%` at the floor; network lanes show the shown ceiling at the top corner, its half on the middle line and `0B` at the floor, through btop's short humanizer. The middle labels drop at drop-p3 (section 9.3); on a GPU lane `75%` and `25%` drop under 64 px, where their boxes would overlap the corner labels (`DEF-LAYOUT-104-WIDE-TEXT`), `50%` under 44 px, and the zero under 25 px; the whole gutter drops at drop-p2, where the plot keeps its width. A network lane with no ceiling renders an empty gutter, never the word `unavailable` over the plot.
- No text inside a plot box (`ACC-GRAPH-24-UNOBSCURED`): axis labels are 9 px `--muted` below the plot, and the network and GPU legends sit beside the plot, each entry marked by a swatch or an edge tab that matches its line (section 8.10).
- A new fragment replaces the path. Between readouts the animation clock slides the drawn series one slot (section 6); a scale change of a network plot or the process count plot moves it to the new ceiling (section 7.4).
- The y scale is fixed for CPU, memory and GPU (0-100 %) and follows the shown ceiling for the network (section 7.4). It does not rescale when a GPU is hidden. The process count ceiling changes only when the largest drawn count crosses a 1-2-5 step, and moves there as the network ceiling does.
- A chart that cannot be read is not drawn as zero: a series without samples has no path, and a panel without data states that it is unavailable or collapses (section 9.2).

### 7.4 Network auto scale

The collector keeps btop's auto scale in `graph_max` with this project's parameters: five consecutive ticks above the ceiling raise it and `net_fall` seconds below a tenth of it lower it. The plots show a decade axis instead (`ACC-GRAPH-164-NET-SCALE`): a sample above the ceiling raises it at once, only a page load lowers it, the shown ceiling never goes below the decade of `net_floor`, and a new ceiling is reached over several frames. `net_floor` (Mbit/s) and `net_fall` (seconds) are server options on the net page of the config menu (section 8.14), defaulting to 1 and 300; `net_fall` applies to `graph_max` only. The collector is `crates/btop-web/src/collector/net.rs`; the motion is `chart::Scale` in `chart.rs`, `.chart .rescale` in `app.css` and `syncRescale()` in `app.js`.

- **btop's rule** (`Net::collect`: `graph_max`, `max_count`, `rescale`). With `net_auto`, each direction counts ticks above its ceiling and ticks below a tenth of it. Five consecutive ticks above the ceiling set it to the mean of those five samples times 1.3; consecutive ticks below a tenth of it, once they fill the fall window, set it to the mean of the last five samples times 3.0, never below 10 KiB. Any other tick clears both counts, so each rule needs an unbroken spell. `net_sync` shares one ceiling between the directions. With `net_auto` off, the ceilings are the fixed `net_download` and `net_upload`, and no change runs
- **Rise window** (`ACC-GRAPH-96-NET-AUTOSCALE`). The rise keeps btop's five-tick count at any interval: 10 s at btop's 2000 ms and 1.25 s at 250 ms. `DEF-GRAPH-39-NET-JUMP`'s counted 10 s span is replaced, because the criterion states five consecutive ticks
- **Fall window** (`ACC-GRAPH-96-NET-AUTOSCALE`). The fall of `graph_max` is `net_fall` seconds (300 by default) of consecutive ticks below a tenth of it, where btop falls after five ticks, 10 s at its interval. At 250 ms the default window is 1200 ticks
- **Decade axis** (`ACC-GRAPH-164-NET-SCALE`, `DEF-GRAPH-102-NET-SCALE`). The target ceiling is 1, 10 or 100 of the binary unit the legend prints, the lowest of them at or above the measured peak (`net_stat::top`, the legend's `top`): a 2.72 MiB/s peak shows 10 MiB/s and a 17 MB/s peak shows 100 MiB/s. A sample above the ceiling raises it at once. The ceiling then stays through any drop, because a scale that shrinks hides the traffic level (the owner's words of 2026-10-29: "next scale is always 10x more and stays"). Only a page load lowers it: `net::reset_scale` drops the latch and the next sample sets the ceiling to the decade above the peak. `net_stat::top` only rises while the server runs, so after a burst a reload shows the same ceiling; a reload shows a lower one only after `net_floor` was reduced
- **Floor** (`ACC-GRAPH-96-NET-AUTOSCALE`). With `net_auto`, the target ceiling is never below the decade of `net_floor` Mbit/s (1 by default, 125000 bytes per second, so 1 MiB/s), so idle traffic draws low in the box and a burst stands out. btop's `graph_max` keeps its own rule and its 10 KiB floor, and the shown axis does not read it. With `net_auto` off the shown ceiling is the fixed one, whatever `net_floor` says
- **Smooth change** (`ACC-GRAPH-67-SMOOTH-SCALE`). Each direction's target ceiling feeds a `chart::Scale`, which gives the shown ceiling. A change moves the shown ceiling geometrically, `S = from * (to / from)^p` with `p` from 0 to 1, over `T = max(1000, 400 * |ln(to / from)|)` ms: 1000 ms up to a ratio of 12.2, 1843 ms for a ratio of 100. Geometric motion moves every vertex the same distance in each frame, at most about 3.2 px per 60 Hz frame on a 77.5 px plot for any ratio
- **Mechanism.** During a change the server draws the paths at the larger of the two ceilings and wraps area and line in `<g class="series rescale">` with `--k0`, `--q`, `data-rescale` (change id and elapsed ms) and `animation: rescale T ms`. `app.css` animates the registered property `--p` from 0 to 1 and applies `scaleY(k0 * q^p)` about the plot floor. Each SSE frame replaces the group, so `syncRescale()` sets the new group's animation time to the time since the change began on this page; a page that opens during a change starts at the server's elapsed time. The server keeps the group one frame past `T`, then sends the plain path. A new ceiling during a change starts from the shown one; a new interface selection or a page load starts at its own ceiling with no change. No crate animates a ceiling across server-rendered frames, so `Scale` is own code
- **Label.** `scale ▼ … ▲ …` in the head prints the shown ceiling of each SSE frame, so at 250 ms it steps through 4-5 log-spaced values, for example `1 MiB/s`, `1.77 MiB/s`, `3.16 MiB/s`, `5.62 MiB/s`, `10 MiB/s`. At 1000 ms or slower the lines still move over 60 or more animation frames, but the label changes once per SSE frame
- **Edges.** The transform scales y about the floor and leaves x alone, so the newest samples stay at the right edge. During a change, values above the shown ceiling are clipped by the box, where the plain path draws them flat 1 unit inside the top edge. During a rise this lasts up to `T`; after a fall of the process count ceiling, the return of the plain path moves those values once by about 1 px
- **Interface** (`DEF-NET-42-LOOPBACK-PICK`, the owner's decision of 2026-10-27). Without a matching `net_iface`, the panel shows the interface with the largest download + upload total other than `lo`, a connected one first, and shows `lo` only when no other interface exists. btop's rule takes the largest total, which picks `lo` on a host whose proxy traffic runs over loopback and shows download equal to upload. `net_iface` still selects any interface, `lo` included
- **Other graphs.** CPU, memory, GPU and the process rows have a fixed 0-100 % scale, so nothing rescales there. The process count plot (`ACC-VIEW-53-PROC-TILE`) gets the same motion from a `chart::Scale` in the process collector's state and the same `<g class="series">` wrapper; its target is its 1-2-5 ceiling, with no floor and no label. `syncRescale()` handles every `[data-rescale]` inside a `[data-chart]` box
- **Measured** (workflow `btop-web-net-scale`, release binary, synthetic counter tree, headless Chromium at 1440x900, every animation frame for 500 s, 13 changes with ratios from 0.116 to 20.1). The largest move of a vertex was 3.14 px per frame, with no frame pair above 4 px; before the change a rescale moved a vertex up to 74.55 px in one frame. Each change lasted 1000-1233 ms, and no vertex moved outside a change. The label never read below the floor. Live in the container for 10 min: 2 smooth changes, against 14 one-frame jumps for the binary before the change. Server CPU per tick was 0.8-1.0 % lower, inside the spread of the measurement; Chromium's main thread was busy 77 ms per second during a change against 54 ms at rest
- **Tests.** `chart.rs`: `a_scale_starts_idle_at_its_first_ceiling`, `a_scale_change_moves_geometrically_over_the_duration`, `a_large_ratio_takes_longer`, `a_second_change_starts_from_the_shown_ceiling`. `net.rs`: the five btop parity tests (`auto_scale_*` and `net_sync_*`) at `update_ms = 2000`, `auto_scale_at_250_ms_rises_after_five_ticks`, `the_fall_window_is_configured_in_seconds`, `the_configured_floor_sets_the_shown_ceiling`, `a_tick_between_the_thresholds_clears_both_counts`, `the_shown_axis_never_goes_below_the_floor`, `the_axis_rises_at_once_holds_after_a_drop_and_a_page_load_resets_it`, `the_pick_skips_lo_unless_it_is_the_only_interface_or_configured`. `tests/http.rs`: `network_plots_draw_in_a_series_group`, `network_scale_rows_persist_through_the_config_path`

## 8. Components

Each component lists anatomy, states, tokens and accessibility, as the shipped page implements them.

### 8.1 Shell: the top bar

The shipped page has one top bar and no status strip (`ACC-VIEW-41-TOP-BAR`, `templates/index.html`).

- Anatomy, left to right: the wordmark `btop-web`, the version (`1.0.2`), the host name with its full text in `title`, and the sampling interval (`250 ms` at the default, from `data-update-ms`; every cpu frame carries it out-of-band, so an interval set in the config menu reaches every viewer's bar). At the right end: the clock (`HH:MM`), the live chip with its state word and the sample age (`LIVE 0.3 s`), the READ-ONLY chip while the guard is on, and the CONFIG chip, which opens the config popover (`aria-expanded`, `aria-controls="config"`).
- Size: one row of `--bar-h`, 26 px at 1440x900 and 22-26 px along E (section 9.3); `--surface` background and a 1 px bottom border. Every indicator is a house chip (`ACC-VIEW-18-HOUSE-CHIP`). From the drop-p4 tier down the host and clock drop and the tier chip shows before the live chip; the version, interval, live chip, READ-ONLY and CONFIG stay (`ACC-LAYOUT-62-VERSION`). At drop-p2 the bar's gap is 3 px, the right group's gap 2 px, its side padding 4 px and each chip's side padding 3 px, so `0.6.100` fits at 360 px in every live state with 6.2 px to spare and `0.6.1000` by 0.5 px; nothing drops (`DEF-LAYOUT-58-VERSION-WIDTH`).
- Live states: `live` (border and lamp `--ok`); `paused` (`--warn`), set by pause in the config popover, where frames still arrive and are held, and resume applies the newest frame of each event; `stale` (`--warn`) when no cpu frame has arrived for 2 x the interval, 500 ms at the 250 ms default. STALE outranks PAUSED, because a paused page still receives frames.
- Sample age: the gap between the last two cpu frames in seconds with one decimal. The clock and the age are repainted when a cpu frame arrives. When the STALE timeout fires, it writes the clock time of the last frame into the age (`since HH:MM:SS`); the next cpu frame replaces it. The chip takes only its content's width and its auto margin absorbs a state change or a ticking age, so no blank band sits inside it and no other chip moves (`DEF-VIEW-55-CHIP-BLANK`, `ACC-LAYOUT-74-RESERVED-WIDTH`).
- CONTAINER chip: inside a container, a house chip with the word `CONTAINER` sits before the live chip, in the READ-ONLY chip's `--warn` tone (`.chip-container`). Its `title` names the runtime, the detection signal, the cgroup path and the metrics that are unavailable. It is static markup set at startup and drops at drop-p2 (P2, section 16.3)
- Live region: only the state word is `aria-live="polite"`, and it is written only when the state changes. The age is `aria-hidden`. The metric panels are not live regions; values that change every 250 ms would flood a screen reader.
- Tab title: the page `<title>` reads `btop-web`, or the text of `--title TEXT` (`ACC-VIEW-55-TAB-TITLE`, `ACC-CONF-63-TITLE-FLAG`). Askama escapes it, so `lab <b>1</b>` shows as that text and adds no element. A missing, empty or blank value stops startup with `btop-web: --title requires a non-empty text` and exit status 2

### 8.2 Panel

- Anatomy: a head (`.ph`) with the `<h2>` title and optional meta at its right end, then a body. The three stat panels add a foot; its first item takes the free width, so no foot item moves (`DEF-VIEW-45-STAT-FOOT`)
- Semantics: `<section>` with `aria-labelledby` pointing at its `<h2>`
- Style: `--surface`, a 1 px border in the panel's box token (`--border-cpu` on cpu-stat, cpu-gauge and cores; `--border-mem` on mem-stat, mem-gauge and storage; `--border-net` on net; `--border-proc` on proc-stat, gpu and proc; in Default, cores, gpu and storage read `--border-cores`, `--border-gpu` and `--border-storage`, section 3.3), 2 px radius, no shadow. The head is `--head-h` high (26 px at 1440x900, 22-26 px along E, section 9.3) with 8 px side padding, a 12 px semibold `--text-strong` title and a 1 px `--border-subtle` rule below it
- States: default; unavailable (the body says so in words, section 8.11); collapsed (gpu with no device, storage with no mount, section 9.2)

### 8.3 READ-ONLY chip

- Anatomy: a house chip in the top bar with a 4 px square `--warn` lamp and the word `READ-ONLY`
- Style: 18 px high, 5 px side padding (4 px at drop-p2, as every chip), 1 px `--warn` border, 2 px radius, 10 px uppercase with `0.03em` spacing, `--chip-fg` text on a 12 % `--warn` tint (`ACC-VIEW-18-HOUSE-CHIP`)
- `aria-describedby="readonly-reason"` points at a visually hidden paragraph: "The server refuses process signals, priority changes and every mutating request."
- States: shown while the guard is on, which `--readonly` sets; absent by default. With the guard on, the config menu's server options render as text and the page has no mutating control, so nothing is disabled; with the guard off they are forms (section 8.14)

### 8.4 Theme picker and mode chips

- Both sit on the General page of the CONFIG popover (`ACC-VIEW-19-CONFIG-MENU`, section 8.14). Theme: a native `<select id="theme-picker">` styled as an 18 px chip, with eight options in the order of the `theme` numbers: Default, Everforest Medium, Flexoki, Gruvbox, Solarized, Synthwave, Terracotta, Ultraviolet. Mode: three chips, `dark`, `light` and `auto`, in `data-group="mode"`; the chosen chip has the `on` class, an `--accent` border and a 12 % `--accent` tint, and the server renders `on` on `auto`
- Persistence: `btop-web-theme` and `btop-web-mode` in `localStorage`, every read and write inside `try/catch`. `theme.js` runs in `<head>` and sets `data-theme` and `data-mode` on `<html>` before the body paints. An unknown stored theme falls back to the picker's first option and rewrites the key
- Mode (`ACC-THEME-69-MODE-AUTO`): `btop-web-mode` stores the choice, `dark`, `light` or `auto`. No stored value, `auto` or an unknown value gives auto, so a fresh viewer gets auto. Auto sets `data-mode` from `prefers-color-scheme` and follows a `matchMedia` change at once, without a reload. Dark and light pin the mode and ignore the system; a stored `dark` or `light` from before auto existed keeps working. `data-mode` takes only `dark` and `light`; without JavaScript the `data-mode="dark"` on `<html>` applies
- Storage blocked (private window) leaves the page on the system mode and the configured theme; it does not break
- The open theme list is native: `color-scheme` follows `data-mode` (`:root[data-mode="dark"]` is `dark`, `light` is `light`), so the operating system's scheme no longer picks the list's colours, and each `option` paints `--text` on `--surface` (`DEF-THEME-38-SELECT-POPUP`). Before, a dark page under a light OS scheme drew `#cccccc` names on a white list (1.6:1). The highlighted option is the browser's `SelectedItem` pair for the page mode, 6.4:1 in dark and 5.6:1 in light in Chrome

### 8.5 Pause and live state

- Pause is a chip on the General page of the CONFIG popover (`#pause-btn`, `aria-pressed`); its label reads `pause`, then `resume`. It is not stored, so a reload starts live
- While paused, frames still arrive and each target keeps only its newest one; the live chip reads `paused` in `--warn`. Resume applies the held frames. The live states and the sample age are in section 8.1
- No keyboard shortcut is bound: `p`, `l` and `/` do nothing

### 8.6 Meter

- Anatomy: a `<span class="meter">`, a `--meter-track` track with an `::after` fill as wide as `--value` percent in `--meter-color`. Square ends, no radius
- Uses: the storage rows (6 px, `--chart-mem-used` for used and `--chart-mem-free` for free) and the core rows (3 px, the cpu gradient, section 8.8)
- The value is printed beside every meter; the meter itself has no role and no ARIA attributes
- Unavailable: `data-state="unavailable"` draws a hatched track with no fill, and the printed value is left empty

### 8.7 Plots

- A plot is an `<svg class="chart">` inside a `[data-chart]` box. The box is `role="img"` with an `aria-label` that names the series ("CPU history, 0 to 100 percent"); the svg is `aria-hidden`
- The viewBox is the plot's size at 1440x900 (section 7.3) with `preserveAspectRatio="none"`. Each series is a line path (1.5 px, 2 px on the network and GPU plots, `vector-effect: non-scaling-stroke`) and an area path at 0.15 opacity. The box paints the quarter grid under the svg: 1 px `--grid-line` lines at 25, 50 and 75 % across and down (section 7.3)
- No text inside the box, no tooltip, no point markers (`ACC-GRAPH-24-UNOBSCURED`); the scale sits in a gutter left of the box (section 7.3); no animation except the animation clock's slide of the drawn series and a scale change of a network plot or the Processes count graph (sections 6 and 7.4)
- States: a history with fewer than two samples draws no svg; an unavailable series draws no path
- The process row graph is a line-only `<svg data-chart="proc">`, `aria-hidden`, before the Cpu% number in its cell; the number carries the value (section 8.9)

### 8.8 Cores

- Anatomy: one `.core-row` per core, up to 64, in btop's column-major order: the label `C` and the core number with at least two digits (`C00` to `C63`; a cpuset CPU 12 reads `C12`, CPU 100 `C100`), a 3 px meter, and the number with its `%` right of it, in 8 px type (`ACC-VIEW-48-CORE-BARS`, `ACC-VIEW-54-CORE-LABEL`). `core_rows()` allows at most 16 rows, so 64 cores fill 4 columns with C00-C15 in the first (`ACC-VIEW-163-CORE-METER`)
- Size: the label and the number (reserved at its widest form, `data-w="100"`) keep their width; the meter is the only part that shrinks. Rows have a 2 px gap and columns a 6 px gap. In the sweep a 920 px wide viewport gives an 84.3 px row with a 41.7 px meter and 1024x768 a 94.9 px row with a 52.3 px meter; a host with GPU, cores and storage all shown at 1101x641 gives a 103 px row and a 60.3 px meter, and a stored `cores:s` size at 1440x900 still leaves 71.1 px. Only the horizontal axis clips a row, because the 8 px glyph box is 9 px tall and passes a 9.3 px row by a fraction of a pixel
- Values: the row carries the core's 1 s EMA as `--value` (one decimal), which sets the meter and the colour, and the number is that value as an integer (`CoreUsage.smoothed_pct`, the same `chart::smooth` step as the plots), not btop's instant percent (section 12.4). Live, the number moved 0.86 per core per frame against 4.0 for the raw values
- Colour, as btop's `Cpu::draw` (`btop_draw.cpp:815-822`) and `Meter` (367-383) (`ACC-VIEW-56-CORE-COLOUR`): the meter shows `linear-gradient(in srgb-linear to right, --ok, --warn, --crit)` across the whole track, clipped at the value, so each position has the gradient colour at that position; the number takes `color-mix(in srgb-linear, …)` of the same stops at `--value` (start to mid over 0-50, mid to end over 50-100, as btop's `generateGradients`), and `%` takes `--text`. `--ok`, `--warn` and `--crit` are `cpu_start`, `cpu_mid` and `cpu_end` clamped to at least 4.55:1 on `--surface`; the 0.05 above the text floor covers the browser's 8-bit rounding of a mix, which cost up to 0.024 at the bare 4.5:1 floor (Gruvbox light). Every value 0-100 reaches 4.5:1 in all sixteen variants; among the tabulated ten, the lowest is 4.54:1 (Everforest Medium light). The colour moves only with the smoothed value; the page sends no per-row colour
- Each row carries the core's name in `title`. An unavailable core shows a hatched meter, no number and no `%`
- Responsive: the labels and numbers show wherever the panel shows; the panel drops at drop-p3 (section 9.4, `DEF-VIEW-37-CORE-LABEL`)

### 8.9 Process table

- Anatomy: a native `<table>` with a visually hidden caption ("Processes") and `scope="col"` headers PID, PROGRAM, COMMAND, THREADS, USER, MEM and CPU%; each header's text is a `<button>` that sorts. Pid and Command use the mono stack. The head is sticky; even rows take `--zebra` and the hovered row `--row-hover`
- Cpu% is coloured by value: `--crit` at 66 % and above, `--warn` from 33 %, `--ok` above 0 and `--muted` at 0. The number is always printed
- Rows from the server (`ACC-VIEW-58-PROC-VIEW`): one proc fragment serves every viewer, so it carries no viewer's view, collapse set or sort. The server sends every row in btop's tree pre-order (`_tree_gen`, `btop_shared.cpp:41-192`), whatever `proc_tree` says: a pid whose parent is not listed is a root, the roots come first and each child follows its parent; siblings keep btop's `proc_sorting` order, which the browser uses only to break ties. Each row carries `data-raw="depth cpu mem threads"` (btop's cpu_p, RSS in bytes and the thread count, `-` for an unavailable value). A row with children also carries `data-sum`, the Cpu%, MemB and Threads labels of its subtree sums and the Cpu% tone
- Aggregation: a subtree sum is the row's own cpu_p, memory and threads plus those of all its descendants, btop's collapsed-tree accumulation (`btop_shared.cpp:163-171`). The collector computes it once per tick over the listed rows, the same rows btop sums. An unavailable value adds nothing, and a sum is unavailable only when no row of the subtree has the value; it is never 0 in place of unavailable. An expanded row shows its own values; a collapsed row shows its own plus those of all its hidden descendants
- Views, applied by `app.js` after each frame:

| View | Rows shown | Values shown | Order |
|---|---|---|---|
| `top` (top-level only) | pid 1, and every row whose parent is pid 1 or is not listed | pid 1 its own; every other row with children is collapsed and shows its subtree sum | flat, by the sort column on the shown values |
| `all` | every row, flat | own | flat, by the sort column on own values |
| `tree` | every row whose ancestors are all expanded, indented under its parent | expanded rows their own, collapsed rows their subtree sum | tree order; the roots and each sibling group by the sort column on the shown values |

- Starting view (`ACC-CONF-60-VIEW-FLAG`): a viewer with no stored view starts in the view of `--proc-view top|all|tree`, else in `tree` when btop's `proc_tree` is True, else in `top`. The server renders it as `data-proc-view` on `#p-proc` (`config::initial_proc_view`); a stored view wins over the flag and the config. A value other than `top`, `all` or `tree`, or no value, stops startup with `btop-web: --proc-view requires top, all or tree` and exit status 2; `--help` lists the flag
- Controls: four rows on the Processes page of the CONFIG popover (section 8.14). `procs` with `top`, `all` and `tree`, `depth` with `1` to `9`, and `dir` with `desc` and `asc` are house chips in a `role="radiogroup"` with `role="radio"` and `aria-checked`; `sort` is a native `<select id="proc-sort">` of the seven columns. The `depth` row selects the display level N: the root and every process down to depth N are expanded, 3 by default, and a choice clears the per-row toggles. `sort` and `dir` write the same `sort` and `descending` the column headers write, a header click updates them, and a new column from the select starts as a header click does. The controls change browser storage and the DOM only and send no request, so the read-only guard does not apply to them
- Tree drawing (`ACC-VIEW-180-PROC-TREE`): in tree view each row's Program cell starts with a `<span class="tree">` of `<i class="tstep">` steps, one 12 px step (`--tree-step`) per ancestor level plus one for the row's own branch, drawn as 1 px borders in `--muted` from `::before` and `::after`, never as glyphs. A continuing ancestor (`tline`) and a middle sibling (`tb-mid`) run the whole row height, a last sibling (`tb-last`) runs from the row top to the elbow, a first root (`tb-first`) runs from the elbow to the row bottom, and every branch carries a 12 px arm at the row's centre; the rules meet between rows, so the columns line up at every depth and in either face. A row with children then has a `<button class="twisty">` one step wide and the row tall, with a filled 4 by 5 px triangle that points right while `aria-expanded="false"` and points down while `aria-expanded="true"`, labelled `children of NAME (pid N)`; a leaf has no button and reserves the step as margin, so its name starts where a sibling's does. A click, Enter or Space toggles that node only; a toggle holds until a `depth` chip is clicked. The Program column reserves `(level + 2) * 12 px + 17 px` - 149 px at level 9, under its 150 px floor - so no rule clips and a face switch moves nothing; a name that does not fit after the prefix ends in an ellipsis, and at drop-p3 the Program cell is held to the column (`.ptable td:has(> .tree) { max-width: 0 }`)
- Sort (`ACC-VIEW-59-PROC-SORT`): a click on a header sorts by its column, and a second click reverses the order. A new column starts largest first for Pid, Threads, MemB and Cpu%, and A to Z for Program, Command and User. A fresh viewer starts on MemB, largest first, whatever btop's `proc_sorting` says. Numbers sort by the raw values of `data-raw` and the sums `app.js` adds from them, never by the humanised text; text compares with one `Intl.Collator`; an unavailable value sorts last in both directions, and ties keep the server's order. The active header carries `aria-sort` and `▼` (descending) or `▲` (ascending) after its label in `--text`; the other headers are `--muted`. Composed on rows, zebra and hovered rows in the ten tabulated variants, the active header and the chevrons reach at least 4.98:1 and the tree's rules at least 4.51:1 on rows and zebra rows and 3.56:1 on hovered rows
- Filter: the search input in the proc head matches a case-insensitive substring of a row's Pid, Program, Command or User cell, the fields btop filters on, after a 150 ms debounce; `Escape` clears it; the head count reads shown/total. `-f TEXT` sets its first value. In tree view a match keeps its ancestors shown and opens them while the query is set; the stored collapse set returns when the query is cleared. In top view a match below the top level shows the top-level row that holds it. The input is a plain section of the panel bar at the `--filter-w` width (`ACC-VIEW-187-FILTER-SEGMENT`): full bar height, flush to the bar's right end, `--surface-raised` behind it, and one 1 px left rule in `--border-subtle`, the bar's rule colour, in every state (`DEF-VIEW-110-FILTER-RED`). The placeholder reads `filter` in `--muted` mixed with 20% `--text` (`ACC-VIEW-188-FILTER-HINT`), because plain `--muted` falls under 4.5:1 on that fill (Default dark `#9a9c9d`); a yellow block over the word is Chromium's find-in-page highlight on the placeholder text (active match `#ff9632`, passive `#ffff00`), not the page's paint, because the only placeholder rule sets the colour and the opacity and no app rule paints a block (`DEF-VIEW-62-FILTER-YELLOW`, established 2026-10-28)
- Per-viewer state: one `procView` object in `app.js`, stored as JSON under `btop-web-proc` in `localStorage`: `v` (1), `view` (only after the viewer picked one), `level` (1 to 9, default 3), `toggled` (pid to collapsed, from chevron clicks), `sort` and `descending`. Every read and write is inside `try/catch`; an unknown value falls back to its default. It is written only on a user action, so a viewer who never picked a view keeps the server's starting view. A second browser has no stored state and starts in the server's starting view. The Processes page (`ACC-CONF-61-CONFIG-PAGES`) reuses this object; its controls are found by `data-group` and `#proc-sort`
- Frames: `app.js` cancels htmx's swap of `#proc-table-wrap` and merges the frame's rows into the live table by pid. An unchanged row keeps its element, steps and chevron, and only changed attributes, graphs and labels are written; a new pid, or a reused pid with another program, takes the server's row, and a gone pid is removed. Only rows whose place changed are moved. A pressed header or chevron holds proc frames until it is released, so a frame cannot replace the control under the pointer, and focus on a header or a chevron stays across frames
- Rows: a `ResizeObserver` keeps whole rows at rest (section 9.2)
- Row graph (`ACC-VIEW-52-PROC-GRAPH`, btop's `proc_cpu_graphs`): a 36 x 12 px line of the last 20 s left of the Cpu% number and centred on its line, so the number sits at one height with or without a graph (`DEF-VIEW-47-CELL-SHIFT`), in the cell's colour, as btop draws the graph and the number in one colour. It starts when cpu_p is above 0, from the previous update's 0, so it draws in that update; it goes after 10 updates in a row below 0.1 % (2.5 s at 250 ms, 20 s at btop's 2000 ms) and with its pid. 0.1 to 5 % draws as 5. The Cpu% column is 106 px, and the graph stays at every viewport
- Collapsed row graph: btop graphs a collapsed row's summed cpu_p (`btop_draw.cpp:1796` runs after `_tree_gen`), so a collapsed top-level row (a root, or a child of pid 1) shows the graph of its subtree's cpu_p, kept by the server under the same start and drop rules, and its own graph when there is none. The server keeps no summed graph below the top level, so a collapsed row deeper in the tree shows no graph (`data-collapsed="deep"`); its number still shows the sum. Expanded rows and every row in `all` show their own graph
- Views cost against the build before them, live host at 250 ms, one headless Chromium viewer in tree view with every node expanded: server user-space instructions per tick -1.4 to -1.5 %, SSE bytes -6.1 %, Chrome renderer -10.7 to -11.0 points of one core (workflow `btop-web-proc-view`, verify round 3)
- Row graph cost, live host at 250 ms with one Chrome viewer: 3.46 graphs per tick on average (median 4, p90 6, max 9); SSE bytes +0.68 % per tick (+1.66 % on the largest tick); Chrome +0.25 pp of one core (htmx replay of the recorded frames, 12 blocks); server about 2 µs per graph per tick, +0.1 %. One graph costs the client about 0.054 pp, so the +0.5 pp limit holds up to about 9 graphs per tick
- Not shipped: row selection, keyboard row navigation, the detail drawer and the disabled signal actions of the earlier proof

### 8.9.1 Process popup

`ACC-VIEW-176-PROC-POPUP`. A click on a process row, or Enter on a focused row, opens a panel over the process table.

- **Place.** `app.js` sets the box to the table's rectangle up to the left edge of the CPU% column, so it hides rows and covers no plot (`ACC-GRAPH-24-UNOBSCURED`), and moves it with the panel. The box scrolls inside itself when the table is shorter than its body; the page never scrolls
- **Body.** `GET proc/PID` returns an HTML fragment: owner, state, threads, memory, CPU, io read, io write and GPU nodes, a CPU plot, a memory plot, one load plot per GPU the process can reach, and the whole command line with a copy chip. The page asks once per slow pass; a paused page holds the first body
- **Data.** `proc::detail` starts a watch, and the sampler's slow pass pushes the row's Cpu% and MemB into two histories over the page's chart window, so a plot draws from the second sample and fills from the right while the popup is open. No watch, no cost; a watch ends five passes after its last request; at most 16 are held
- **GPU.** A process uses a GPU when `<pid>/fd` holds a GPU node (`gpu.rs`, the holder rule). `/dev/nvidia<N>` reaches device N; `/dev/dxg`, `/dev/kfd` and a render node reach every device. The plot is the device's load from the GPU panel's own series, and its label says so: NVML names no per-process share on WSL2 or in a container
- **Closing.** Escape, the close chip and a click outside close it. Focus moves to the close chip on open, Tab cycles inside the box, and focus returns to the row, unless the click landed on another control, which keeps it
- **Scope.** A pid the frame does not list is a 404, so a request reaches only what the table shows. A client relays the selected agent's body

### 8.10 GPU legend and selection

- Legend: one entry per GPU beside its own plot, marked by a 3 px edge tab, the full height of the entry, in that GPU's line colour (`ACC-VIEW-82-GPU-TAB`), then `GPU i` and the model (ellipsised, full text in `title`), utilisation, VRAM `used / total`, and temperature · power (section 9.1, `ACC-GRAPH-43-GPU-LOAD`); utilisation and temperature reserve `100%` and `100°C`, so VRAM and power keep their place (`DEF-VIEW-46-LEGEND-SHIFT`). The tab reads the line's `--chart-gpu-*` token, at 3:1 or more on the panel
- Mini bars (`ACC-VIEW-79-GPU-BARS`): LOAD, MEM and PWR in a reserved 48 px column right of the values, in the core meters' 3 px track-and-fill style and gradient. A bar with no reading renders the hatched unavailable track and no fill, never a zero. The column is fixed, so a fill moves no value; it drops first, at drop-p2 and on a lane under 34 px
- Selection: the `gpu` row on the GPU page of the CONFIG popover has one chip per device (`role="checkbox"`, `aria-checked`, the model in `title`). A hidden GPU loses its plot and its legend entry (`data-selected="false"`); the shown plots share the height and keep 0-100 %, so nothing rescales. The last shown GPU cannot be hidden
- Persistence: `btop-web-gpu-selection` in `localStorage`, a JSON array of device indices, read and written inside `try/catch`. An absent or unreadable key, or one that selects no present device, shows all devices. The selection is re-applied after every gpu frame
- No device: the panel collapses (section 9.2)

### 8.11 Unavailable state

- A value the collector cannot read is drawn as the word `unavailable` in its own slot: the stat tile (`.stat-na`), the gauge hollow (the gauge draws its track and no fill), and the network or GPU legend value. A meter shows its hatched track
- A panel with no data at all states it in muted 11 px text: `GPU metrics unavailable`, `network unavailable`. The gpu panel then collapses
- A core number and a mount percentage are left empty beside the hatched meter, a process cell whose read failed is left empty, and a detail line such as frequency or temperature is not drawn
- Inside a container without a cgroup v2 directory, the withheld cores panel keeps its head with `unavailable` at the right end, has an empty body and narrows to its head, the CPU foot reads `load unavailable` and the `load per core` fact reads `unavailable`; inside any container the storage head reads `IO unavailable` (section 16.3)
- Never `0`, a dash or a fake fill. The shipped page states no reason beside the word; a withheld panel head carries its reason in `title`

### 8.12 Empty state

- A panel with nothing to show collapses: storage without a mount, gpu without a device. A band whose panels are all hidden gives its height to the others (section 9.2)
- A filter with no match leaves the table body empty, the head count at `0/N` and the input available
- The last shown panel cannot be hidden, so the page is never empty; `reset layout` in the CONFIG popover restores the default

### 8.13 Storage

- Anatomy: a panel of its own in band 3, one block per mount except swap: the mount path (ellipsised, full text in `title`), `IO n%` when read and the total, then a `used` meter row and a `free` meter row, each with the integer percent and the size. The free rows drop at 3 or more mounts
- Values use btop's humanizer; the percent is btop's integer percent. `IO` is btop's disk activity percent for the interval, not throughput
- States: normal; a mount without a percent shows a hatched meter and no percent; no mount collapses the panel; inside a container the head reads `IO unavailable` and no mount head draws `IO n%`

### 8.14 Config menu

The CONFIG chip opens a popover of seven pages (`ACC-CONF-61-CONFIG-PAGES`, `templates/index.html`): btop 1.3.0's options-menu categories `general`, `cpu`, `gpu`, `mem`, `net` and `proc` (`btop_menu.cpp:1417`), plus `layout`. A page lists only the options of its category that btop-web implements. A display option changes what one viewer sees and is always editable. A server option changes collection for every viewer and is editable only with the read-only guard off.

- Tabs: `.cfg-tabs` is a `role="tablist"` of seven `role="tab"` chips with btop's names, uppercase by `.opt`. Each tab controls one `role="tabpanel"` `#cfg-page-{page}`, and only the selected panel is shown. The server renders General selected; the selected page holds for the page view and is not stored
- Keys (WAI-ARIA tabs, automatic activation): CONFIG opens the popover and focuses the selected tab. The tablist is one `Tab` stop with a roving `tabindex`; Left and Right select the next page with wrap, Home and End the first and last. `Tab` enters the page's controls, and `Escape` closes the popover and returns focus to CONFIG. A read-only CPU, MEM or NET page has no control, so `Tab` from its tab leaves the popover
- Groups: `view · this browser` holds the display options, and `server · all viewers` holds the server options under one note line. CPU, MEM and NET have only the server group, LAYOUT only the view group
- Box: 320 px wide under the top bar at the right, `max-width: calc(100vw - 16px)`, `max-height: calc(100vh - var(--bar-h) - 14px)` and `overflow: auto`, so the page never scrolls and the popover scrolls inside itself when a page is taller. The tablist is sticky at the popover's top. Over 64 viewports from 360 px wide, 7 pages and both guard states, the tabs held one row and the page never scrolled; the popover scrolled inside itself at 844x390 and 667x375 with the guard off
- Colours: the text is `--text` on `--surface-raised`, and the selected tab and chips read `--chip-fg` on the 12 % `--accent` tint (section 3.4). Typed values are not uppercased. All popover text reached 4.5:1 composed on every page, in the ten tabulated variants, at 1440x900 and 390x844 and in both guard states; the lowest is 4.54:1 (selected tab, Everforest Medium light)

| Page | Display options (this browser) | Server options (all viewers) |
|---|---|---|
| general | `theme` select, `mode` dark / light / auto, `pause`, `motion` on / off | `theme`, `update_ms`, `window_s`, `show_battery`, `selected_battery` |
| cpu | none | `check_temp`, `cpu_sensor`, `show_coretemp`, `cpu_core_map`, `show_cpu_freq`, `custom_cpu_name`, `cpu_load` |
| gpu | `gpu`, one chip per device | `nvml_measure_pcie_speeds`, `custom_gpu_name0` to `custom_gpu_name5` |
| mem | none | `show_disks`, `show_swap`, `swap_disk`, `only_physical`, `use_fstab`, `zfs_hide_datasets`, `disk_free_priv`, `disks_filter`, `zfs_arc_cached` |
| net | none | `net_auto`, `net_floor`, `net_fall`, `net_sync`, `net_download`, `net_upload`, `net_iface` |
| proc | `procs` top / all / tree, `tree` collapsed / expanded, `sort` select, `dir` desc / asc | `proc_sorting`, `proc_reversed`, `proc_tree`, `proc_per_core`, `proc_filter_kernel`, `proc_refresh_ms` |
| layout | `panels`, shown / hidden and s / m / l per panel, `reset layout` | none |

- Not listed: `listen` and `proc_root`, btop-web keys that need a restart; `proc_filter`, which btop sets with `f` outside its options menu, so the filter input in the proc head stays the live control; `show_gpu_info` and `show_io_stat`, which are parsed and have no effect in btop-web; and every btop option btop-web does not implement

Display options apply in the browser at once and change nothing on the server.

- Storage: per viewer in `localStorage`: `btop-web-theme` and `btop-web-mode` (section 8.4), `btop-web-font` (`ACC-THEME-155-FONT-TOGGLE`), `btop-web-live-status` (`ACC-VIEW-200-LIVE-STATUS`), `btop-web-motion` (section 6), `btop-web-num-motion` (`ACC-VIEW-108-NUM-MOTION`), `btop-web-gpu-selection` (section 8.10), `btop-web-proc` (section 8.9), `btop-web-proc-cols` (`ACC-VIEW-140-COLUMN-MENU`) and `btop-web-layout` (section 9.2). Pause is not stored (section 8.5)
- They send no request, so the guard does not apply to them, and a second browser keeps its own values

Server options are btop's keys of `config.rs` that the collectors read.

- Registry: `config::SERVER_OPTIONS` holds the 41 options in page order and, within a page, in btop's menu order, each with its page, key, control and help. The control is `Bool`, `Int { min, max }`, `Chips`, `Text` or `Choice`; the help is btop's first description line, shown as the row's `title`. `Config::get` returns a value as btop writes it: `True`, `250`, `cpu lazy`
- Row (`partials/cfg_key.html`): btop's key in the mono stack, then the value. With the guard on, the value is text (`""` for an empty string), there is no form, and the note reads `read-only: restart without --readonly to change`. With the guard off, the row is a form: a bool is two submit chips `on` and `off` with `aria-pressed`, a multi-value option (the chart window, the CPU load scale, the process refresh period) a group of submit chips, an int a number input, a string a text input, `theme` a select of 0-9 and `proc_sorting` a select of btop's eight sort modes. The note reads `changes apply to every viewer and are saved to {path}` with the resolved btop-web.conf path, or, when neither XDG_CONFIG_HOME nor HOME resolves, `changes apply to every viewer until restart; no btop-web.conf path: set HOME or XDG_CONFIG_HOME`
- `theme` sets the first paint of a viewer with no stored theme; `proc_tree` sets the starting view of a viewer with no stored view when `--proc-view` is absent; `proc_sorting` orders siblings and ties under the viewer's sort (section 8.9)
- A form shows the values of the viewer's page load and refreshes on a reload or on its own POST; the effect on the panels reaches every viewer through the sampler
- Layering (`ACC-CONF-148-OWN-FILE`): startup applies the embedded defaults, then the `-c` btop.conf, then `$XDG_CONFIG_HOME/btop-web/btop-web.conf` (default `$HOME/.config/btop-web/btop-web.conf`; a relative XDG_CONFIG_HOME is ignored), then the environment. A key in both files takes the web value; a key only in the web file overrides the embedded default. btop.conf is the baseline and is never written

The write path, with the guard off:

```text
row form ──POST config──► readonly_guard ──► settings::update
key, value                403 when on        Origin authority == Host, else 403
                                             key in SERVER_OPTIONS, else 400
                                             lock the config file path
                                             Config::set on a copy, else 422
                                             config::save, else 500
                                             config.send_replace(copy)
                                             200 and the row at the new value
```

- Form: `hx-post="config"`, relative so the lab proxy prefix holds, with `hx-trigger="change, submit"`, `hx-swap="outerHTML"` and `novalidate`, so a value outside the input's range reaches the server and returns refused with its reason; without `novalidate` the browser holds it back with no message. The `htmx-config` meta in `<head>` makes htmx swap 422 and 500 answers too
- Origin: the authority of the `Origin` header must equal `Host`, compared ASCII case-insensitive; a missing, `null` or other origin is refused with 403 `cross-origin request refused`. jupyter-server-proxy keeps `Host`, so a POST through the lab proxy passes; a reverse proxy that rewrites `Host` would refuse every write
- Validation: `Config::set` refuses a value that holds a line break or a double quote, formats btop's line as btop's `Config::write` does (`btop_config.cpp:736-741`: bools `True` or `False`, ints bare, strings quoted; btop-web's `theme` bare), then runs the file's parser `Config::apply` and the range check `Config::validate`. A refusal answers 422 with the row at its unchanged value and `refused: {reason}`, for example `refused: update_ms must be in 100..=86400000, got 50`; the config and the file are unchanged. A key that is not a server option answers 400 `not a server option: {key}`
- File: `config::save` writes btop-web.conf, the web file above and never the `-c` btop.conf; it resolves a symlink to its target, so the link stays and its target changes. It replaces the last uncommented `key =` line, the one btop's loader ends with, keeping its indentation and line ending, or appends the line when there is none; every other byte is unchanged. It writes a `mkstemp` file in the same directory with the target's permissions, syncs it and renames it over the target. A failed write removes the temporary file, answers 500 `refused: could not write {path}: {error}; nothing changed` and leaves the sampler on the old config. A mutex around the path serialises writes, so two POSTs never lose each other's line. Success answers 200 with `saved to {path}`
- No path: a process started without XDG_CONFIG_HOME and HOME has no btop-web.conf, so a change applies until restart, nothing is written, and the row reads `applied until restart; no btop-web.conf path: set HOME or XDG_CONFIG_HOME`. btop's own `~/.config/btop/btop.conf` is never written
- Sampler: `AppState.config` is a `watch::Sender<Config>`. The one sampler checks the watch channel after each tick; a new config applies from the next tick, and a new `update_ms` starts a new interval whose first tick fires at once, so every client sees the new cadence within two ticks. A GPU sweep in flight finishes on the old config. Every cpu frame carries `#update-interval` out-of-band, so every viewer's top bar shows the new interval, and `app.js` reads `data-update-ms` from each cpu frame for the STALE timeout. A new page load renders from the live config
- Next start: `-u`, `--theme`, `--proc-view` and the `BTOP_WEB_*` variables still outrank the file, so a menu change to one of those keys survives a restart only without them. `net_iface` is btop's starting interface: a change applies when the shown interface disappears or at restart, as in btop. After an interval change the history plots hold samples of both intervals for one window

The guard decides whether the server options can change.

- `readonly_guard` in `routes/mod.rs` refuses every request other than GET, HEAD and OPTIONS with 403 `read-only mode: mutating requests are refused` before routing, so `POST config` never reaches its handler while the guard is on. `--readonly` turns the guard on; it is off by default
- With the guard off, every viewer who reaches the page can change the server options and the file; the Origin check refuses a form posted from another site, not a viewer. Keep `listen` on loopback or behind a proxy that restricts access
- Cost with the menu closed (`ACC-PERF-16-NO-OVERHEAD`, verify round 1 of workflow `btop-web-config-pages`): the cpu frame grows by 135 bytes, SSE bytes per tick by 0.14-0.24 % and server user-space instructions per tick by 0.04 %; the page makes no request after load beyond the event stream

### 8.15 Stat tooltips

- One `.tip` box in the page, `role="tooltip"`, `--text` on `--surface` (4.5:1 in every pair and mode): at most four short lines naming the value, its unit and the btop source, with detail the panel does not show (`ACC-VIEW-80-STAT-TIPS`). The server writes each stat's text into `data-tip` with `&#10;` line breaks; a process cell takes its column header's text, so a 300-row table repeats no attribute
- Coverage: the four top-bar chips, every stat tile and foot, the gauge readouts and facts, the memory bar and rows, the network head, scale and legends, the GPU head, axis note, legend and mini bars, every core row, every storage mount, the process head, count and numeric columns. No `[data-chart]` box carries one and hovering a plot shows nothing. An element that carries a tooltip carries no `title`, because the browser would draw its own box beside the house one (`DEF-VIEW-74`); the CONTAINER chip's detector, runtime and cgroup path sit in its own tip
- Delay and placement: the box opens 3 s after the pointer comes to rest on one stat, and the wait restarts only when the pointer moves to a different stat (`DEF-VIEW-64`); `app.js` places it 12 px from the pointer, flipped left or up before its far edge leaves the viewport, at the first of the pointer's corners and the stat's text edges, nudged in 6 px steps, clear of the stat's own line boxes and of every visible `[data-chart]` box with 1 px slack for live sub-pixel shifts (`ACC-VIEW-111`); where no near position clears a plot too, the numbers win over the plot, and the box stays inside the viewport at 390x844. A frame replaces the stat under a still pointer, and the box re-reads the text the server now sends; moving off every stat, pointerdown, pointerleave, blur, resize and a different stat under the pointer hide it

## 9. Layout and responsive behaviour

The shipped page follows `docs/design/concepts/05.06-network-paired.html`, the concept the owner chose on 2026-10-26, rendered in the shipped theme tokens (`ACC-DESIGN-33-RENDER`). This section is read from `crates/btop-web/templates/index.html`, `assets/app.css`, `assets/theme.js` and `assets/app.js`. The pixel values were measured in Chrome by verify round 2 of workflow `btop-web-implement05`, and for band 3 (the GPU load panel, cores and storage) by verify round 3 of workflow `btop-web-gpu-load`, on the live host (64 cores, three GPUs, no mounts under `use_fstab`) unless a row says fixture.

### 9.1 Default layout at 1440x900

```text
y=0   +------------------------------------------------------------------------------+
      | btop-web 0.1.0  host  250 ms     17:56 [LIVE 0.3 s] [READ-ONLY] [CONFIG]     |  top bar 26
y=34  +--------------------------+-------------------------+-------------------------+
      | cpu-stat                 | mem-stat                | proc-stat               |  band 1  116
      | 10.0 %  [plot]           | 98.4 GiB  [plot]        | 144 running · sleeping  |
      |                          |                         |  [count plot]           |
      | GHz · load               | used % · available %    | top · 80% cpu · 80% mem |
y=158 +--------------------------------+----------------+----------------------------+
      | net                            | cpu-gauge      | mem-gauge                  |  band 2  214
      | [down plot]    | ▼ legend      | (arc)  GHz     | (arc)   [bar]              |
      | [up plot]      | ▲ legend      | 10.0%  load    | 98.4    used, cache, free  |
      | 240 s ... now  |               |                | GiB     sum row, swap      |
y=380 +--------------------------------+------------------------+--------------------+
      | gpu                            | cores 64 in 6 columns  | storage            |  band 3  190
      | [3 load plots]       | legend  |                        |                    |
      | 240 s ... now        | per GPU |                        |                    |
y=578 +------------------------------------------------------------------------------+
      | proc   head: Processes  n/N  [filter]                                        |  band 4  314
y=892 +------------------------------------------------------------------------------+
```

On a host with no mounts, storage collapses and gpu and cores share its width, as on the host measured here. With mounts, band 3 splits 5:4:3, which is the 12-column grid's columns 1-5, 6-9 and 10-12.

| Panel (`data-panel`) | Band | Width weight | Width at 1440x900 | SSE event | Content |
|---|---|---:|---|---|---|
| `cpu-stat` | 1 | 4 | 469 px | cpu | CPU total in `--ok`/`--warn`/`--crit`, a 467x28 plot, frequency and load average |
| `mem-stat` | 1 | 4 | 469 px | mem | used bytes with `of` total in the head, a 467x28 plot, used percent and available percent |
| `proc-stat` | 1 | 4 | 469 px | proc | the Processes tile (`ACC-VIEW-53-PROC-TILE`): the process total, beside it a grid of `running`, `sleeping`, `waiting`, `zombie`, `threads` and, on a host with a GPU, `gpu` (two rows, filled column by column); a 30x28 count plot (one knot per 8 s); `top` (the process with the highest Cpu% and that value), `80% cpu` and `80% mem` (section 12.4) in the foot |
| `net` | 2 | 5 | 586 px | net | interface and the shown scale per direction with the window, for example `scale ▼ 1 MiB/s ▲ 1 MiB/s · 240 s`, in the head (section 7.4); the download plot above the upload plot, 390x78 each, the quarter grid and labels `240 s` to `now`; a legend per direction with the rate, `top` and `total`, and one link line, `link 10 Gb/s · full · up`, with `err` and `drop` chips only above zero (`ACC-VIEW-174-NET-STRIP`) |
| `cpu-gauge` | 2 | 3 | 353 px | cpu | CPU name and core count in the head; a 270° arc drawn from the 1 s EMA (section 6.2) with btop's raw value, `%` and `total` inside the hollow, ticks every 10 %, labels 0/50/100; beside the arc, four plain text rows built like the memory table, one row height, rule, type size and label colour, no chip: `load` (the 1, 5 and 15 minute load averages as a percent of the cores), `user`, `system` and `pressure`. Labels share a left edge, values a right edge, and the first row is level with the memory table's first row where the panels share a band. The load triple is one line where it fits, each figure in its own reserved width, and otherwise wraps under its label, right-aligned (`ACC-VIEW-49-TOTAL-LABEL`, `ACC-VIEW-178-CPU-FACTS`, `DEF-VIEW-117-CPU-CHIPS`) |
| `mem-gauge` | 2 | 4 | 469 px | mem | total in the head; an arc of two segments from zero, used and cache, each the 1 s EMA of its share of the total, with the used bytes and `used` inside the hollow and the rest the bare track (`ACC-VIEW-192-GAUGE-CONTAINER`); beside the arc, one bar of every row's share that adds up to the total, a row per segment with its share, `free + reclaimable` with its share, and swap in GiB (`DEF-VIEW-50-MEM-TERMS`, `DEF-VIEW-101-MEM-BREAKDOWN`, `DEF-VIEW-106-SWAP-UNITS`, `ACC-VIEW-195-MEM-SUM`). In a container the arc's two segments are the container's used and reclaimable, and the rows are used, reclaimable, other used, other reclaimable and free, the first two's tooltips stating they are this container's (`ACC-VIEW-194-MEM-WORDS`); under a `memory.max` limit the free row is its room under the limit |
| `gpu` | 3 | 5 | 786 px; 586 px when storage shows; collapsed on the fixture | gpu | `{n} devices` and `0–100% · 240 s` in the head (`ACC-GRAPH-43-GPU-LOAD`); one lane per GPU in 334x135, separated by a 1 px rule, each a 0-100 % plot on the quarter grid inset 3 px inside its lane, beside its entry in a 224 px legend, over one axis with the network plot's labels (`DEF-GRAPH-51-GPU-LANES`); each entry: a 3 px edge tab in its line colour, `GPU i` and the model (ellipsised, full text in `title`), utilisation and VRAM `used / total`, then temperature · power, with LOAD, MEM and PWR mini bars right of the values |
| `cores` | 3 | 4 | 630 px; 469 px when storage shows; 809 px on the fixture | cpu | `64 · 0–100%` in the head; one row per core, column-major in 4 columns of up to 16 rows: `C00`, a 3 px meter in the cpu gradient and the integer with `%`, both the core's 1 s EMA (section 8.8) |
| `storage` | 3 | 3 | collapsed; 353 px with mounts; 607 px on the fixture | mem | one block per mount with the mount, `IO n%` and total, then a used meter row and a free meter row; the free row drops at 3 or more mounts |
| `proc` | 4 | 12 | 1424 px | proc | a static head with `Processes`, the shown/total count and the filter; 7 columns PID, PROGRAM, COMMAND, THREADS, USER, MEM, CPU%; whole rows only |

Departures from 05, each with the criterion that requires it:

1. The top bar gains the version, the interval in effect and the STALE state; the interval, clock and live state leave the panels (`ACC-VIEW-41-TOP-BAR`)
2. `sample 2000 ms` leaves the cpu-gauge head, which carries the CPU name and the core count as btop's CPU box does (the `DEF-VIEW-7-CPU-NAME` test)
3. The gauge readout moves inside the hollow as SVG text in the `data-gauge` SVG, so it scales with the arc; ticks and 0/50/100 labels are added and `range 0-100%` goes (`ACC-GRAPH-42-GAUGE-TICKS`). The freed side holds the facts (cpu) and the rows (mem)
4. Swap moves into the mem rows. Storage shows mounts only and collapses when it has none (`ACC-LAYOUT-37-SPACE`, `ACC-VIEW-23-RELEVANT`)
5. The gpu panel collapses on a host with no GPU (`ACC-VIEW-23-RELEVANT`). Where 05 had per-device GPU cards, the panel draws a load plot per GPU, each beside its legend entry, in the network panel's anatomy, and it takes band 3's first five columns from cores (`ACC-GRAPH-43-GPU-LOAD`)
6. proc-stat shows the total with the state counts, threads and GPU processes beside it, a count plot and the top process and 80 percent counts in the foot (`ACC-VIEW-53-PROC-TILE`); the shown/total count stays in the proc head beside the filter it depends on, written `n/N` without spaces as btop writes its `selected/total` counter (`ACC-LAYOUT-37-SPACE`)
7. Bands use flex shares in place of fixed rows. The default equals 05 at 1440x900 within 3 px: weights 5:3:4 give net 586 px and cpu-gauge 353 px where 05's 12-column grid gives 589 and 350 (`ACC-LAYOUT-20-ADAPTIVE`, `ACC-VIEW-40-GADGETS`)
8. Plot scales follow btop: CPU, memory and GPU on 0-100 %, network on a decade of bytes per second, rising at once and lowered only by a page load, floored at `net_floor` (1 Mbit/s) (section 7.4); 05 zooms to the data range. Every plot spans the same 240 s window, so the network and GPU plots share their axis labels
9. Cores are btop's rows with a meter in place of 05's tinted cells (`ACC-VIEW-48-CORE-BARS`), gauge fills use the band tokens, and panel borders use btop's box tokens, or the muted Steel borders in Default (section 3.3)
10. Storage draws btop's used and free meter rows per mount, because a single bar left a 26 px empty band at 1440x900 (`ACC-LAYOUT-37-SPACE`)
11. Type is `system-ui` with `tabular-nums`; the mono stack is used only on Pid and Command
12. The legend arrows are btop's `▼` and `▲` in place of 05's `↓` and `↑`

05 shows some values twice, and the page keeps them: the CPU total in cpu-stat and cpu-gauge, memory used in mem-stat and mem-gauge, and load in both CPU panels. A gadget can hide either panel of a pair.

### 9.2 Bands, panels and gadgets

- **Bands.** Below the top bar the page is a flex column of four bands with `flex-basis: 0`. A band grows by its share times its band factor, 116 : 214 : 190 : 314 at 1440x900, and stops shrinking at a content minimum; the shares and minimums per tier are in section 9.3
- **Panels.** A panel grows by its width weight times its size factor and has `min-width: 0`. Heads, stat tiles, feet and the legend have fixed sizes and the plots take the rest, so a shorter band shrinks the plots first
- **Collapse.** CSS hides the gpu panel when its body holds the unavailable note and the storage panel when it holds no mount; the markup stays in the page. `syncBands()` in `app.js` hides a band whose panels all compute to `display: none`, which covers hidden gadgets, tier drops and collapsed panels in one place, and the other bands take its height
- **Gadgets** (`ACC-VIEW-40-GADGETS`). The Layout page of the CONFIG popover lists the ten panels from `PANELS` in `src/routes/dashboard.rs`. Each row has a shown/hidden chip and S, M and L chips. S, M and L multiply the width weight by 0.75, 1 and 1.5; a band's height factor is the largest factor among its shown panels. The last shown panel cannot be hidden. Right-click on a panel, or the ContextMenu key or Shift+F10 on a focused one, opens a menu of Off, S, M and L on the same state (`ACC-VIEW-75-PANEL-MENU`). `reset layout` removes the stored key and applies the default
- **Storage.** The key `btop-web-layout` sits beside `btop-web-theme` and `btop-web-mode` and holds `{"v":1,"off":[ids],"size":{id:"s"|"l"}}`. An absent key, a parse error or `v` other than 1 gives the default; unknown ids are ignored. `theme.js` applies it before first paint as `data-off`, `data-size` and `--bf-1` to `--bf-4` on `<html>`
- **GPU selection.** The `gpu` row on the GPU page of the CONFIG popover has one chip per device. A hidden GPU loses both its plot and its legend entry (`data-selected="false"` on its lane and entry); the shown plots share the height and keep 0-100 %, so nothing rescales. The selection is stored in `btop-web-gpu-selection`, is re-applied after every gpu frame, and the last shown GPU cannot be hidden
- **Order of rules.** A panel the viewport drops stays dropped whatever the gadget state
- **Server.** The server renders every panel and sends every fragment whatever the layout; there is no JSON API
- **Process rows.** A `ResizeObserver` on the proc panel sets a row height within half a row of `--row-h`, so the table shows whole rows at rest and the last row is never cut

Each `/events` connection opens with one `stamp` event, the build stamp the page's asset URLs carry, and no frame repeats it. The stamp hashes the crate version and the build-time sha256 of every asset and template, so a template-only build has a new one and a restart of the same binary keeps it. A page whose stamp differs reloads, once per stamp, and localStorage settings survive; the filter text and pause are not stored, so they start over. In client mode the stream states the client's own stamp and drops the agent's (`DEF-VIEW-119-STALE-TAB`).

One SSE event feeds panels in several bands through htmx out-of-band swaps:

| Event | Out-of-band targets |
|---|---|
| cpu | `#p-cpu-stat`, `#p-cpu-gauge`, `#p-cores`, `#update-interval` |
| mem | `#p-mem-stat`, `#p-mem-gauge`, `#p-storage` |
| net | `#p-net` |
| proc | `#p-proc-stat`, `#proc-table-wrap` |
| gpu | `#p-gpu` |

### 9.3 Tiers and fluid sizes

The page follows `docs/design/concepts/05.06-adaptive-1-priority.html`, the strategy the owner chose on 2026-10-26 (`ACC-LAYOUT-46-ADAPTIVE-PRIORITY`). As the viewport shrinks, bands and plots shrink first, down to content minimums; then type and padding step down along `clamp()` on E = min(100vw, 160vh); then fields drop by priority at named tiers (section 9.4). The values below were measured in Chrome by the build and verify round 1 of workflow `btop-web-adaptive-ship`, on the live host and on the fixture tree (no GPU, 2 mounts, 9 processes).

| Tier | Media query | Drops | Tier chip | Form |
|---|---|---|---|---|
| full | none of the below: width ≥ 1101 and height ≥ 641 | nothing | hidden; `P1-P4` while the GPU legend drops temperature and power | four bands, section 9.1 |
| drop-p4 | `(max-width: 1100px), (max-height: 640px)` | P4 | `P1-P3` | four bands; storage's width goes to GPU and cores by their weights 5:4 |
| drop-p3 | `(max-width: 899px), (max-height: 640px)` | P4, P3 | `P1-P2` | four bands; net 6 and the gauges 3 + 3, or net 8 and the gauges 2 + 2 under `(min-width: 1101px) and (max-height: 640px)`; GPU alone in band 3 |
| drop-p2 | `(max-width: 599px), (max-height: 540px)` | P4, P3, P2 | `P1` | one column under `(max-width: 599px) and (min-height: 541px)`; two columns under `(max-height: 540px)` |

- A viewport that matches a tier's query also matches the query of every tier above it, so the drops add up: drop-p2 also applies the drop-p4 and drop-p3 rules
- The tier rules replace the earlier `max-height: 860px`, `max-width: 1100px` and `max-width: 899px` blocks

| Token | Range, px | At 1440x900 | Sets |
|---|---|---:|---|
| `--bar-h` | 22-26 | 26 | top bar |
| `--head-h` | 22-26 | 26 | panel head |
| `--tile-h`, `--stat-size` | 30-42, 22-32 | 42, 32 | stat tile and its value |
| `--foot-h` | 15-18 | 18 | stat foot |
| `--row-h` | 19-22 | 22 | process row target |
| `--gap` (`--pad` follows it) | 5-8 | 8 | page padding, band and panel gaps |
| `--fs-12` | 11-12 | 12 | brand, panel titles, stat unit |
| `--fs-11` | 10-11 | 11 | process table, mount heads |
| `--fs-10` | 9-10 | 10 | top bar meta, head meta, CPU name, stat foot, legend name, VRAM, mount rows, table headers; the memory rows and the CPU facts take `--mem-fs`, this token less 1 px, then 8 percent less (`ACC-VIEW-189-GAUGE-ROOM`, `ACC-VIEW-197-MEM-FONT`) |
| `--fs-9` | 8-9 | 9 | x axes, legend detail |
| `--lg-v` | 12-14 | 14 | legend value |
| `--legend-w` | 112-168 | 168 | network legend |
| `--gpu-legend-w` | 192 or 224 | 224 | GPU legend, fixed 224 px; 192 at drop-p2 |
| `--filter-w` | 92-132 | 132 | process filter |

- Every token reaches its base value at E ≥ 1440, so the page at 1440x900 is the page of section 9.1. Against the build before the tiers, 0 element boxes, 0 styles and 0 pixels differ on the fixture tree
- The chips, the process filter (18 px, 10 px type), the 27 px plot scale gutter, the core rows (8 px type), the `unavailable` note and the GPU container-query sizes stay fixed
- `--row-h` is registered with `@property` as a `<length>`, so `fitProc()` in `app.js` reads it as a px number. Unregistered, it reads the `clamp()` text and the last process row is cut
- Gauge readouts are SVG text in the gauge's viewBox and scale with the arc

A band grows by its share times its band factor and stops shrinking at its minimum; below that the process table gives up rows. The shares are 116 : 214 : 190 : 314 at full, drop-p4 and drop-p3; 152 : 150 : 150 : 300 in the one-column form, where band 1 is a column with cpu-stat above mem-stat; and 76 : 150 : 150 for bands 1-3 in the two-column form, with band 4 as the right column, 1 : 1 with the left. The minimums, with name = 1.25 · `--fs-10`, value = 1.1 · `--lg-v` and meta = 1.25 · `--fs-9`:

| Tier | Band 1 | Band 2 | Band 3 | Band 4 |
|---|---|---|---|---|
| full, drop-p4 | head + tile + foot + 22 | the larger of the network legend, head + 17 + 2 · (name + value + 3 · meta + 4) + 12, and the CPU facts, head + 41 + 4 · (name + 1.25 · `--fs-12` + 1) + meta | head + 112 (11 core rows of 8 px type) | 120 |
| drop-p3 | head + tile + 16 | the larger of head + 17 + 2 · (name + value + 1) + 12 and head + 112 | head + 17 + 3 · (name + value + 1) + 16 | 120 |
| drop-p2, one column | 2 · (head + tile + 16) + gap | head + 17 + 2 · (name + value + 1) + 12 | head + 60 (one-line GPU legend) | 120 |
| drop-p2, two columns | as drop-p3 | as one column | as one column | the right column, full height |

Band 3 is also at least head + 25 + 18 px per shown GPU lane (`--gpu-lanes`, set by `app.js`), which binds from five GPUs (`DEF-GRAPH-51-GPU-LANES`).

The two-column form serves landscape phones and windows 540 px high or less. The four-band minimums sum to about 500 px at drop-p3, and the concept kept four bands and scrolled 98 px at 700x400.

- `.page` becomes a row. `<div class="tele">` wraps bands 1-3: it is `display: contents` at every other tier, so the other forms are untouched, and a flex column here. Band 4, the processes, is the right column
- A band in the left column keeps its gadget size factor. With every panel of bands 1-3 hidden the wrapper hides and the processes take the full width; with the processes hidden band 4 hides and the left column takes the full width. Gadget proc S and L have no effect in this form
- The CONFIG popover keeps `max-height: calc(100vh - var(--bar-h) - 14px)` and scrolls inside itself: 28-382 px of 390 at 844x390

The tier chip and the rules between tiers, gadgets and bands:

- `#tier-chip` sits in the top bar before the live chip as static markup, in no SSE fragment. It reads `P1-P3`, `P1-P2` or `P1`, and is hidden at full unless the GPU legend drops temperature and power, when it reads `P1-P4` (the last item of this list); its `title` names every dropped field: `P1-P3` names host, clock and storage, and from `P1-P2` down the titles name the Processes tile with its count graph and every fact. It is a house chip in `--muted` with `--chip-fg` text, 4.55:1 at the lowest (Everforest Medium light), where `--text` would reach 4.48:1
- Viewport drops outrank the gadget state: both set `display: none`, so a tier-dropped panel stays dropped whatever `data-off` says, and a gadget-hidden panel stays hidden at every tier
- `syncBands()` in `app.js` listens to the three tier queries, so a band whose panels all drop hides on a tier change, also while paused
- The Layout page of the CONFIG popover lists all ten panels at every tier. A row for a dropped panel edits the stored state, which applies once a tier shows that panel
- `.gauge-side` is `flex: 1 0 auto`, so the CPU facts and memory rows keep their text width and the arc gives up width. `#p-cpu-gauge .head-right` does not shrink, so the ellipsised CPU name shrinks before `64 cores`
- The memory table and the CPU facts share one row pitch, `--mem-row-h`: 20 px where the body holds it, else what the body leaves seven rows after the bar's 10 px and 8 px above and below the block. Both columns hang from the top of the centred seven-row block and end on the head text's right edge, so a memory table of fewer rows leaves its room below (`DEF-LAYOUT-121-MEM-TABLE`)
- The `@container gpu-lane` rules follow each GPU lane's height inside any tier (`DEF-GRAPH-51-GPU-LANES`): below 64 px the `75%` and `25%` scale labels drop; below 44 px the entry row gap is 0 and the value 13 px; below 37 px the name and meta lines take line height 1.1 and the value 1; below 34 px temperature · power drop; below 25 px each entry is one line. No media query matches the 34 px step: with three GPUs the lane is 42.33 px at 1440x900, which takes the 44 px step (13 px value, row gap 0) and keeps every field, and 34.36 px at 1024x768, which keeps every field; 25 px at 900x641 drops temperature, power and the mini bars; a stored gadget size moves it. `syncGpuDrop()` in `app.js` observes `#p-gpu` and adds `.gpu-drop` to the tier chip while the first shown lane's `.legend-meta` computes to `display: none`; the chip then reads `P1-P4` at full and `P1-P3` at drop-p4, and its `title` adds `GPU temperature and power` (`DEF-LAYOUT-36-GPU-DROP`). From drop-p3 down the tier titles already name them. With three GPUs and the default layout the smallest lane measured at full and drop-p4 is 25 px (900x641), so the 25 px step is not reached there

### 9.4 Shipped drop table

Priorities follow `ACC-LAYOUT-21-PRIORITY`. P1 is never dropped: the top bar's brand, version, interval, live chip, READ-ONLY and CONFIG; cpu-stat's value and plot; mem-stat's value, `of` total and plot; both network plots with each legend's name and rate; the GPU plots with `GPU i`, utilisation and VRAM per entry; the process head, count and filter and the columns Pid, Program, MemB and Cpu% with its row graph. P2 is the gauges, P3 the details and P4 storage, the host name and the clock. The version and the core labels and numbers are not in the table: the version is P1, and the core rows show whole wherever the cores panel shows (`ACC-LAYOUT-62-VERSION`, `DEF-VIEW-37-CORE-LABEL`).

| Tier (named sizes) | Panels and forms | Dropped |
|---|---|---|
| full (1440x900, 1280x800) | top bar: brand, version, host, interval, clock, LIVE, READ-ONLY, CONFIG. Band 1: cpu-stat 4, mem-stat 4, proc-stat 4 (total and facts, count plot, foot). Band 2: net 5 (2 plots, x axis, legend: name, rate, now, top, total), cpu-gauge 3 (arc, scale 0/50/100, `total` readout; `load`, `user`, `system`, `pressure` beside), mem-gauge 4 (arc, scale, `used` readout; memory bar, used, cache, free, available, swap beside). Band 3: GPU 5 (a plot per GPU, x axis, legend: `GPU i` and model, utilisation and VRAM, temperature · power), cores 4 (64 rows of label, meter and number in 6 columns), storage 3 (mounts). Band 4: processes, 7 columns, whole rows | nothing |
| drop-p4 (1024x768) | as full, minus P4. Band 3: GPU 5 and cores 4 share storage's width; cores show label, meter and number | host, clock, storage (3 fields) |
| drop-p3 (768x1024, 1440x600) | top bar: brand, version, interval, tier chip, LIVE, READ-ONLY, CONFIG. Band 1: cpu-stat 6, mem-stat 6 (tile and plot). Band 2: net 6 (2 plots, legend: name and rate), cpu-gauge 3 and mem-gauge 3 (arc, ticks, readout, centred); net 8 and the gauges 2 + 2 at 1101 px wide or more. Band 3: GPU 12 (a plot per GPU, legend: `GPU i`, utilisation, VRAM; 2 rows per entry). Band 4: processes, Pid, Program, MemB, Cpu%, `table-layout: auto` | P4 and proc-stat, cpu-stat-foot, mem-stat-foot, net-interface, net-axis-note, net-x-axis, net-legend-detail, cpu-name, cpu-gauge-cores, gauge-scale, cpu-gauge-facts, mem-gauge-total, mem-gauge-facts, gpu-count, gpu-axis-note, gpu-x-axis, gpu-model, gpu-temp-power, cores, proc-command, proc-threads, proc-user (25 fields) |
| drop-p2, one column (390x844; width ≤ 599, height ≥ 541) | one column: cpu-stat, mem-stat, net, GPU (one line per entry: `GPU i`, utilisation, VRAM, no mini bars; legend 192 px), processes (4 P1 columns, the remaining height). Top bar gaps 3 and 2, side padding 4, chip padding 3 | drop-p3 and cpu-gauge, mem-gauge, sample-age (28 fields) |
| drop-p2, two columns (844x390, 667x375, 915x412; height ≤ 540) | left column: band 1 cpu-stat and mem-stat side by side, band 2 net, band 3 GPU (one-line legend); right column: processes, 4 P1 columns, full height | as one column (28 fields) |

Each dropped name is a field. An element that appears once in its panel carries `data-p` and `data-field`; an element repeated per core, process, GPU or direction drops by class, so the SSE fragments carry no repeated marker. The markers add about 800 bytes per tick, +0.6 % of a 126 KB tick.

| Field | P | Element | Dropped by |
|---|---|---|---|
| host, clock | 4 | `.u-host`, `.u-clock` in the top bar | marker |
| storage | 4 | `#p-storage` | marker |
| proc-stat, cores | 3 | `#p-proc-stat`, `#p-cores`; the Processes tile drops whole, its count plot and every fact with it, and no fact drops on its own | marker |
| cpu-stat-foot, mem-stat-foot | 3 | `.stat-foot` of cpu-stat (frequency, load) and mem-stat (used %, available %) | marker |
| net-interface, net-axis-note, net-x-axis | 3 | the net head metas (interface, `scale ▼ … ▲ … · 240 s`) and `.xaxis` | marker |
| net-legend-detail | 3 | `#p-net .legend-meta`: top, total | class |
| net-link | 3 | `.net-link`: the link line and its `err` and `drop` chips | marker |
| cpu-name, cpu-gauge-cores | 3 | `.cpu-name` and the `64 cores` head meta | marker |
| gauge-scale | 3 | a `<g>` around the three `.g-scale` texts, in both gauges | marker |
| cpu-gauge-facts | 3 | `.gauge-side.facts`: `load`, `user`, `system`, `pressure` | marker |
| mem-gauge-total, mem-gauge-facts | 3 | the `of total 503 GiB` head meta and `.gauge-side.mem-rows`: the memory bar, the used, reclaimable, other and free rows, free + reclaimable, swap | marker |
| gpu-count, gpu-axis-note, gpu-x-axis | 3 | the GPU head metas (`3 devices`, `0–100% · 240 s`) and `.xaxis` | marker |
| gpu-model, gpu-temp-power | 3 | `.lg-model` and `#p-gpu .legend-meta`, per GPU | class |
| proc-command, proc-threads, proc-user | 3 | `<col>` and `<th>` 3-5 carry the marker; `td:nth-child(3)` to `(5)` drop by class | marker and class |
| cpu-gauge, mem-gauge | 2 | `#p-cpu-gauge`, `#p-mem-gauge` | marker |
| sample-age | 2 | `#sample-age` in every live chip state: the age (`0.3 s`) in LIVE and PAUSED, `since HH:MM:SS` in STALE; the chip keeps its state word | class |
| container-chip | 2 | `.chip-container` in the top bar, present only inside a container (section 16) | marker |

Measured on the live host, storage collapsed; sizes in px:

| Size | Tier, chip | Band heights | Plots | Gauges | Process rows, columns |
|---|---|---|---|---|---|
| 1440x900 | full, none | 116/214/190/314 | cpu 467x28, net 2 x 390x78, GPU 534x135 | 163x172 | 12, 7 |
| 1280x800 | full, none | 104/190/167/276 | cpu 415x20, net 2 x 338x66, GPU 471x113 | 141x149 | 11, 7 |
| 1024x768 | drop-p4, `P1-P3` | 99/183/162/268 | cpu 331x22, net 2 x 256x64, GPU 342x111 | 136x145 | 11, 7 |
| 768x1024 | drop-p3, `P1-P2` | 136/250/222/367 | cpu 374x79, net 2 x 233x107, GPU 542x189 | 165x212 | 17, 4 |
| 390x844 | drop-p2 one column, `P1` | 161 (78 + 78)/159/159/318 | cpu 378x24, net 2 x 240x61, GPU 160x126 | - | 14, 4 |
| 844x390 | drop-p2 two columns, `P1` | left 70/139/139, right 358 | cpu 203x16, net 2 x 275x51, GPU 195x106 | - | 17, 4 |
| 667x375 | drop-p2 two columns, `P1` | left 68/132/132, right 343 | cpu 159x14, net 2 x 186x48, GPU 106x99 | - | 16, 4 |
| 915x412 | drop-p2 two columns, `P1` | left 75/147/147, right 380 | cpu 221x20, net 2 x 310x55, GPU 230x114 | - | 18, 4 |

- The sweep is 88 sizes: widths 360-1920 px in 40 px steps at 800 high, 1024 high for widths 720-1080, 844 high for widths 360-600, heights 600-1200 in 50 px steps at 1440 and 390 wide, the five named sizes and the three landscape phones. Every size passes on the live host and the fixture tree, on a fresh load and on a live resize: no page scroll, no text over a `[data-chart]` box, no panel overlap, no cut text and no partial process row
- Text reaches 4.5:1 on its composed layers in the ten tabulated variants at 1440x900, 1024x768, 768x1024 and 390x844. The lowest is 4.51:1, the network down arrow in Solarized light
- The network and GPU plots shrink from 1440x900 to 1280x800 to 1024x768. The cpu-stat and mem-stat plots go 28 → 20 → 22 px, because band 1 sits at its minimum at 1280x800 and the fluid head, tile and foot shrink more than the band between the two sizes. The concept has the same pattern; the breakpoints it replaced gave 24 and 23 px. This breaks the strict reading of `ACC-LAYOUT-20-ADAPTIVE` and is an open owner decision
- Collapse is not a drop. Storage collapses on a host with no mounts and GPU on a host with no device, at every tier. On the fixture tree band 3 holds cores and storage at full and drop-p4 and hides from drop-p3 down
- The process row graph (`ACC-VIEW-52-PROC-GRAPH`) belongs to the Cpu% cell and stays at every tier. The four P1 columns stay whole down to a 326 px process column (667x375)
- 568x320 fails on the live host: the Cpu% number is cut in a 277 px process column. It is outside the sweep and not handled
- Each GPU entry is centred beside its own lane, and a fourth GPU repeats the first colour. With eight GPUs, injected into the page, every lane keeps 14 px or more at 1440x900, 1024x768, 1280x720, 390x844 and 390x600
- On the fixture tree the 9 recorded processes leave empty rows below them at every size, because the table has fewer rows than room

### 9.5 Grid rules

- Every panel has `min-width: 0` and `min-height: 0`; a long GPU model or mount path ends in an ellipsis and keeps the full text in `title`; a long command ends in an ellipsis
- The page fills the viewport with no `max-width`; `body` does not scroll
- DOM order matches visual order at every breakpoint. No `order` overrides
- The process table wrapper is the only element that scrolls; the CONFIG popover scrolls only when it is taller than the viewport

## 10. Accessibility

- **Contrast.** WCAG AA in every variant: 4.5:1 for normal text, 3:1 for large text, 3:1 for non-text UI and graphics. Every text token reaches 4.5:1 on the layer it sits on, composed (section 3.4), and `tests/theme_contrast.rs` enforces it for all eight pairs in dark and light.
- **Semantics.** The top bar is a `<header>`, the bands sit in `<main>`, and each panel is a `<section>` with `aria-labelledby` on its `<h2>`. The process table is a native `<table>` with a visually hidden caption and `scope="col"` headers; each header holds a sort button, the active header carries `aria-sort`, and each parent row's chevron is a button with `aria-expanded` and a label that names the process (section 8.9). The CONFIG popover is `role="dialog"` with `aria-label="Config"`; the CONFIG chip carries `aria-expanded` and `aria-controls`. Its seven pages are a `role="tablist"` of `role="tab"` chips with `aria-selected`, each controlling a `role="tabpanel"` (section 8.14). The theme picker and the sort select are native `<select>` elements; the GPU and panel shown chips are `role="checkbox"` with `aria-checked`, the S, M and L chips, the `procs`, `tree` and `dir` chips and the `motion` chips are `role="radio"` in a `radiogroup`, and pause and the server options' `on` and `off` chips carry `aria-pressed`. The page has no `<h1>`.
- **Live region.** Only the live chip's state word is live: `aria-live="polite"`, written only when the state changes. The sample age is `aria-hidden`. Metric values are not live regions.
- **Focus.** Every interactive element shows a 1 px `--accent` outline at 1 px offset through `:focus-visible`. Opening the CONFIG popover focuses its selected tab; `Escape` closes it and returns focus to the CONFIG chip. The page has no skip link.
- **No colour-only encoding.** The live state is a word, read-only is the words `READ-ONLY`, the CPU readout's band colour sits on a printed number, and every meter and core row prints its value.
- **Plots and gauges.** Each plot box is `role="img"` with an `aria-label` naming the series; the svg inside is `aria-hidden`. Each gauge is `role="img"` with an `aria-label` that states btop's raw value, for example "CPU total 12.0 percent", or "CPU unavailable".
- **Meters.** The storage and core meters are decorative; the printed number beside each is the content.
- **Reduced motion.** The page does not consult `prefers-reduced-motion`: the CONFIG motion toggle is the only switch (section 6). The scale change of a network plot or the process count graph (section 7.4) also runs for every viewer, because `app.css` has no rule for it; both animations are transforms only.
- **Forced colours.** `app.css` has no `forced-colors` rule, and the page has not been checked under `forced-colors: active`.
- **Zoom and reflow.** The page fits the viewport with no page scroll at 1440x900, 1280x800, 1024x768 and 390x844 (section 9.4); the process table is the only element that scrolls. 320 px and 200 % zoom have not been measured.
- **Keyboard.** `Tab` follows the DOM order, and every control is a button, a select or an input. `Escape` closes the CONFIG popover and clears the process filter; Left, Right, Home and End move between the config pages. No shortcut key is bound, and the process table has no keyboard row navigation; its header buttons and chevrons are in the `Tab` order, and Enter or Space sorts or toggles.

## 11. Performance budget

The dashboard reports the host's load and must not become part of it (`ACC-PERF-16-NO-OVERHEAD`).

### 11.1 Collection rules

- **One sampler tick per interval.** One collection pass feeds a shared watch channel; every SSE client reads the same tick. N clients cost no extra collection.
- **Static facts are cached.** CPU model, core count, mount list, interface list and GPU inventory are read once and refreshed slowly, never per tick.
- **One tick, one fan-out.** No per-client collection, no per-panel collection, no polling from the page.

### 11.2 Render rules

- **Fragment-driven.** The page changes the DOM only when a fragment arrives: `app.js` writes each out-of-band fragment (section 9.2) into the nodes already on the page, except the process table, which it merges by pid 50 ms later, in a frame apart from the charts' new paths (section 8.9); then `app.js` sets the clock of a running scale change and re-applies the GPU selection, the process view, filter and sort, and the band sync. While a number effect can run, a fragment replaces its panel's nodes instead, because an effect's layers live inside the value's node. The page polls nothing.
- **Two timers, stated.** The STALE timeout runs while the stream is live, because a stream that stops sends no event to notice. It is re-armed on every cpu frame and fires after 2 x the interval without one: 500 ms at the 250 ms default. The process rows' 50 ms merge runs once per slow pass. The process filter's 150 ms debounce runs only while the user types. The clock and the sample age are repainted when a cpu frame arrives; the STALE timeout writes only the last frame's time into the sample age.
- **No chart work in the browser.** Charts are SVG paths built on the server; scrolling, hovering and resizing rebuild nothing. The scroll is one compositor animation per chart's svg, restarted once per slow pass; during a network scale change the browser animates one CSS transform per plot for 1 s or more. Neither builds a path. A `ResizeObserver` on the process panel recomputes the row height when the panel's size changes, not per frame.
- **Hidden tabs.** The stream stays open and a hidden tab applies fragments as they arrive; `visibilitychange` stops the animation clock. Pause in the config popover holds the frames, and resume applies the newest frame of each event.
- **Theme changes restyle only.** Switching theme or mode changes CSS custom properties; no chart is rebuilt.
- **Only changes are written.** A fragment writes only the attributes and text that differ, in every panel and the process table alike, so an unchanged node is neither restyled nor repainted and a chart keeps its compositor layer; a node whose element name differs from the one in its place is replaced.

### 11.3 Thresholds

| Threshold | Value at the 250 ms default | Why |
|---|---|---|
| Sampler ticks | 4 per second, one per interval; 1 and 10 clients see the same tick count within 1 over 20 s | one collection shared by every client (`ACC-PERF-16-NO-OVERHEAD`, `ACC-CONF-39-SAMPLE-RATE`) |
| Sampler tick, p95 | <= 8 ms | 3.2 % of the interval; the collector stays invisible in the host's own load |
| Sampler tick, max | <= 25 ms | 10 % of the interval; a late tick is skipped (`MissedTickBehavior::Skip`), not queued |
| NVML sweep | off the tick, at most one in flight; each tick takes the newest finished sweep | a sweep of three GPUs took 182-1267 ms, longer than one tick |
| Chart build | on the server, inside the tick | the browser draws no curve |
| STALE | no cpu frame for 2 x 250 ms = 500 ms | the only timer (section 11.2) |
| Client fragment apply | <= 16 ms | one display frame |

- Measured on 2026-10-26 on the live host at 250 ms (`tmp/workflows/btop-web-gpu-load/docs/tick-cost.json`): the server process used 8.3 ms of CPU per tick with no client and 8.8 ms with one client, which is 3.3-3.5 % of one core. The figure includes the off-tick NVML sweeps and the chart paths, so it is not the tick's own duration.
- The server does not record per-tick durations and the page shows no tick cost, so the p95 and max rows are targets without a gate. The tick-count row is the `ACC-PERF-16-NO-OVERHEAD` check.

## 12. Content rules

### 12.1 Names

- Panel names: `CPU`, `Memory`, `Network`, `Processes`, `GPU`. These are btop's names and are not renamed.
- Field names: `Used`, `Available`, `Cache`, `Other`, `Free`, `Swap`, `Load`, `Frequency`, `Temperature`, `Threads`, `User`, `MemB`, `Cpu%`. `Cache` and `Other` replace btop's `Cached` (`DEF-VIEW-50-MEM-TERMS`).
- The process CPU column is labelled `Cpu%`, btop's header. It is the share of all cores, so one saturated core reads about 1.6% on 64 logical CPUs; the page has no footer that says so.

### 12.2 Units and precision

| Field | Unit | Precision |
|---|---|---|
| Total CPU | % | btop's integer 0-100, printed as a whole percent (`12`) |
| Frequency | GHz | two decimals (3.99 GHz from 3994.064 MHz) |
| Load | dimensionless | integers in the cpu-stat foot (`85 85 87`); divided by the core count with two decimals in the cpu-gauge facts (`1.32 1.33 1.36`) |
| Temperature | °C | integer; not drawn when unread, as on this host |
| Per-core | % | integer 0-100 each, of the core's 1 s EMA; 64 values |
| Memory | GiB | btop `floating_humanizer`, non-short |
| Memory percent | % | integer, except a non-zero value below 1% gets one decimal |
| Swap | MiB | non-short value; percent of total swap |
| Disk IO | % | integer; btop activity percent, not throughput |
| Network rate | B/s | non-short humanized, per second; bits in parentheses |
| Network top | bit/s | subsequent samples |
| Network total | bytes | cumulative since boot with rollover and offset |
| Process threads | integer | `K` suffix above 9999 |
| Process CPU | % | one decimal below 10; integer 10-999; `k` above 9999 |
| Process memory | bytes | short humanized by default |
| GPU utilization | % | integer 0-100 |
| GPU VRAM | bytes / % | humanized bytes; integer percent, one decimal below 1% |
| GPU temperature | °C | integer |
| GPU power | W | one decimal from mW/1000; integer cap |

### 12.3 Rounding and absence

- Percentages round to integers. A non-zero value that would round to 0 or 100 keeps one decimal, so a row cannot read as zero when it is not. Swap at 0.35% is the case: `0.3%`.
- A missing counter is `unavailable`, not 0. btop's `readfile` fallback cannot distinguish a missing file from a real zero; the web collector must.
- A process field whose read failed is left empty in its cell.
- A mount whose `statvfs` fails is ignored for the rest of the process lifetime, as btop's ignore list does.
- The page prints the word `unavailable` without a reason (section 8.11).

### 12.4 Stated divergences from btop

- The web shows full values where btop truncates: load to two decimals, frequency `3.99 GHz` where btop's short form is `4.0 GHz`.
- The core rows show each core's 1 s EMA, meter and number alike, where btop prints the instant percent; the owner chose this on 2026-10-26 so the panel stops jumping (`ACC-VIEW-48-CORE-BARS`).
- The core colours mix btop's cpu gradient in linear light where btop mixes in gamma sRGB (`btop_theme.cpp:291-335`): only a linear-light mix keeps every value's luminance between the stops, so the stops' 4.5:1 floor holds at every value on dark surfaces too; a gamma mix of two colours can fall below both (`ACC-VIEW-56-CORE-COLOUR`).
- The Memory panel draws one bar, used | cache | free = MemTotal, with free = MemFree and cache = MemAvailable − MemFree, in place of btop's cached row: btop's Cached leaves Buffers and reclaimable slab out, so used + Cached + free is not MemTotal (`DEF-VIEW-50-MEM-TERMS`, owner 2026-10-28). btop's strings truncate, so the legend raises cache, other or free by one unit of its last digit, largest remainder first, until the printed parts add up to the total within one such unit; used keeps btop's string, and free + reclaimable prints the printed free row plus the printed reclaimable rows.
- The Processes tile's facts are btop-web additions the owner asked for on 2026-10-27; btop 1.3.0 draws only the process count (`Proc::draw`, 1537). Over the process rows after btop's filters: `sleeping`, `waiting` and `zombie` count the rows in btop's `Sleeping`, `Waiting` (`D`) and `Zombie` (`Z`) states as `running` counts `Running`; `top` is the row with the highest Cpu%, the table's value, the lower pid on a tie; `80% cpu` and `80% mem` are the fewest rows, largest first, whose Cpu% (MemB) reaches 80 percent of all rows' sum, 0 when the sum is 0; `gpu` is the number of distinct pids over NVML's compute and graphics process lists of every device, unavailable on WSL2, where NVML returns empty lists while processes hold VRAM (0 processes while GPU 2 held 2.1 GiB), and with a DRM sysfs or ROCm SMI device, which cannot list processes; in a container it is the number of the container's processes that hold a GPU device node open, `gpu n+` when some could not be read (section 16.2, `DEF-VIEW-48-GPU-FACT`). The count plot draws the total (`ACC-VIEW-53-PROC-TILE`, section 14.3).
- The process list follows the owner's choices of 2026-10-27 (`ACC-VIEW-58-PROC-VIEW`, `ACC-VIEW-59-PROC-SORT`, section 8.9). The `top` view is a btop-web addition; btop has its flat list and its tree only. btop's `proc_tree` picks only the starting view. A fresh viewer sorts by MemB, largest first, where btop starts on `proc_sorting` (`cpu lazy` by default), and the browser's sort has no lazy variant. A tree filter match keeps and opens its ancestors, where btop shows a match at depth 0 with its descendants (`btop_shared.cpp:119-131`). The tree keeps btop's slot per ancestor level and its middle, last and first-root branch meanings, drawn as 1 px CSS rules at one 12 px step instead of glyphs, with a rotating chevron button replacing `[+]─` and `[-]─` (`ACC-VIEW-180-PROC-TREE`); the tree starts at display level 3, where btop expands every node. A collapsed row below the top level shows no graph, where btop graphs every collapsed row's sum: a busy process deep in the tree gave every ancestor a summed graph, and the server's cost per tick rose above the build before the views
- `pressure` (the `some avg10` value of the kernel's CPU PSI file) is a btop-web addition; btop 1.3.0 reads no PSI file. `user` and `system` are btop's own time fields, which btop collects and does not draw (`ACC-VIEW-178-CPU-FACTS`).
- The web shows `unavailable` in a stat tile, gauge or legend where btop hides a missing metric.
- The disk list shows the statvfs mounts the collector reads. btop's `use_fstab=True` with an empty `/etc/fstab` would show only the swap disk on this host; the source decision is recorded as open question Q4.
- Inside a container every panel shows only that container, from its cgroup v2 files and namespaces, where btop shows the host; a metric with no container-scoped source is unavailable. The owner ordered this on 2026-10-27 (`ACC-SCOPE-64-CONTAINER`, section 16).
- `graph_max` rises after five consecutive ticks, btop's count at every interval, and falls only after `net_fall` seconds (300 by default) below a tenth of the ceiling, where btop falls after five ticks, 10 s at its interval (`ACC-GRAPH-96-NET-AUTOSCALE`, section 7.4). At btop's 2000 ms the fall window is 150 ticks against btop's 5.
- The network plots show a decade axis of bytes per second, where btop draws to `graph_max`; the ceiling rises at once and only a page load lowers it, where `graph_max` falls after a calm spell (`ACC-GRAPH-164-NET-SCALE`, section 7.4). btop prints a bits-per-second figure beside the rate; btop-web prints bytes per second only, the owner's decision of 2026-10-30 (`DEF-GRAPH-102-NET-SCALE`). The link line is a btop-web addition; btop's net box has no link facts (`ACC-VIEW-174-NET-STRIP`).
- The network plots never show a ceiling below the decade of `net_floor` Mbit/s (1 by default, so 1 MiB/s), where btop's floor is 10 KiB, and a new ceiling is reached over 1 s or more, where btop redraws at once (`ACC-GRAPH-96-NET-AUTOSCALE`, `ACC-GRAPH-67-SMOOTH-SCALE`, section 7.4). btop's `graph_max` values are kept unchanged.
- Without a matching `net_iface`, the network panel skips `lo` unless it is the only interface, where btop takes the interface with the largest total (`DEF-NET-42-LOOPBACK-PICK`, section 7.4).

### 12.5 Microcopy

- State words are literal: `live`, `paused`, `stale`, `READ-ONLY`, `unavailable`; the chips render them in uppercase.
- No exclamation marks, no apology, no "Oops". An error states what is missing and what still works.
- Units stay with their value: `3.99 GHz`, `94 KiB/s`, `36 °C`, not `3.99` and `GHz` in separate corners.

## 13. Do and don't

### Do

- Read semantic tokens in components; add a theme by adding two theme blocks with the same token names.
- State `unavailable` for a stat tile, gauge or legend value the collector cannot read (section 8.11).
- Keep the number, unit and precision together; use tabular numerals for every telemetry value.
- Keep btop's panel names, field names, units and hierarchy; keep btop's per-core rows wherever the drop order in section 9.4 shows the cores panel.
- Use PCHIP for every sparkline and history chart; keep the knots exact and the curve inside the sample range.
- Show all three NVML devices by default; persist the device selection and fail soft when storage is blocked.
- Keep the READ-ONLY chip visible while the guard is on, with its reason for screen readers.
- Put the state in a word as well as a colour.
- Use the shipped steps: 8 px page padding and gaps at 1440x900, 5-8 px along `--gap` (section 9.3).

### Don't

- Don't use a raw btop colour for text or a required border. Raw values are for chart gradients after the 3:1 clamp.
- Don't put `--muted` on `--surface-raised` or `--surface-sunken`; the measured contrast is 4.31-4.45:1.
- Don't use `--ok`, `--warn` or `--crit` below the 4.5:1 text floor; they carry process percentage text in both modes.
- Don't set normal-size link text in `--accent`; links use `--text-strong` with an `--accent` underline, because accent is a 3:1 non-text token.
- Don't render a missing metric as `0`, `--` or a fake gauge.
- Don't copy btop's braille, block glyphs, box-drawing borders, superscript panel numbers or hotkey letters.
- Don't animate a value between readouts, glide a gauge, pulse the live dot, auto-scroll the table or transition a layout property; the animation clock moves chart geometry only.
- Don't add a timer, a second `requestAnimationFrame` loop, a polling fetch or per-frame work beyond the animation clock (`ACC-GRAPH-98-ANIM-CLOCK`).
- Don't drop a field outside the order in section 9.4.
- Don't enable terminate, kill, signal or priority in read-only mode; there is no unlock toggle.
- Don't encode a state in colour alone.
- Don't copy JupyterLab tokens, `jp-*` class names or DOM; only the house habits travel.

## 14. Implementation map

| File | Role |
|---|---|
| `crates/btop-web/templates/index.html` | The page: top bar, four bands of panels, the CONFIG popover with its seven pages and the five SSE swap targets |
| `crates/btop-web/templates/partials/*.html` | One partial per panel, rendered into the page and into the SSE fragments; `cfg_key.html` is one config-menu server option, as text or as a form (section 8.14) |
| `crates/btop-web/templates/event_*.html` | The five SSE fragments, each with its out-of-band targets (section 9.2) |
| `crates/btop-web/assets/app.css` | Layout, components and breakpoints; it adds no colour of its own |
| `crates/btop-web/assets/theme.js` | Theme, mode and layout bootstrap, run in `<head>` before first paint; the storage keys and the `try/catch` contract |
| `crates/btop-web/assets/app.js` | Live state, the CONFIG popover and its tabs, gadgets, GPU selection, pause, the process view, tree, sort, filter and frame merge, the row fit, the animation clock and the scale-change clock |
| `crates/btop-web/src/chart.rs` | PCHIP paths and their one-slot tail, `History` and its 1 s EMA, `Scale` for a smooth scale change, the gauge geometry |
| `crates/btop-web/src/theme.rs` | Raw key order, fallbacks, `ensure_contrast` and `dim_towards`; the Default overrides `STEEL_DARK`, `STEEL_LIGHT` and `DEFAULT_PANEL_BORDERS`; `render_css` writes the theme blocks into the page |
| `crates/btop-web/src/collector/container.rs` | Container detection, the cgroup directory, the network namespace check and the effective CPUs (section 16) |
| `crates/btop-web/src/config.rs` | The btop.conf parser and the flags' values; `SERVER_OPTIONS`, `Config::get`, `Config::set`, `replace_key_line` and `save` for the config menu (section 8.14) |
| `crates/btop-web/src/routes/process.rs` | `GET proc/PID`: the process popup's fragment, relayed from the agent in client mode (section 8.9.1) |
| `crates/btop-web/src/routes/settings.rs` | `POST config`: the Origin check, validation, the file write and the hand-over to the sampler (section 8.14) |
| `crates/btop-web/themes/*.theme` | The fourteen file-backed palettes: btop's four pairs unmodified with their provenance headers, and the three designed pairs |
| `docs/design/notes.md` | This system: tokens, components, layout, rules, thresholds |
| `docs/design/index.html` | Historical: the earlier rendered proof, with inline CSS and JS and every proof state |

- `/assets`: every response carries a strong `ETag`, rust-embed's build-time sha256 of the file bytes, and `Cache-Control: no-cache`; a matching `If-None-Match` (weak prefixes, comma lists and `*` included) answers 304 with the same validator, so a new build reaches a returning browser on its next load (`DEF-VIEW-59-ASSET-CACHE`).

`make ci` runs 269 tests: 198 library, 5 end-to-end, 55 HTTP, 11 theme contrast. Collector tests read recorded trees through `BTOP_WEB_PROC_ROOT`; a mount path and the interface list resolve inside the tree, so a test asserts nothing about the kernel that runs it (`DEF-TEST-52-HOST-MOUNT`, `DEF-TEST-53-HOST-IFACE`).

### 14.1 Theme block contract

```css
[data-theme="<id>"][data-mode="dark|light"]{--main-bg:#...;/* btop's raw keys, then the semantic tokens; the same names in every block */}
```

- `data-theme` values, in the order of the `theme` numbers 0-8: `default`, `everforest-medium`, `flexoki`, `gruvbox`, `solarized`, `synthwave`, `terracotta`, `ultraviolet`, `cyberpunk`. A pair appended after the first eight lists last (`APPENDED` in `theme.rs`), so the numbers 0-7 keep their themes. The Default dark block also applies to `:root`.
- The cyberpunk pair also sets `--deco-glow`, `--deco-trace` and `--deco-corner` in `app.css`; every other pair keeps the `:root` `none`. The panel paints the glow on its border and the two SVG layers on its pseudo-elements in the 4 px rim below content (ACC-THEME-101-CYBERPUNK).
- `data-mode` values: `dark`, `light`. The stored mode choice can also be `auto`, which `theme.js` resolves to one of the two (section 8.4).
- Adding a family: add its dark and light `.theme` files to `crates/btop-web/themes/`; the server pairs them by the `dark` or `light` token in the name, writes both blocks and adds the picker option. A file without a pair gets the other mode by inversion. No component changes.
- An unknown stored theme is replaced with the picker's first option, `default`, and the key is rewritten.

### 14.2 Acceptance criteria

| Id | Contract |
|---|---|
| `ACC-SAFE-12-READONLY` | With `--readonly`, the server refuses kill, signal, priority and every other mutating request, whatever the client sends |
| `ACC-SAFE-13-READONLY-UI` | With `--readonly`, the UI hides or disables every control that would change the system and states why |
| `ACC-CONF-61-CONFIG-PAGES` | The config menu has btop's option pages plus Layout; with the guard on it changes display options only, stored per viewer in the browser; with the guard off it also changes server options for every viewer and writes them to the loaded config file |
| `ACC-GRAPH-17-PCHIP-SMOOTH` | Every sparkline and history chart interpolates with PCHIP; exact knots, monotone runs, no overshoot |
| `ACC-PERF-16-NO-OVERHEAD` | One sampler tick per interval fanned out to every client; static facts cached; no busy loops; N clients cost no extra collection |
| `ACC-GRAPH-67-SMOOTH-SCALE` | When an auto-scaled graph changes its scale, the lines and the scale label move to the new scale over several frames, never in one frame |
| `ACC-GRAPH-68-PLOT-GRID` | Every plot except the small CPU graph in each process row has a very faint vertical and horizontal grid under its lines, never over a value |
| `ACC-GRAPH-96-NET-AUTOSCALE` | btop's `graph_max` rises after five consecutive ticks above it and falls after `net_fall` seconds below a tenth of it; the network plots never show less than `net_floor` and print the top value |

### 14.3 Source map

| Claim | Source |
|---|---|
| CPU formulas, per-core percent, frequency, load | `src/linux/btop_collect.cpp:544-574,817-897`; `src/btop_draw.cpp:780-794` |
| Memory and swap formulas | `src/linux/btop_collect.cpp:1510-2074` |
| Humanizer and uptime | `src/btop_tools.cpp:399-485,504-542` |
| Network counters and scale | `src/linux/btop_collect.cpp:2076-2286`; `src/btop_draw.cpp:1368-1449` |
| Process table, sort and filter | `src/linux/btop_collect.cpp:2288-2777`; `src/btop_shared.cpp:41-192`; `src/btop_draw.cpp:1537-1934` |
| Processes tile facts (btop-web additions) | the process rows of `collector/proc.rs` (state words from btop's `Proc::proc_states`, Cpu% from btop's cpu_p, MemB from btop's RSS); the GPU process count from nvml-wrapper 0.13 `running_compute_processes` and `running_graphics_processes` in the NVML sweep of `collector/gpu.rs`, and in a container from an fd walk over the cgroup members with nix 0.30 `Dir` and `readlinkat`; formulas in section 12.4 and the module docs |
| GPU fields | `src/btop_draw.cpp:939-1087`; `src/linux/btop_collect.cpp:106-1508`; `src/btop_shared.hpp:108-149` |
| Battery fields | `src/linux/btop_collect.cpp:666-801`; `src/btop_draw.cpp:705-743` |
| Theme fallbacks and gradients | `src/btop_theme.cpp:265-287,321-346` |
| Read-only surfaces | `src/btop_menu.cpp:902,1018`; `src/btop_input.cpp:398-409` |
| btop panel hierarchy | `src/btop_draw.cpp:1955-2238` |

## 15. Borrowed from

Each panel and component of the shipped page names the btop source it follows and the researched reference its presentation follows (`ACC-DESIGN-22-BORROW-REFS`). btop supplies the information architecture, the fields and their formats; the references supply the panel anatomy, typography and chart presentation. btop line numbers are in `src/btop_draw.cpp` of btop 1.3.0 (`tmp/btop-src`) unless stated. The researched references are the ones the design-revision research opened, recorded in `tmp/workflows/btop-web-design-rev/checkpoints/web-dashboards.json`, `patterns.json` and `revbrief.md`:

- Grafana Node Exporter Full, dashboard 1860 (<https://grafana.com/grafana/dashboards/1860-node-exporter-full/>): a row of stat and gauge panels above four time-series panels
- Grafana stat panel documentation (<https://grafana.com/docs/grafana/latest/visualizations/panels-visualizations/visualizations/stat/>): one dominant value with the sparkline behind it, hidden when the tile is too small
- Netdata (<https://www.netdata.cloud/>): numeral-first tiles and Live/Stale/Offline status words with counts
- Tufte, sparkline theory and practice (<https://www.edwardtufte.com/notebook/sparkline-theory-and-practice-edward-tufte/>): word-sized, frameless graphics tied to a printed number
- Small multiples (<https://en.wikipedia.org/wiki/Small_multiple>): similar graphics on one shared scale
- The JupyterLab device-bridge chip (`ACC-VIEW-18-HOUSE-CHIP`): 1 px border, 2 px radius, uppercase, a 12 % tint

Concept 05 names Grafana's gauge, time-series and table panel types. The research opened the dashboard 1860 screenshots, which show the gauge and time-series panels; it did not open Grafana's documentation for the gauge, time-series or table panel.

| Element | btop source | Researched reference | What is borrowed |
|---|---|---|---|
| Top bar | the CPU box top border: `update_ms` + `ms` in `Cpu::draw` (587); the clock and its `/host` format in `Draw::update_clock` (297-346) | Netdata top bar; Grafana 1860 toolbar | the interval, clock and host from btop; the live state as a word chip with the sample age from Netdata; one compact control group at the right end from Grafana |
| `cpu-stat` | `Cpu::draw` (530): the total CPU graph, frequency and load average | Grafana stat panel; Netdata tile | one dominant value with the unit beside it, the plot below it, details in the foot |
| `mem-stat` | `Mem::draw` (1107): used and available | Grafana stat panel | as cpu-stat |
| `proc-stat` | `Proc::draw` (1537): the process count `numpids`; the state words of `Proc::proc_states`; the Cpu% and MemB columns | Grafana stat panel; Netdata tile | one dominant value with the facts beside it, the count plot below, details in the foot; btop draws no process-count graph and none of the facts, which are btop-web additions (section 12.4) |
| `net` | `Net::draw` (1368-1449): download above upload, `▼` and `▲`, the rate, `Top:` and `Total:`, the auto scale `graph_max` | Grafana time-series panel (dashboard 1860 Network Traffic Basic) | two plots on one time axis with gridlines and axis labels below them; the legend beside the plots, never over them |
| `cpu-gauge` | `Cpu::draw`: the CPU name, core count, frequency, load and temperature | Grafana gauge panel (dashboard 1860 CPU Busy) | the arc; the readout, ticks and labels follow `ACC-GRAPH-42-GAUGE-TICKS` |
| `mem-gauge` | `Mem::draw`: used, available, cached, free and swap | Grafana gauge panel (dashboard 1860 RAM Used) | the arc, with a used, cache and free bar, a row per segment, free + reclaimable and swap beside it (`DEF-VIEW-50-MEM-TERMS`, `ACC-VIEW-195-MEM-SUM`) |
| `cores` | `Cpu::draw` (810-830): the per-core list, `C{n}`, a one-row graph per core when the box is wide enough, and an integer percent coloured by the cpu gradient with `%` in `main_fg`; `Meter` (367-383) | small multiples | btop's per-core rows with a meter in place of the one-row graph, on one shared 0-100 % scale, coloured by btop's cpu gradient as btop colours a meter and the percent; the values are the 1 s EMA (`ACC-VIEW-48-CORE-BARS`, `ACC-VIEW-56-CORE-COLOUR`) |
| `gpu` | `Gpu::draw` (939-1087): name, utilisation, temperature, power and VRAM `used / total` | the network panel's legend (concept 05.06-network-paired); concept 05.06-adaptive-base `#gpu-load` | a GPU load plot per device (next row), each with its legend entry beside it: an edge tab per line, `GPU i` and the model, btop's utilisation, VRAM, temperature and power readouts, and LOAD, MEM and PWR mini bars (`ACC-GRAPH-43-GPU-LOAD`, `ACC-VIEW-79-GPU-BARS`, `ACC-VIEW-82-GPU-TAB`) |
| GPU load plot | `Gpu::draw` (972): one utilisation history graph per GPU box, drawn with the `cpu` gradient | Grafana time-series panel (dashboard 1860: several series on one time axis with a legend); small multiples (one shared scale) | one 0-100 % plot per device, as btop draws a graph per GPU box, stacked on the network plot's 240 s axis; the series colours `--chart-gpu-0` to `--chart-gpu-2` come from btop's `cpu_mid`, `download_mid` and `temp_end`, clamped to 3:1 |
| `storage` | `Mem::draw` disks block (1293-1318): mount, used and free meters, IO activity | none | btop's disk block as meter rows |
| `proc` | `Proc::draw` (1537-1934): the columns Pid, Program, Command, Threads, User, MemB and Cpu%, the filter, and the `selected/total` count (1922) | Grafana table panel as concept 05 names it | btop's process table with a sticky head and zebra rows |
| Gauge arc | none | Grafana gauge panel (dashboard 1860 draws semicircles) | an arc gauge; the 270° sweep comes from concept 05. The arc draws the 1 s EMA, this project's own rule (`ACC-GRAPH-47-GAUGE-SMOOTH`); no predictor or filter was built, so none has a source to name (section 6.1) |
| Sparklines and plots | btop graphs (`Draw::Graph`, 455-502), built from the sample history | Tufte sparkline; Grafana stat panel | an area under a line, no frame, the number beside it; PCHIP (section 7) and the 1 s EMA (`ACC-GRAPH-38-TEMPORAL`) are this project's own rules |
| Meters | btop meters (`Draw::Meter`, 362) | none | a track with a fill and the value printed beside it |
| Chips: LIVE, PAUSED, STALE, READ-ONLY, CONFIG | none | the house chip; Netdata status words | a word in a 1 px, 2 px radius chip with a 12 % tint |
| CONFIG popover and gadgets | the page names and order of btop's options menu (`btop_menu.cpp:1417`) and each server option's key and first description line; the menu itself is terminal-only | Grafana 1860 toolbar | settings in one menu reached from the top bar, out of the page body (`ACC-VIEW-19-CONFIG-MENU`), on tabbed pages (`ACC-CONF-61-CONFIG-PAGES`) |
| Themes | btop theme files and fallbacks (`src/btop_theme.cpp`) | none | the eight families and their gradients (section 3) |

## 16. Container scope

Inside a container btop-web shows only that container: processes, CPU, cores, load, memory, swap, disks, network and GPU (`ACC-SCOPE-64-CONTAINER`, CRITICAL, ordered by the owner on 2026-10-27). A metric with no container-scoped source renders as unavailable, never as the host value and never as 0. The owner's exception of 2026-10-28: the Memory panel's total and free are the system's MemTotal and MemFree, and `other` is the memory used outside the container (`DEF-VIEW-50-MEM-TERMS`). Inside a container this rule outranks btop collection parity; on a host no detection signal fires and every collector keeps its btop 1.3.0 path. Each collector module records its container source, formula and period beside the code and states that it departs from btop on the owner's order.

### 16.1 Detection

Detection runs once at startup in `main.rs`, after the configuration loads, on the tree that `BTOP_WEB_PROC_ROOT` selects (`src/collector/container.rs`). `<root>` is the parent of `<proc>` (`/` live) and `<sys>` is `<root>/sys`. No signal reads the process environment through `std::env`, so the tests stay hermetic. The first signal that fires wins:

| Order | Signal | Read | Runtime named |
|---|---|---|---|
| 1 | Docker marker | `<root>/.dockerenv` exists | `docker` |
| 2 | Podman marker | `<root>/run/.containerenv` exists | `podman` |
| 3 | `container=<v>` in btop-web's environment | `<proc>/self/environ` | `<v>` (podman, oci, lxc, systemd-nspawn, docker) |
| 4 | `container=<v>` in pid 1's environment | `<proc>/1/environ`, when readable | `<v>` (LXC and Incus set it for the container's init) |
| 5 | `KUBERNETES_SERVICE_HOST=` in btop-web's environment | `<proc>/self/environ` | `kubernetes` (the kubelet sets it in every container) |
| 6 | Runtime segment in the cgroup v2 path | the `0::<path>` line of `<proc>/self/cgroup`: a segment `docker`, `kubepods*`, `lxc.payload*`, or `<p>-<hex id>.scope` with `<p>` one of docker, libpod, crio, cri-containerd, nerdctl | from the segment |
| 7 | Container-store root mount | the `/` line of `<proc>/self/mountinfo`: fstype `overlay` with `containerd`, `/docker/` or `/containers/storage/` in its options, or fstype `fuse.fuse-overlayfs` | `container` |

- Signal 7 reads `mountinfo`, never `mounts`: the host fixture `tests/fixtures/proc/self/mounts` was recorded in a container and carries an overlay root, so a `mounts`-based signal would turn the host fixture into a container
- The cgroup directory is `<sys>/fs/cgroup/<path>` from the `0::<path>` line, whichever signal fired. Under a private cgroup namespace, the Docker, Podman and Kubernetes default on cgroup v2, the line is `0::/` and the directory is `<sys>/fs/cgroup` itself; under `--cgroupns=host` it is the container's scope directory, for example `system.slice/docker-<id>.scope`
- With no `0::` line (cgroup v1) or no such directory, container mode stays on and every cgroup-sourced metric is unavailable; cgroup v1 files are not read
- A btop-web moved into a child cgroup of a private namespace (`0::/init` in Docker-in-Docker) scopes to that child only
- The network namespace is the host's when an interface in `<sys>/class/net` other than `lo` has a `device` link, which a bus-backed NIC has (PCI, vmbus, virtio, USB). veth, macvlan, ipvlan, tap and tun interfaces have none, so a container's own namespace passes. The check runs once at startup
- The startup log line names the result: `scope=container runtime=docker signal="1 /.dockerenv" cgroup=/sys/fs/cgroup network="own namespace"`, or `scope=host`
- No flag overrides detection

| Platform | Detected by | Wrong answer possible |
|---|---|---|
| Docker: any storage driver, the containerd image store, rootless, `--pid host`, `--network host`, `--cgroupns host` | 1; Docker creates `/.dockerenv` in every container | none: a shared pid namespace still lists only the cgroup's members, and the `device` check catches a shared network namespace |
| Podman: rootful, rootless, pasta, slirp4netns | 2, then 3 (`container=podman`) | none |
| Kubernetes: containerd, CRI-O | 5, then 6 under a host cgroup namespace, then 7 | only with all three absent, which the kubelet's `KUBERNETES_SERVICE_HOST` prevents |
| LXC, Incus, systemd-nspawn | 3 or 4 | missed when btop-web cannot read pid 1's environment and no other signal fires; the page then shows the host values with no CONTAINER chip |
| Bare `unshare` or `bwrap` sandbox | none | treated as a host |
| Plain host, WSL2 distribution | none fires | a false container only from a stray `/.dockerenv` or `/run/.containerenv` (a root filesystem imported from `docker export`) or `container=` in the service environment; the page then shows the CONTAINER chip and the values of btop-web's own cgroup, never a mix of the two |

- Run only in this Docker container: Docker Desktop, the containerd snapshotter, cgroup v2, a private cgroup namespace. The Podman, Kubernetes and LXC rows follow the runtimes' documented behaviour and were not run, and a host NIC's `device` link was not observed from inside the container

### 16.2 Source map

Every row is read every sample, `update_ms` (250 ms by default), unless it says otherwise. The cgroup files are in the cgroup directory of section 16.1. Withheld means the collector does not read the host source.

- Effective CPUs E = min(N, quota / period), read every sample, so a `docker update` shows at the next sample. N is the count of CPUs in `cpuset.cpus.effective`, or the online count when the file is missing; quota and period come from `cpu.max`, and `max` or a missing file means no quota
- Memory total T = MemTotal, and `memory.max` is not read (`DEF-VIEW-50-MEM-TERMS`, owner 2026-10-28): free memory exists only for the whole system, so total, free and other are the system's, and used and cache are the container's own

| Panel | Metric | Host source (btop) | Container source | Container formula | Shown in a container |
|---|---|---|---|---|---|
| Top bar | host | `gethostname` | the same call; the container's UTS namespace answers | - | the container's hostname |
| Top bar | CONTAINER chip | - | detection, once at startup | - | `CONTAINER`, P2 |
| cpu-stat, cpu-gauge | total % | `/proc/stat` aggregate | `cpu.stat` `usage_usec` | clamp(round(Δ`usage_usec` × 100 / (Δt in µs × E)), 0, 100), Δt from a monotonic clock; the first reading is stored, so the first published sample is a delta | value |
| cpu-gauge | CPU name | `/proc/cpuinfo` model name | the same | the processor's static identity, not a measurement | unchanged |
| cpu-gauge | core count | `/proc/stat` cpuN lines | E | - | `E cores` |
| cpu-gauge | user, system | `/proc/stat` aggregate | `cpu.stat` `user_usec`, `system_usec` | clamp(round(Δusec × 100 / (Δt in µs × E)), 0, 100), as the total | value |
| cpu-gauge | pressure | `/proc/pressure/cpu` `some avg10` | `cpu.pressure` `some avg10` | the kernel's two decimals; the host file is never read in a container | value |
| cpu-stat, cpu-gauge | load, load per core | `/proc/loadavg` | `cgroup.threads` of the container cgroup and every cgroup below it; the state field of `/proc/<tid>/stat`; every 15.004 s (three of the kernel's 5 s LOAD_FREQ periods plus its one extra tick of `5*HZ+1` at HZ = 250, so the count drifts across the phase of periodic tasks as the kernel's does; the owner chose 15 s on 2026-10-28 to cut the count's cost to a third), off the tick | the kernel's `calc_load` (`kernel/sched/loadavg.c`): active = the tids in state R or D, less the counting thread; avenrun = (avenrun × e + active × 2048 × (2048 − e)) / 2048 in FIXED_1 = 2048 fixed point, e = 2048 × exp(−15 s / 60 s), exp(−15/300), exp(−15/900) = 1595, 1948, 2014 for 1, 5 and 15 minutes (the kernel's rule for its EXP_1, EXP_5 and EXP_15, which gives 1884, 2014, 2037 at its 5 s), rounded as `/proc/loadavg` prints it. The first count, before the first tick, seeds all three averages; the kernel starts at 0 at boot, and the VM's history is not the container's. Load per core divides by E (`DEF-CPU-44-CONTAINER-LOAD`) | value |
| cpu-stat, cpu-gauge | frequency, temperature | policy0 `scaling_cur_freq`, `cpuinfo`, hwmon, thermal | none | withheld | not drawn |
| (snapshot, not drawn) | user, nice, system | `/proc/stat` | `cpu.stat` `user_usec`, `nice_usec`, `system_usec` | as the total | not drawn |
| cores | per-core % | `/proc/stat` cpuN | the same, for each CPU in `cpuset.cpus.effective` (every CPU when the file is missing) | btop's per-core formula; one row per cpuset CPU, named by its CPU number (`DEF-CPU-41-CONTAINER-CORES`) | value |
| mem-stat, mem-gauge | used | MemTotal − MemAvailable | `memory.current`, `memory.stat` | U = max(0, `memory.current` − R), R = `active_file` + `inactive_file` + `slab_reclaimable`, the kernel's MemAvailable terms taken from the cgroup | value |
| mem-stat, mem-gauge | total `of X`; mem-gauge reads `of total X` | MemTotal | MemTotal | T | value |
| mem-stat | available | MemAvailable | MemAvailable; under a `memory.max` limit free + cache | as named | value |
| mem-gauge | free + reclaimable | the free and reclaimable rows | the free, reclaimable and other reclaimable rows; under a `memory.max` limit the container free and reclaimable rows | the printed rows' sum; percent of T (`ACC-VIEW-195-MEM-SUM`) | value; `unavailable` when a part is unreadable |
| mem-gauge | reclaimable | MemAvailable − MemFree (btop-web) | `memory.stat` | R | value |
| mem-gauge | free | MemFree | MemFree | MemFree | value |
| mem-gauge | other reclaimable | none | MemAvailable, MemFree, `memory.stat` | MemAvailable − MemFree − (`inactive_file` + `active_file` + `slab_reclaimable`): the kernel counts all three of this container's as reclaimable; dropped under a `memory.max` limit | value |
| mem-gauge | other used | none | the other four rows | T − the other four rows, the remainder, so the rows sum to T exactly; dropped under a `memory.max` limit | value |
| mem-stat, mem-gauge | percentages | round(x × 100 / MemTotal) | - | round(x × 100 / T) | value |
| mem-gauge | ZFS ARC | arcstats | none: the ARC is not charged to a cgroup | not added | - |
| mem-gauge | swap used, total, free | SwapTotal, SwapFree | `memory.swap.current`, `memory.swap.max`, SwapTotal | used = `memory.swap.current`; total = min(`memory.swap.max`, SwapTotal), SwapTotal when `max`; free = total − used | value; `unavailable` when `memory.swap.current` is missing |
| storage | mount list, total, used, free | the mount table, `statvfs` | the same: the container's mount namespace and its `df` view | unchanged | value |
| storage | IO activity | `/sys/block/<dev>/stat` io_ticks | none | withheld | head `IO unavailable` |
| net | interfaces, rates, top, total, scale | `getifaddrs`, `<sys>/class/net/*/statistics` | the same; they answer for btop-web's own network namespace | unchanged | value |
| net | all, under the host's network namespace | as above | none | withheld: an empty snapshot | `network unavailable` |
| gpu | plot and legend | NVML, ROCm SMI, DRM sysfs | the same: the devices passed into the container (`NVIDIA_VISIBLE_DEVICES`) are its own | unchanged; the sweep runs as on a host (`DEF-GPU-40-CONTAINER-HIDDEN`) | value |
| proc | rows | every `/proc/<pid>` | `cgroup.procs` of the container cgroup and of every descendant cgroup, depth first | rows = the numeric `/proc` entries that are members; an unreadable `cgroup.procs` leaves the processes unavailable | rows |
| proc | Cpu% | btop's cpu_p over `/proc/stat` | the same counters | with `proc_per_core` false, cpu_p × online / E, online being the cpuN lines of the same `/proc/stat` read: a percent of the container's capacity. With `proc_per_core` true, unchanged | value |
| proc, proc-stat | other columns; total, running, threads; the Processes tile facts | `/proc/<pid>` and the rows | the member rows | btop's code, unchanged; the tile facts over the member rows (section 12.4) | value |
| proc-stat | `gpu` | NVML process lists | `cgroup.procs` of the container cgroup and every cgroup below it; `/proc/<pid>/fd/*` | the members, less btop-web, that hold `/dev/nvidia<N>`, `/dev/dxg`, `/dev/kfd` or `/dev/dri/renderD<N>` open; every 5 s in the off-tick GPU sweep (`DEF-VIEW-48-GPU-FACT`) | value; `n+` when some fd tables could not be read |

### 16.3 Unavailable in a container, and why

| Metric | Why no container source exists | On the page |
|---|---|---|
| Per-core usage, only on cgroup v1 or without a cgroup directory | no `cpuset.cpus.effective` names the container's CPUs | cores head `unavailable`, no core rows |
| CPU frequency, temperature | hardware state of the machine's cores and sensors | not drawn (`ACC-VIEW-23-RELEVANT`); named in the chip title |
| Disk IO activity | `io.stat` carries bytes and operations, no busy time | storage head `IO unavailable` |
| Network, only under the host's network namespace | the interfaces and counters are the host's | `network unavailable` |
| CPU, memory, swap, processes, GPU process count, load, only on cgroup v1 or without a cgroup directory | cgroup v1 files are not read | `unavailable`; the cpu-stat foot reads `load unavailable` |

- A withheld panel keeps its head with `unavailable` at the right end (`[data-withheld]`, the reason in `title`) and an empty body. `[data-panel]:has(> .ph [data-withheld])` narrows it to its head and gives its width to the band. Band 3 shrinks to one head row when the cores are withheld, no GPU is found and storage shows no mount, and at drop-p4 and below, where storage drops
- The CONTAINER chip is P2 and drops at drop-p2, where the top bar already drops the sample age. Its title reads `<runtime> container, detected by <signal>, cgroup <path>. Unavailable here: CPU frequency and temperature, disk IO activity`, with `network` added under the host's network namespace and CPU, memory, swap, processes, per-core usage and load added without a cgroup directory
- Every new word uses an existing token (`--muted`, `.meta`, the READ-ONLY chip tone), and no new text sits inside a `[data-chart]` box
- Disk capacity is the container's `df` view: the used figure covers every writer of that filesystem, because no source inside the container gives a per-container byte count
- Memory used applies the kernel's MemAvailable terms to the cgroup. On this host it gave 9.42 GiB, where the `docker stats` formula (`memory.current − inactive_file`) gave 105 GiB, more than the whole VM's btop used of 46.3 GiB, because that formula counts active page cache as used
- With `memory.swap.max` and `cpu.max` at `max` and `cpuset.cpus.effective` listing every CPU, the ceilings `/ 128 GiB` and `64 cores` equal the machine's sizes, as `of 503 GiB` (MemTotal) always does; the usage figures under them are the container's

### 16.4 Measured

Measured on 2026-10-27 in this Docker container (E = 64, no memory or CPU limit) by verify round 1 of workflow `btop-web-container-scope`, on the release binary:

| Check | Page | Container source | VM-wide source |
|---|---|---|---|
| CPU, mean over 20 s | 0.80 % | `cpu.stat` 0.66 % | `/proc/stat` 4.51 % |
| CPU rise, two busy loops for 15 s | 3.42 points | `cpu.stat` 3.21 points | expected 2 / 64 = 3.125 points |
| Memory used | 8.38 GiB | 8.386 GiB | btop used 41.3 GiB |
| Processes | 134 rows, 0 outside the cgroup | 134 in `cgroup.procs` | - |
| Threads | equal to `cgroup.threads` in every sample, 2,800-2,808 | `cgroup.threads` | about 12,000 kernel-wide in `/proc/loadavg` |
| Network eth0, mean down / up | 6,349 / 7,339 B/s | `/proc/net/dev` 6,309 / 7,287 B/s | - |

- Before `DEF-GPU-40` and `DEF-CPU-41` were fixed, load, cores, frequency, temperature, GPU and disk IO read `unavailable` or were not drawn, with no `0` in a withheld slot, while the pre-change binary drew the VM's load, 64 core rows and three GPUs at the same moment
- Live, the storage panel collapses in this container: `/etc/fstab` lists no mount and `use_fstab` defaults to true, as before the change. `IO unavailable` shows when mounts are listed, checked with `use_fstab = False`
- Layout at 1440x900, 1024x768, 768x1024, 390x844 and 844x390: no page scroll and no text over a `[data-chart]` box; the six new texts reach at least 4.52:1 in all ten theme variants
- Cost (`ACC-PERF-16-NO-OVERHEAD`), release binaries, one SSE client, 60 s windows: 6.66 ms of server CPU per tick against 7.95 ms for the pre-change binary (16 % less) and 18.5 % fewer SSE bytes per second, because no NVML sweep ran in a container before `DEF-GPU-40` was fixed
- On the host fixture tree the new binary logs `scope=host`, and the first 12 frames of each of the five SSE events are byte-identical to the pre-change binary's

## Appendix A: raw btop palette

Eight file-backed variants, 48 raw keys in btop's order. A value marked `*` was absent or empty in the theme file and comes from btop's own fallback rule (`meter_bg` and `graph_text` from `inactive_fg`; `process_*` from `cpu_*`; an empty gradient midpoint from the two-stop rule). The Default pair starts from btop's `Default_theme` (`src/btop_theme.cpp:49-92`) with the Steel overrides of section 3.3, which also lists its semantic values.

| Raw key | Gruvbox dark | Gruvbox light | Solarized dark | Solarized light | Everforest dark | Everforest light | Flexoki dark | Flexoki light |
|---|---|---|---|---|---|---|---|---|
| `main_bg` | #1d2021 | #fbf1c7 | #002b36 | #fdf6e3 | #2d353b | #fdf6e3 | #100F0F | #FFFCF0 |
| `main_fg` | #a89984 | #3c3836 | #eee8d5 | #586e75 | #d3c6aa | #5c6a72 | #CECDC3 | #343331 |
| `title` | #ebdbb2 | #3c3836 | #fdf6e3 | #002b36 | #d3c6aa | #5c6a72 | #FFFCF0 | #100F0F |
| `hi_fg` | #d79921 | #cc241d | #b58900 | #b58900 | #e67e80 | #df69ba | #DA702C | #BC5215 |
| `selected_bg` | #282828 | #f2e5bc | #073642 | #eee8d5 | #3d484d | #4F585E | #403E3C | #E6E4D9 |
| `selected_fg` | #fabd2f | #8f3f71 | #d6a200 | #b58900 | #dbbc7f | #dfa000 | #FFFCF0 | #100F0F |
| `inactive_fg` | #282828 | #ebdbb2 | #073642 | #eee8d5 | #2d353b | #9DA9A0 | #575653 | #B7B5AC |
| `graph_text` | #585858 | #a89984 | #073642 * | #eee8d5 * | #d3c6aa | #5c6a72 | #878580 | #6F6E69 |
| `meter_bg` | #282828 * | #ebdbb2 * | #073642 * | #eee8d5 * | #2d353b * | #9DA9A0 * | #343331 | #DAD8CE |
| `proc_misc` | #98971a | #98971a | #bad600 | #d33682 | #a7c080 | #8da101 | #879A39 | #66800B |
| `cpu_box` | #a89984 | #a89984 | #586e75 | #93a1a1 | #3d484d | #4F585E | #4385BE | #205EA6 |
| `mem_box` | #a89984 | #a89984 | #586e75 | #93a1a1 | #3d484d | #4F585E | #879A39 | #66800B |
| `net_box` | #a89984 | #a89984 | #586e75 | #93a1a1 | #3d484d | #4F585E | #CE5D97 | #A02F6F |
| `proc_box` | #a89984 | #a89984 | #586e75 | #93a1a1 | #3d484d | #4F585E | #3AA99F | #24837B |
| `div_line` | #a89984 | #a89984 | #586e75 | #93a1a1 | #3d484d | #4F585E | #403E3C | #CECDC3 |
| `temp_start` | #458588 | #98971a | #268bd2 | #268bd2 | #a7c080 | #8da101 | #4385BE | #205EA6 |
| `temp_mid` | #d3869b | #5474e8 * | #ccb5f7 | #ccb5f7 | #dbbc7f | #dfa000 | #8B7EC8 | #5E409D |
| `temp_end` | #fb4394 | #cc241d | #fc5378 | #fc5378 | #f85552 | #f85552 | #D14D41 | #AF3029 |
| `cpu_start` | #b8bb26 | #427b58 | #adc700 | #adc700 | #a7c080 | #8da101 | #3AA99F | #24837B |
| `cpu_mid` | #d79921 | #d79921 | #d6a200 | #d6a200 | #dbbc7f | #dfa000 | #D0A215 | #AD8301 |
| `cpu_end` | #fb4934 | #cc241d | #e65317 | #e65317 | #f85552 | #f85552 | #D14D41 | #AF3029 |
| `free_start` | #4e5900 | #cc241d | #4e5900 | #4e5900 | #f85552 | #f85552 | #343331 | #DAD8CE |
| `free_mid` | #b5e685 * | #d79921 | #b5e685 * | #b5e685 * | #dbbc7f | #dfa000 | #575653 | #B7B5AC |
| `free_end` | #98971a | #427b58 | #bad600 | #bad600 | #a7c080 | #8da101 | #879A39 | #66800B |
| `cached_start` | #458588 | #458588 | #114061 | #114061 | #7fbbb3 | #3994c5 | #205EA6 | #4385BE |
| `cached_mid` | #74e6fc * | #076678 | #74e6fc * | #74e6fc * | #83c092 | #35a77c | #4385BE | #205EA6 |
| `cached_end` | #83a598 | #427b58 | #268bd2 | #268bd2 | #a7c080 | #8da101 | #3AA99F | #24837B |
| `available_start` | #d79921 | #cc241d | #705500 | #705500 | #f85552 | #f85552 | #AD8301 | #D0A215 |
| `available_mid` | #ffd77a * | #d65d0e | #ffd77a * | #ffd77a * | #dbbc7f | #dfa000 | #D0A215 | #AD8301 |
| `available_end` | #fabd2f | #b57614 | #edb400 | #edb400 | #a7c080 | #8da101 | #DA702C | #BC5215 |
| `used_start` | #cc241d | #427b58 | #6e1718 | #6e1718 | #a7c080 | #8da101 | #879A39 | #66800B |
| `used_mid` | #d9626d * | #d65d0e | #d9626d * | #d9626d * | #dbbc7f | #dfa000 | #DA702C | #BC5215 |
| `used_end` | #fb4934 | #cc241d | #e02f30 | #e02f30 | #f85552 | #f85552 | #D14D41 | #AF3029 |
| `download_start` | #3d4070 | #98971a | #3d4070 | #3d4070 | #a7c080 | #8da101 | #205EA6 | #4385BE |
| `download_mid` | #6c71c4 | #689d6a | #6c71c4 | #6c71c4 | #83c092 | #35a77c | #4385BE | #205EA6 |
| `download_end` | #a3a8f7 | #79740e | #a3a8f7 | #a3a8f7 | #7fbbb3 | #3994c5 | #3AA99F | #24837B |
| `upload_start` | #701c45 | #cc241d | #701c45 | #701c45 | #dbbc7f | #dfa000 | #A02F6F | #CE5D97 |
| `upload_mid` | #b16286 | #d65d0e | #d33682 | #d33682 | #e69875 | #e66868 | #CE5D97 | #A02F6F |
| `upload_end` | #d3869b | #b57614 | #f56caf | #f56caf | #e67e80 | #df69ba | #8B7EC8 | #5E409D |
| `process_start` | #b8bb26 * | #427b58 | #adc700 * | #adc700 * | #a7c080 | #8da101 | #879A39 | #66800B |
| `process_mid` | #d79921 * | #af3a03 | #d6a200 * | #d6a200 * | #e67e80 | #df69ba | #D0A215 | #AD8301 |
| `process_end` | #fb4934 * | #cc241d | #e65317 * | #e65317 * | #f85552 | #f85552 | #D14D41 | #AF3029 |
| `proc_pause_bg` | - * | - * | - * | - * | - * | - * | #D14D41 | #AF3029 |
| `proc_follow_bg` | - * | - * | - * | - * | - * | - * | #4385BE | #205EA6 |
| `proc_banner_bg` | - * | - * | - * | - * | - * | - * | #8B7EC8 | #5E409D |
| `proc_banner_fg` | - * | - * | - * | - * | - * | - * | #FFFCF0 | #FFFCF0 |
| `followed_bg` | - * | - * | - * | - * | - * | - * | #4385BE | #205EA6 |
| `followed_fg` | - * | - * | - * | - * | - * | - * | #FFFCF0 | #FFFCF0 |

## Appendix B: mock data provenance

Historical: the mock data of the earlier proof `docs/design/index.html`. The shipped page renders live collector data.

- Host: `lab-workstation`; Linux 6.18.54.1-microsoft-standard-WSL2 x86_64; container view of a WSL2 VM; uptime 3d 12h 39m at the 03:16Z sample.
- CPU: AMD Ryzen Threadripper PRO 7975WX 32-Cores, 64 logical CPUs, SMT 2; total 4.80%; per-core values from `prep.json` (64 values, mean 4.61, max 49.0), rendered as integers per section 12.2; frequency 3994.064 MHz from `/proc/cpuinfo`; load 84.55, 85.26, 87.32 from `/proc/loadavg`. The six-way breakdown (user 2.1, system 1.4, iowait 0.4, irq 0.0, softirq 0.9, steal 0.0, idle 95.2) is an illustrative distribution that sums to the measured busy share; `prep.json` records only the aggregate and per-core deltas.
- Memory: total 503 GiB; used 33.6 GiB / 7%; available 469 GiB / 93%; cached 280 GiB / 56%; free 161 GiB / 32%; swap 455 MiB / 0.3% of 128 GiB.
- Disks: `/home` ext4 on `/dev/sdd`; `/dev/shm` tmpfs; `/mnt/nas-shared` nfs; `/mnt/personal/onedrive-konrad-jelen-gmail-com` nfs. The four 1 PiB NFS placeholders in `prep.json` are excluded.
- Network: `eth0` 172.24.0.0/16, 1.6 MiB/s down, 94 KiB/s up; `eth1` 10.230.6.0/24, idle; `lo` 127.0.0.1, idle. Top and total values are from `prep.json`; the 120-sample histories are deterministic mock series that end on the sampled current rate.
- Processes: 201 processes, 7,860 threads; 12 mock rows from `prep.json`; all users are `lab` except `systemd` and `containerd` as `root`. The sparkline histories are five-point mock histories derived from each row's sampled CPU value, except PID 1158269, which carries a one-interval 1.4% spike so the proof shows the lazy/direct difference. `ps %CPU` in `prep.json` is a lifetime average and is not the table value; the table uses btop's interval share of all 64 CPUs. The lazy sort uses those lifetime averages as btop's `cpu_c`; the direct sort uses the interval share as `cpu_p`.
- GPU: three NVML devices: NVIDIA RTX PRO 4000 Blackwell (24,467 MiB, 0 MiB used, 0%, 36 °C, 3.9 W of 145 W), NVIDIA RTX PRO 6000 Blackwell Max-Q (97,887 MiB, 64 MiB used, 0%, 37 °C, 14.1 W of 300 W) and NVIDIA RTX PRO 5000 72GB Blackwell (73,415 MiB, 853 MiB used, 1%, 37 °C, 21.9 W of 300 W). No running compute processes at the sample.
- Battery: unavailable; `/sys/class/power_supply` is empty.
- Temperature: unavailable; no `/sys/class/hwmon`, no `/sys/class/thermal/thermal_zone*`.

## Appendix C: open decisions

Historical: the decisions recorded for the earlier proof `docs/design/index.html`. Open work is tracked in `docs/acc-crit.md` and `docs/defects.md`.

| Id | Decision |
|---|---|
| Q1 | Ten variants. The picker has five families; eight variants are file-backed and Default is generated. |
| Q2 | Show 3.99 GHz in the design; btop's short form is 4.0 GHz. The parity test decides the production format. |
| Q3 | Integer percent by default; one decimal only when a non-zero value would round to 0 or 100. |
| Q4 | Show the statvfs mounts the collector reads; decide `use_fstab` versus `/proc/mounts` in the collector and document it. |
| Q5 | Show all three NVML devices by default; `Unavailable` only when NVML reports no device at all. |
| Q6 | No priority route in the first release; keep the read-only guard and the disabled control as preventative surface. |
| Q7 | The process drawer ships read-only in the first release. |
| Q8 | Pin btop main at `87bb1610` for the theme files; 1.3.0 lacks four of the eight. |
| Q9 | Reproduce btop's derived fallbacks; do not fill every missing key from Default. |
| Q10 | Cap rendered process rows at 200 or virtualize; the proof renders 12. |
| Q11 | Keep server-rendered SSE and add idiomorph; no SPA. |
| Q12 | Verify 320 px reflow as well as the 390 px design case. |
| Q13 | Read-only defaults true; the first release exposes no mutating route. |


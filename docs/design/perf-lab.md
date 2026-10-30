# btop-web - Chart rendering lab v1

Status: DRAFT for owner review, 2026-10-29.

## 1. Overview

This note ranks three ways to paint the dashboard's ten charts by measured renderer CPU per 60 s, then by bytes, maintenance and fit with the server-rendered page; it names the winner, the runner-up and the migration cost of each.

The lab page reproduces the ten chart boxes at the dashboard's measured 1440x900 positions, feeds one identical 600-sample series set to every mode, advances one sample every 250 ms and glides the series between readouts with the production formula from `app.js`. SVG and canvas consume the server's PCHIP path strings, precomputed at load; uPlot consumes the raw sample arrays. Each mode ran three times for 60 s after a 2 s settle, in a rotating order, in headless Chromium 156 with SwiftShader software raster; the winning mode then ran once more after the others.

## 2. Measurements

This section lists the measured cost of each mode.

| Approach | Renderer CPU, s/60 s (runs) | CPU mean | Main-thread task, s/60 s (mean) | FPS (dropped) | Bytes loaded (requests) | Data payload |
|---|---|---:|---:|---|---:|---|
| Promoted SVG | 6.15, 5.55, 5.90, 5.82 | 5.86 | 2.64 | 60.0 (0) | 10,787 (1) | no |
| Canvas 2D from the server path strings | 6.11, 5.72, 5.39 | 5.74 | 2.57 | 60.0 (0) | 10,787 (1) | no |
| uPlot from raw sample arrays | 6.47, 6.04, 5.80 | 6.10 | 2.99 | 60.0 (0) | 64,107 (3) | yes |
| Whole dashboard, ACC-PERF-133 (context) | 33.12 | - | 14.70 | 59.7 (0) | event stream only | no |

Within-mode spread was 0.60 s/60 s for SVG, 0.72 for canvas and 0.67 for uPlot, at host load of 22 to 26. Every run held 60 fps with a 16.7 ms median frame gap and no dropped frame. The baseline row paints the whole dashboard, including the process table, gauges and htmx swaps; the lab page paints only the ten charts, so the two rows are not directly comparable.

## 3. Ranking

This section ranks the three modes on renderer CPU, then on bytes, maintenance and fit.

| Rank | Approach | Renderer CPU per 60 s | Bytes | Maintenance and fit |
|---:|---|---|---:|---|
| 1 | Promoted SVG | Inside the noise band with canvas; no separation | 10,787 | One CSS declaration; no new code, payload or dependency; keeps the server-built path strings |
| 2 | Canvas 2D from the server path strings | Nominally lowest mean, inside the noise band | 10,787 | A new client draw module of roughly 60 to 100 lines; keeps the path strings and adds no payload |
| 3 | uPlot from raw sample arrays | Highest mean; higher than both only on main-thread task time | 64,107 | A vendored library and client-side history; needs a data payload, which the no-JSON-API rule forbids |

On the means alone canvas is first, SVG second and uPlot third. The gaps between the CPU means are 0.12 and 0.36 s/60 s, below the within-mode spread, so total renderer CPU does not separate the three modes. uPlot separates only on main-thread task time, where every uPlot run is higher than every SVG and canvas run. Bytes then tie SVG with canvas, and maintenance and fit decide for SVG.

## 4. Recommendation

This section names the winner, the runner-up and the third mode, with the migration cost of each.

### 4.1 Winner: promoted SVG

Keep the server-built SVG and promote the scrolling series group to its own compositor layer.

- Files changed: `crates/btop-web/assets/app.css` only. Add `will-change: transform` to the existing `.chart .series, .chart .gpu-series` rule; the `.chart .rescale` rule selects the same group and inherits the promotion.
- Data payload: none. Bytes added: 0.
- Gain: the per-frame path raster leaves the renderer. The survey attributes 7.31 s/60 s to RasterTask on the document layer and projects about 21 s/60 s on the real page, range 15 to 23, against the 33.12 s/60 s baseline; that projection needs one run of the real page after the change.
- Give up: one compositor layer per chart stays allocated while the page is open, including while motion is PAUSED; releasing it needs a motion class toggled in `app.js`, a second optional change. A fractional layer translate can resample the 1.5 px stroke, so verify the line at DPR 1 and DPR 2 on the real page.
- Variant: promote the SVG root and translate `.chart` instead. The 8-chart micro page measured 5.25 s/60 s against 5.70 for the series group, at the cost of one selector change in `app.js`.

### 4.2 Runner-up: canvas 2D from the server path strings

Draw the same server path strings into a canvas through `Path2D` and translate the canvas element between redraws.

- Files changed: `crates/btop-web/templates/partials/cpu_stat.html`, `mem_stat.html`, `proc_stat.html`, `net.html`, `gpu.html` and the process-row sparkline markup change the chart body from `<svg><g><path>` to `<canvas>` with the path strings as data attributes; `crates/btop-web/assets/app.js` gains a draw module for `Path2D`, device pixel ratio, resize, theme colours read through `getComputedStyle`, the 4 Hz redraw and the `--shift` translate; `crates/btop-web/assets/app.css` gains the canvas rules. The server and the collectors do not change.
- Data payload: none. The same path strings use the existing SSE fragments.
- Gain: two fewer DOM path nodes per chart. The nominal 0.12 s/60 s CPU advantage is inside the noise band.
- Give up: the vector chart becomes a bitmap, so fractional DPR scaling blurs unless the canvas is redrawn at the device pixel ratio; the draw module adds roughly 60 to 100 lines where the winner adds none. On the 8-chart micro page canvas measured 6.37 to 6.77 s/60 s against 5.70 for promoted SVG.

### 4.3 Third: uPlot from raw sample arrays

Vendor uPlot and let it draw each chart from a raw sample array.

- Files changed: `uPlot.iife.min.js` (51,276 bytes) and `uPlot.min.css` (2,044 bytes) are vendored under `crates/btop-web/assets/`; the chart templates gain a host element; `app.js` keeps a 600-sample history per chart and calls `setData` at 4 Hz; `crates/btop-web/src/chart.rs` and the templates emit raw sample arrays in place of, or beside, the path strings.
- Data payload: yes. The event stream carries sample arrays, which is a design change under the no-JSON-API rule and needs the owner's decision.
- Gain: the library owns the axes, the scales and the redraw.
- Give up: 53,320 extra bytes, 5.9 times the page, a third-party dependency with a single maintainer, and the existing server-built path code. Measured no CPU gain: the highest mean and the highest main-thread task time of the three.

## 5. Sanity check

The winner was measured once more after every other run: 5.82 s/60 s renderer CPU, 2.61 s/60 s main-thread task, 60.01 fps, no dropped frame, 10,787 bytes, host load 23.5 at the start and 23.2 at the end. The value falls inside the earlier SVG range and 0.08 s/60 s higher than the canvas mean. The ranking does not flip: SVG and canvas remain inside the noise band on renderer CPU, and promoted SVG keeps the bytes, maintenance and fit advantage.

## 6. Limits

This section lists what the numbers do not cover.

- All numbers are headless Chromium 156 with ANGLE Vulkan SwiftShader software raster at 1440x900 and DPR 1. Compositing and raster cost more on this path than on the owner's GPU browser; the order should hold and the saving there may be larger.
- The lab page paints ten charts only, so its numbers rank the modes rather than re-measure the page. The projected real-page figure needs one run of the real page after the CSS change.
- Within-mode spread of 0.60 to 0.72 s/60 s comes from host load of about 22 to 26; a difference below that band is not separable. Only uPlot's main-thread task time separates from the other two.
- The uPlot row measures the data-payload design change, so its byte count includes the vendored library, not only the event stream.

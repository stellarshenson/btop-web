# Layout rules

The dashboard is one fixed viewport: a top bar, four bands and twelve column weights. This document is the layout authority for a later change. The owner's decisions of 2026-10-29 come first, the trackers second, the code third. Numbers are the CSS tokens in `crates/btop-web/assets/app.css` evaluated at the stated viewport, or measurements in the evidence files below; numbers marked derived come from those tokens.

## 1. Column grid

- Twelve columns; a band is a flex line of panels with flex-basis 0 and flex-grow weight, so a panel takes `weight / sum(visible weights)` of the band's content width
- Weights per band: 4:4:4 band 1, 4:4:4 band 2, 5:4:3 band 3, 12 band 4 (ACC-LAYOUT-157)
- Band 2 is 4:4:4 from 2026-10-29, so Cores, the CPU gauge and the Memory gauge take a third each and the two gauges line up (DEF-LAYOUT-88, ACC-VIEW-163)
- Panel width = `(page width - 2 * --gap - (n - 1) * --gap) * weight / sum(visible weights)`, with `--gap` 8 at 1440, 6.25 at 1024 and 5 at 390; at 1440x900 band 3 with Storage absent is 5:4 of the 1416 px content width, 786.67 and 629.33 px, and 558.47 and 446.78 at 1024x768
- A panel size factor multiplies its width weight: s 0.75, m 1, l 1.5; the band height factor is the largest size factor among the band's shown panels
- A hidden panel leaves the line and the band re-splits by the remaining weights: two weight-4 panels take half each; a band with every panel hidden is hidden and its height share goes to the other bands
- A moved panel takes the destination slot's weight: the slot carries the width, the panel keeps its content, size and controls (ACC-LAYOUT-157, ACC-LAYOUT-158)
- A drag swaps two order positions; band membership follows the order in groups of 3, 3, 3, 1 (theme.js BAND_SIZES)
- A panel with nothing to show (no GPU, no mount) leaves the line and the band re-splits; a withheld container panel narrows to its header and gives the rest of its width to the band (ACC-VIEW-23)
- Tiers key on the viewport, never on a panel's measured width: P4 at width <= 1100 or height <= 640, P3 at width <= 899 or height <= 640, P2 at width <= 599 or height <= 540 (ACC-LAYOUT-46)
- Full-tier widths at 1440x900: band 1 and band 2 469.33 each; band 3 586.67 / 469.33 / 352.00; band 4 1424.00
- With Storage absent, the shipped 1440x900 page draws band 3 at 5:4: GPU 786.67 and Network 629.33
- P4 widths at 1024x768: band 1 and band 2 333.00 each; band 3 558.47 / 446.78 at 5:4, because Storage drops
- Full-tier widths at 1920x1080: band 1 and band 2 629.33 each; band 3 786.67 / 629.33 / 472.00 with Storage, 1053.33 / 842.67 without (derived)
- The narrowest panel kept while shown is 260.00 px derived and 260.22 px measured, the Network panel in the 5:4 band 3 at 600x700; at the P4 threshold the Storage slot is 268.4 px when it drops
- Below 599 px wide the page is one column and every panel is full width: 589 px at 599 and 380 px at 390
- The process table keeps its own column floors: Program 150 px or the tree prefix width, Pid 7ch + 16 px below 900, the filter 92 to 132 px; these are cell floors, not panel floors

## 2. Band heights

- The page column is the top bar `--bar-h` (26 at 1440 and 1920, 24.25 at 1024, 22 at 390), page padding `--gap`, three `--gap` gaps and four bands; the page never scrolls
- A band has flex-basis 0, flex-grow = `share * band factor` and min-height = `max(band minimum, GPU minimum)`
- Shares are 116 : 190 : 214 : 314; they sum to 834, the height available at 1440x900 after the 26 px bar, 16 px page padding and three 8 px gaps
- Band minimums: band 1 `head + tile + foot + 22` = 108.00 at 1440; band 2 `max(head + 17 + 2*(name + value + 3*meta + 4) + 12, head + 41 + 4*(name + fs12*1.25 + 1) + meta)` = 192.25 at 1440 and 180.68 at 1024; band 3 `head + 112` = 138.00; band 4 120
- A GPU band adds `head + 25 + 18 * lanes`: 25 px for the 2 px border, the 9 px body padding and the 14 px axis; 18 px for a 14 px lane and its 4 px gap
- A share below the minimum freezes that band at the minimum and the other bands split the remainder by their shares; at 1440x900 band 2 freezes at 192.25 and the heights are 115.59 / 192.25 / 213.25 / 312.90
- At 1024x768 band 2 freezes at 180.68 and band 1 at 96.75: heights 96.75 / 180.68 / 176.33 / 258.73 (derived)
- At 1920x1080 every share clears its minimum: 141.04 / 231.01 / 260.19 / 381.77 (derived)
- At 390x844 band 2 is hidden and the visible bands get 162.11 / 319.95 / 319.95 (derived)
- When content needs more height than its share, the fixed parts keep their heights (header, stat tile, axis, fact rows) and the flexible element gives height first: the plot strip, the gauge, a core row or the storage mount spacing
- The process table is the only scroller; it scrolls inside band 4
- At drop-p3 the minima lower: band 1 `head + tile + 16`, band 2 `max(head + 17 + 2*(name + value + 1) + 12, head + 112)`, band 3 `head + 17 + 3*(name + value + 1) + 16`, band 4 120

## 3. Panel internals

- Panel: 1 px border, 2 px radius, overflow hidden; the content box is the border box less 2 px
- Header: `flex: 0 0 --head-h`, padding 0 8, gap 8, 1 px bottom border; `--head-h = clamp(22px, 12.404px + E * 0.009615, 26px)` = 26 at 1440 and 1920, 22.25 at 1024, 22 at 390, with `E = min(100vw, 160vh)`
- Body padding: gauge 6 top / 10 sides / 8 bottom; network and GPU 5 / 8 / 4; cores 4 / 8; storage 2 / 9; stat 0, with the stat tile text inset 10 px and the plot spanning the panel; process cells 0 8 and header cells 1 8 0
- Plot to side column: 10 px; the legend adds a 1 px left border and 10 px left padding; the gauge to its side block 10 px
- Plot scale gutter: 27 px plus a 10 px gap to the plot box; the labels right-align on the gutter (DEF-GRAPH-63)
- Time axis 14 px; GPU lanes 4 px apart with a 1 px rule centred in the gap; the GPU legend 224 px wide (192 at 599 and below) and its mini bars reserve 48 px
- The process filter is `clamp(92px, 63.019px + E * 0.048077, 132px)` = 132 at 1440, 112.25 at 1024, 92 at 390

## 4. Gauge rule

- One token sizes both gauges: `--gauge-w = clamp(96px, 9.2vw, 132px)`, the design width; 132 at 1440 and 1920, 96 at 1024 and 390
- Circle: `width = min(body content height * 112/114, content width - 10 - --gauge-side-w)`, aspect 112:114
- Squircle: height = body content height, width = height * 112/122
- `--gauge-side-w = 18.1 * --fs-10` = 181.00 at 1440, 167.42 at 1024, 162.90 at 390
- Circle when the gauge body's content height is at least `--gauge-w * 114/112`: 134.36 px at 1440 and 97.71 px at 1024; one shape for both gauges, taken from the shorter body, decided in CSS before first paint (DEF-LAYOUT-90)
- Short view (height <= 640, DEF-LAYOUT-94): drop-p3 removes the side column and the gauge is the body's only content, while the body is several times wider than tall. The gauge then shows a third server shape, `g-wide`: the squircle band cropped to its geometry box plus a stroke margin (`GAUGE_WIDE_VIEWBOX` = `13 19.7 82 86.3` in the 112 x 122 space) and stretched to the body's content box with `preserveAspectRatio="none"`, so the band's aspect is the box's - 609.09:96 = 6.34:1 at 1280x600, 481.09:96 = 5.01:1 at 1024x600, 689.09:96 = 7.18:1 at 1440x600
- Short view strokes: `vector-effect: non-scaling-stroke` keeps the 8 px band at device width, so the band's ink clears the body content edge by about 3 px at 1280x600 (3.43 / 3.01 / 3.43 / 3.08) and never exceeds 7.33 px at 1920x600; the circle and squircle SVGs are hidden in this state, so both gauges show one shape whatever a stored panel size does
- Short view readout: the HTML `.g-read` over the hollow at the design type sizes (16 / 9 px), centred on the band's centre, half the width and 53.65 percent of the height (54 and 66 of the wide viewBox), because a stretched SVG glyph is unreadable. The SVG readout, the ticks and the scale labels are not drawn in this shape; the scale labels are a P3 drop already
- The bands and the dropped fields are unchanged by the short-view shape: it is a geometry state of the gauge, not a tier
- Measured circles: 147.61 x 150.23 at 1440x900, where the body height decides over a 150.25 px body; 133.58 x 135.95 at 1024x768, where the side column decides over a 311.00 x 142.44 px body; the smallest circle is 97.58 x 99.31 at 900x700
- Measured short-view shapes: 609.09 x 96.00 at 1280x600, 481.09 x 96.00 at 1024x600 and 689.09 x 96.00 at 1440x600, each in a 96.00 px content-height body; the two panels are 631.09, 503.09 and 711.09 wide there, so both gauges take the same box
- The squircle stays the shape for a body under the circle threshold outside the short-view state (a P3-by-width page just under the threshold, 97.0 px at 700x641, or a stored s/l size)
- Both gauges take one size at the default 1:1:1 and at every tier; at 390x844 both are hidden by P2
- Text fits its boundary box (ACC-VIEW-162): value box 60 x 23 SVG units at (24, 49), label box 68 x 23 at (20, 59), scale label 6.24 x 9.4 per character (0.78 em at font 8); design sizes 16 / 9 / 8 px, floors 10 / 7 / 6 px; a text steps down 1 px until it fits and steps back up only with 0.5 units of slack
- The design readout 23.8 GiB fits the value box at 16 px in both faces; a value as wide as 1234.5 GiB does not fit at the design size and steps down
- A stored s/l panel size changes a panel's width and the band height factor, so the width limit can differ between the two gauges at 1024x768; the equality rule holds at the default 1:1:1 (open item)

## 5. Side columns

- The gauge side takes its natural width and never stretches: 155.45 px CPU facts and 163.31 px Memory rows at 1440x900; 139.17 and 141.06 at 1024x768; 0 at 390, where P2 hides the gauges
- The Memory legend's widest row is `other reclaimable` in a container with no limit and `free + reclaimable` under a limit or on a host; the first measures 163.31 px at 1440x900 in the system face, inside the 21.3 em `--gauge-side-w`
- The faces change the natural width: the Memory rows measure 163.31 px in the system face and 171.06 px under GalaxaTron Narrow at 1440x900, 141.06 and 149.38 at 1024x768; the gauge box is the same in both faces, 184.41 x 159.25 at 1440x900 and 141.59 x 122.28 at 1024x768, because the body height decides at 1440 and the face-independent `--gauge-side-w` token at 1024
- The gauge leaves `--gauge-side-w` beside it: 181.00 / 167.42 / 162.90; the gauge shrinks first and the side keeps its content
- The network legend is a fixed token: `--legend-w = clamp(112px, 43.635px + E * 0.086538, 168px)` = 168 at 1440, 132.25 at 1024, 112 at 390; the plots take the rest with `minmax(0, 1fr)`
- The GPU legend is 224 px (192 px at 599 and below); each lane is scale 27 / plot 1fr / legend 224 with 10 px column gaps
- Alignment: every Memory row starts at one label edge, 0 px spread; every percent cell ends at the content box's right edge, 0 px spread; the free + reclaimable row's value and percent sit in the same two columns as the other rows; facts stack label over value, left aligned
- A side column never wraps under the plot; the plot column is `minmax(0, 1fr)` and gives width first, then the tier drops the side: P3 removes the gauge facts, the gauge scale and the legend meta and model, P2 makes the GPU legend one line per entry and removes the scale gutter
- The load triple is the one value that wraps: each figure with its window is a nowrap cell and the cells wrap as units (ACC-VIEW-129/130/131)
- The Network legend fills its cell height: its entries sit within 12 px of the cell's top and bottom edges, never centred with an empty block under the last entry (DEF-LAYOUT-93)

## 6. Dead space

- Dead space is an empty region larger than 12 px at any content-box edge, an empty band, or an empty block under the last item of a side column
- A gap bounded by content on both sides is not dead space: the gauge-to-side gap (120.86 px at 1440x900) and the free width between a stat value and its facts are the flexible part of the line
- Every panel spans its content box horizontally: the gauge touches the left edge and the side block the right edge, 0 px on both sides at 1440x900 and 1024x768
- Every panel covers its content box vertically within 12 px; the gauge fill is 0.02 px short at 1440x900 and 6.49 px short at 1024x768 (3.25 px per side) and meets a tighter 8 px check
- The accepted gauge side leaves 10.1 px above and below its 130 px of rows in a 150.25 px body, inside the 12 px tolerance
- Without a side column the gauge fills the body: in the short-view state the wide band spans the body's content box with about 3 px of stroke clearance (section 4), so the gauge panel has no empty side in any measured panel at 1280x600 and 1024x600; at a P3-by-width viewport with a body taller than the design threshold it keeps the circle and takes the whole body height, `width = 100cqh * 112/114`
- Which element grows first depends on the panel: the plot strip in a stat, network or GPU panel; the gauge in a gauge panel; a core row in Cores; the storage mount spacing in Storage; the header, tile, axis and fact rows keep their heights
- When a panel has free width the gauge takes it up to its height limit and the flexible gap takes the rest; the side block does not stretch (DEF-LAYOUT-89)
- An empty band is hidden and its share goes to the other bands; a panel with no data is hidden (ACC-VIEW-23)
- The Network legend fills its cell: the two entries grow to its top and bottom edges, 0 px at 1440x900, 1024x768, 1920x1080 and 390x844 (DEF-LAYOUT-93)
- The rejected shape of this rule is the stretched Memory legend at 305.33 px and the 132 px gauge it held; the owner rejected both (DEF-LAYOUT-89)

## 7. Text

- Two text tokens: `--font-ui` for body, labels, controls and table cells; `--font-display` for panel headings and stat values; both resolve to the system stack by default
- The FONT control's GalaxaTron option points both tokens at `--font-narrow`, GalaxaLabsTron Narrow, the 85 percent width derivative of GalaxaLabsTron; its digits advance 0.655 em against the system stack's 0.636 em
- The process Program and Command cells and the config popover keep the system stack under GalaxaTron (DEF-THEME-78, ACC-THEME-155)
- Type steps on `E = min(100vw, 160vh)`: `--fs-12` 11 to 12, `--fs-11` 10 to 11, `--fs-10` 9 to 10, `--fs-9` 8 to 9 px; `--stat-size` 22 to 32, `--lg-v` 12 to 14; body 12 px at line-height 1.25 with tabular figures
- Tracking: uppercase chips 0.03em, legend names and gauge labels 0.05em, config labels 0.04em, panel headings and brand 0.01em; large stat values -0.015em and gauge values -0.02em; units and the narrow-face exception 0
- Reserved width (ACC-LAYOUT-74): every live value is an inline block whose hidden `::before` carries its widest form in `data-w`; digits are tabular and right-aligned in the reserve, so the empty part falls before the number and the unit or the next word keeps its x
- Reserves in the page: core value `data-w="100"` 15.28 px, memory percent `data-w="100%"` 23.64 px at 1440 and 21.67 at 1024, memory value `data-w="0000 MiB"` 38.77 and 35.55, the scale gutter 27 px, the network legend 168 px, the GPU legend 224 px, the Pid cell 7ch + 16 px below 900
- The stat tooltip opens after 1500 ms of rest on one stat; a move to a different stat restarts the wait and staying on the same stat does not (ACC-VIEW-168)
- The tooltip sits 12 px right and 16 px below the pointer, flips at the viewport edge and stays 4 px inside it

## 8. Adversarial sizes

| Size | E | --gap | --head-h | --fs-10 | --gauge-w | --legend-w |
|---|---|---|---|---|---|---|
| 1920x1080 | 1728 | 8.00 | 26.00 | 10.00 | 132 | 168.00 |
| 1440x900 | 1440 | 8.00 | 26.00 | 10.00 | 132 | 168.00 |
| 1280x720 | 1152 | 6.87 | 23.48 | 9.56 | 117.76 | 143.33 |
| 1024x768 | 1024 | 6.25 | 22.25 | 9.25 | 96 | 132.25 |
| 1280x600 | 960 | 5.94 | 22.00 | 9.10 | 117.76 | 126.71 |
| 1024x600 | 960 | 5.94 | 22.00 | 9.10 | 96 | 126.71 |
| 390x844 | 390 | 5.00 | 22.00 | 9.00 | 96 | 112.00 |

- 1920x1080: full tier, every clamp at its maximum; bands 141.04 / 231.01 / 260.19 / 381.77; band 1 and band 2 panels 629.33; band 3 786.67 / 629.33 / 472.00, or 1053.33 / 842.67 when Storage is absent; gauge circle 185.66 x 189.01; nothing drops
- 1440x900: full tier; bands 115.59 / 192.25 / 213.25 / 312.90; band 1 and band 2 panels 469.33; band 3 786.67 / 629.33 with Storage absent, the shipped host; gauges 147.61 x 150.23; no page scroll
- 1280x720: full tier (width 1280 > 1100 and height 720 > 640); bands 100.44 / 185.17 / 152.08 / 223.16; band 2 panels 417.52; gauges circle 143.14 x 145.69; the largest edge gap is at most 0.08 px in every panel; no page scroll; the Network legend cell is 113.61 px and the two five-row entries are 64.23 px each, 140.46 px together against 113.61 px, 13.43 px cut at each end before the step; the height step (DEF-LAYOUT-99) at a panel under 185 px sets the gap to 6 px, drops the 2 px vertical padding and the row gaps and tightens the line-heights (name 1.1, value 1, meta 1.1), so each entry carries 53.45 px of rows in a 55.80 px box with 0 px at both cell edges and 6 px between the entries; below 150 px, at the shortest full-tier viewports, the value steps to 12 px and the meta to 8 px instead of dropping a fact
- 1024x768: P4 drops host, clock and Storage; head 22.25, gap 6.25, fs-10 9.25, legend 132.25; bands 96.75 / 180.68 / 176.33 / 258.73; band 1 and band 2 panels 333.00; band 3 558.47 / 446.78; gauges 133.58 x 135.95, side-limited; the load triple wraps to two lines; no page scroll
- 1280x600: P4 and P3 by height drop host, clock, Storage, proc-stat, Cores, the gauge facts and scale, the CPU name and core count; bands 74.28 / 134.00 / 137.03 / 201.06; band 2 holds the two gauges at 631.09 each; both gauges 609.09 x 96.00 (6.34:1) in a 629.09 x 110.00 px body whose content is 609.09 x 96.00; the Memory panel content width is 629.09; the largest empty region is 0 px in every panel; no page scroll
- 1024x600: P4 and P3 by height; the same bands 74.28 / 134.00 / 137.03 / 201.06; the two gauges 481.09 x 96.00 (5.01:1) in 503.09-wide panels; Memory panel content 501.09; largest empty region 0 px; no page scroll
- 1440x600: the same short-view state; band 2 panels 711.09 and both gauges 689.09 x 96.00 (7.18:1); largest empty region 0 px
- 390x844: P4, P3 and P2 drop; one column; band 2 hidden; bands 162.11 / 319.95 / 319.95; panels 380 wide and stacked; CPU, Memory, GPU, Network and Processes remain; the gauge panels are hidden, so the short-view shape does not apply
- 390x844 drops: host, clock, Storage, proc-stat, Cores, both gauges, the CPU name and core count, the CPU facts and gauge scale, the Memory total, bar and legend, the network interface and axis labels, legend meta, the GPU model, temperature and power, process Command, Threads and User, the live and agent ages and the container chip
- 390x844 top bar: padding 0 4 and gaps 3 / 2, chip padding 0 3, live age hidden, version kept (DEF-LAYOUT-58)
- The short-view shape changes no band height and no dropped field: 1280x720, 1440x900 and 390x844 keep the gauge boxes and band heights they had before it

## Evidence

- `tmp/workflows/btop-web-layout/checkpoints/band-split.json`: band 2 panels 469.33 x 192.25 at 1440x900 and 333.00 x 180.67 at 1024x768; cores meter 65.67 px, 64 rows in 4 columns x 16, 27 percent paints 17.73 px; no scroll or clip at 1440x900, 1024x768, 390x844
- `tmp/workflows/btop-web-layout/checkpoints/mem-legend.json`: Memory rows 178.86 / 165.77 px; CPU side 188.36 / 144.92 px in `tmp/workflows/btop-web-layout/measure-mem-legend/after.json`, which this checkpoint names; gauges 147.61 x 150.23 and 133.58 x 135.95; vertical fill 0.02 / 6.49 px; label and percent edges aligned
- `tmp/workflows/btop-web-gauge-size/checkpoints/gauge.json`: circle 97.58 x 99.31 at 900x700; squircle 88.13 x 96.00 at 1440x600 and 1024x600, the shape DEF-LAYOUT-94 replaced there; the token clamp and the shape threshold
- `tmp/workflows/btop-web-space/checkpoints/short-view.json`: the short-view gauge at 1280x600, 1280x720, 1024x600, 1440x600, 1440x900 and 390x844 - gauge box, body, Memory panel content width, per-panel largest empty region, all five themes in dark and light, the system and GalaxaTron faces, contrast and clipping
- `tmp/workflows/btop-web-space/short-view/`: before/after measurement JSON and screenshots for the short-view change
- `tmp/workflows/btop-web-layout/measure-mem-legend/sweep.json`: the tier sizes and side widths at 1440x900, 1024x768, 900x700, 800x700, 1440x600, 390x844
- `tmp/workflows/btop-web-layout/measure-band-split/band-split-1440x900.png`: the aligned page at 1440x900

## Open

- A stored s/l panel size can make the two gauges differ at 1024x768 when the side-column limit binds; the equality rule holds at the default 1:1:1
- At P3 by width (max-width 899 with a body above the design threshold) the gauge keeps the circle and centres it in a side-less body; 800x700 leaves 130.94 px at each side of the gauge body. The short-view stretched shape is keyed to the short height (DEF-LAYOUT-94) and does not claim that state
- The Network legend is centred in the code at write time and leaves the block DEF-LAYOUT-93 names; this rule requires the entries to fill the cell
- The app.css header comment still lists the old shares 116 : 214 : 190 : 314; the tokens are 116 : 190 : 214 : 314

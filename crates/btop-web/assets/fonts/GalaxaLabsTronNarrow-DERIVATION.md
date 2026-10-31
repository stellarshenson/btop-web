# GalaxaLabsTron Narrow derivation

GalaxaLabsTronNarrow-VF.ttf is the 85 percent width derivative of GalaxaLabsTron-VF.ttf for the btop-web dashboard. One horizontal scale is applied to the outlines, the variation deltas and the advances, then the plus is grown about the centre of its own ink box; the family is renamed and the licence and designer records are kept.

## Source and licence

- Source: `crates/btop-web/fonts/GalaxaLabsTron-VF.ttf`, outside the embedded assets, so the page serves only the narrow face
- Source copyright: Copyright 2018 The Orbitron Project Authors (https://github.com/theleagueof/orbitron); GalaxaLabs derivative Copyright 2026 GalaxaLabs
- Licence: SIL Open Font License 1.1, shipped in `GalaxaLabsTronNarrow-OFL.txt`; name IDs 13 and 14 keep the OFL description and URL byte for byte
- Copyright (name ID 0): kept byte for byte from the source, the notice OFL requires
- Derivation date: 2026-10-29; rechecked 2026-10-30, when the plus was grown for ACC-THEME-170
- Build: fontTools 4.67.0, script `scripts/build_narrow_font.py`; the optional arguments are the source and destination paths, defaulting to the current chain. The script checks the plus geometry and the untouched tables itself, and a rerun always builds from the wide source, never from its own output

## Transform

- Every x coordinate in `glyf` is multiplied by 0.85 and rounded to the nearest font unit: simple glyph points, composite offsets and composite component matrices. The `y` coordinates are unchanged
- Every x delta in `gvar` is multiplied by 0.85, phantom points included, so the `wght` axis 400-900 keeps interpolating
- Every advance and left side bearing in `hmtx` is multiplied by 0.85 and rounded. `hmtx` stores integers, so the 770 unit digit advance becomes 655 units: 654.5 rounded up by the half unit that integer storage cannot hold, which is 0.655 em against the 0.6545 em target
- Every advance delta in `HVAR` is multiplied by 0.85 and rounded. `HVAR` carries no left or right side bearing maps in the source, so none were scaled
- Recomputed after the transforms: glyph bounds, `head` bounds, `hhea` advance maximum and side bearing extremes, `OS/2.xAvgCharWidth` (562)
- Kept without change: glyph order, `cmap`, `GPOS`, `GDEF`, `MVAR`, `STAT`, `gasp`, `post`, glyph instructions, and the source's `wght` 400 default
- For the plus (U+002B) only, after the width scale, every outline point p is mapped to c + 1.22 (p - c) in x and y, where c is the centre of its scaled default ink box, (180, 295). The ink box grows from 332 by 390 to 406 by 476 units and the bars from 69 and 82 thick to 84 and 100, so at 10 px the arms reach 0.84 and 1.0 px and the sign no longer reads as an asterisk. The glyph stays one 12 point contour, and the mapping runs before the metadata recompute
- The plus x and y deltas in every `gvar` tuple are multiplied by 1.22 with the outline, so every weight keeps the same shape grown about the same centre. Its four phantom point deltas are left alone, and so are its `hmtx` advance and side bearing and its `HVAR` advance delta: the 368 unit default advance stays 368, so no string reflows. The grown ink overhangs that advance by 23 units on the left and 15 on the right at wght 400, into the spaces beside the sign; the right overhang is 24 units at 900

## Name records

- Family (IDs 1, 16): GalaxaLabsTron Narrow
- Subfamily (IDs 2, 17): Regular
- Full name (ID 4): GalaxaLabsTron Narrow
- PostScript name (ID 6): GalaxaLabsTronNarrow-Regular
- Unique ID (ID 3): 1.000;GLXL;GalaxaLabsTronNarrow-Regular
- Version (ID 5): Version 1.000; GalaxaLabsTron Narrow; derived from Orbitron
- Variation PostScript prefix (ID 25) and instance names (IDs 404-407): GalaxaLabsTronNarrow, GalaxaLabsTronNarrow-Regular, -Medium, -Bold, -Black
- Manufacturer and designer (IDs 8, 9): Anonymous, as in the source
- Sample text (ID 19) and description (ID 10): renamed to the Narrow family
- Copyright (ID 0), licence (IDs 13, 14): unchanged

## Checks

- All 281 glyphs draw, every outline is the source outline scaled by 0.85 except the grown plus, and the 265 `gvar` tuples carry the same axes. Every x delta is scaled by 0.85, except the plus: its outline x deltas are scaled by 0.85 then 1.22 and its outline y deltas by 1.22 alone, while its four phantom deltas are unchanged. The 51 `HVAR` advance deltas are scaled
- The ten digits advance 655 units at every weight. At 32 px the browser measures 21.0 px against the 20.944 px target, 0.056 px off and inside the 0.5 px tolerance
- Intermediate weights 400, 500, 650 and 900 differ from the source advance times 0.85 by at most 1.1 units
- Adjacent-glyph ink overhang in font units matches the source scaled by 0.85 within 0.6 units at weights 400, 650 and 900, except at the grown plus, whose ink now overhangs its own advance. The only overlapping printable ASCII pair is `ij`, which overlaps in the source face too; at wght 900 the plus's left overhang also reaches the preceding `*` by 4 units, a pair no dashboard string contains
- The zero and L are the source outlines scaled in x, point for point at the stored coordinates: every stored x equals round(0.85 times the source x) and every y is unchanged. At wght 400 the zero's bbox is 38..615 by 0..720 with its ink centre at 326.5/360, the L stem is 48..117, the gap is 80 units, the floor runs 48..660 (612 long, 81 thick in y, the source thickness; the stem is 69 wide in x because the scale is horizontal) and the L advance is 662. The crossbar's perpendicular thickness is 75.7 units at 400, 114.5 at 700 and 140.3 at 900, between the side strokes (69.5 / 105.8 / 130.0) and the top and bottom strokes (82.0 / 124.6 / 153.0): one horizontal scale leaves a diagonal stroke between the two
- The plus ink box is 332 by 390 units (0.332 by 0.390 em) before and 406 by 476 (0.406 by 0.476 em) after at wght 400, 341 by 401 before and 417 by 489 after at 600, and 355 by 417 before and 434 by 509 after at 900: every size clears the 390 by 460 unit floor of ACC-THEME-170. The four inner corners, which bound the two bars, put their centre at most 2 units from the ink centre in x and y at all three weights: 2.0 and 1.0 at 400, 1.9 and 1.3 at 600, 2.0 and 1.0 at 900
- The plus advance is unchanged at every weight: 368 at 400, 376 at 600 and 387 at 900 in both builds, the last two from its untouched `HVAR` variation, so the dashboard's 10 px `free + cache` row keeps its width
- No other glyph, `gvar` delta, advance or name record changed from the pre-plus build: the other 280 outlines and their tuples, and the `hmtx`, `HVAR`, `fvar` and `name` bytes, all compare equal, the `head` and `hhea` extremes recompute unchanged, and the script asserts it
- A fresh build from the unchanged source reproduces `glyf`, `gvar`, `hmtx`, `HVAR`, `cmap`, `GPOS`, `GDEF`, `MVAR`, `STAT`, `gasp`, `post`, `name`, `maxp`, `hhea`, `loca`, `OS/2` and `fvar` byte for byte; only `head.modified` and `head.checkSumAdjustment` differ, so the shipped file stands
- The specimen page at 12, 16 and 32 px, DPR 1 and 2, shows the digits, uppercase, lowercase, punctuation and the full printable ASCII set; every glyph is distinguishable, no new ink overlap appears apart from the grown plus's designed overhang into the spaces beside it, and the strokes match the current face's vertical extents
- At 10 px the L gap is a fully empty row at wght 400 and a partial-coverage row at 700 and 900 on a 1x device scale, and a clear empty band from 2x; from 12 px it is an empty row at every weight on 1x. The zero's crossbar and ring render with no visible weight difference at 10, 12 and 32 px. No dashboard string collides: the digits keep their 78 unit zero-to-zero ink gap, and at wght 400 the only overlapping printable ASCII pair remains `ij`

## Files

- `GalaxaLabsTronNarrow-VF.ttf`: the variable font
- `GalaxaLabsTronNarrow-OFL.txt`: SIL OFL 1.1 text
- `GalaxaLabsTronNarrow-DERIVATION.md`: this note

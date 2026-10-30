# GalaxaLabsTron Narrow derivation

GalaxaLabsTronNarrow-VF.ttf is the 85 percent width derivative of GalaxaLabsTron-VF.ttf for the btop-web dashboard. One horizontal scale is applied to the outlines, the variation deltas and the advances; the family is renamed and the licence and designer records are kept.

## Source and licence

- Source: `crates/btop-web/fonts/GalaxaLabsTron-VF.ttf`, outside the embedded assets, so the page serves only the narrow face
- Source copyright: Copyright 2018 The Orbitron Project Authors (https://github.com/theleagueof/orbitron); GalaxaLabs derivative Copyright 2026 GalaxaLabs
- Licence: SIL Open Font License 1.1, shipped in `GalaxaLabsTronNarrow-OFL.txt`; name IDs 13 and 14 keep the OFL description and URL byte for byte
- Copyright (name ID 0): kept byte for byte from the source, the notice OFL requires
- Derivation date: 2026-10-29; rechecked 2026-10-30
- Build: fontTools 4.67.0, script `tmp/workflows/btop-web-ui-steel/build_narrow_font.py`; the optional arguments are the source and destination paths, defaulting to the current chain

## Transform

- Every x coordinate in `glyf` is multiplied by 0.85 and rounded to the nearest font unit: simple glyph points, composite offsets and composite component matrices. The `y` coordinates are unchanged
- Every x delta in `gvar` is multiplied by 0.85, phantom points included, so the `wght` axis 400-900 keeps interpolating
- Every advance and left side bearing in `hmtx` is multiplied by 0.85 and rounded. `hmtx` stores integers, so the 770 unit digit advance becomes 655 units: 654.5 rounded up by the half unit that integer storage cannot hold, which is 0.655 em against the 0.6545 em target
- Every advance delta in `HVAR` is multiplied by 0.85 and rounded. `HVAR` carries no left or right side bearing maps in the source, so none were scaled
- Recomputed after the scale: glyph bounds, `head` bounds, `hhea` advance maximum and side bearing extremes, `OS/2.xAvgCharWidth` (562)
- Kept without change: glyph order, `cmap`, `GPOS`, `GDEF`, `MVAR`, `STAT`, `gasp`, `post`, glyph instructions, and the source's `wght` 400 default

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

- All 281 glyphs draw with the scaled outlines; the 265 `gvar` tuples carry the same axes with scaled x deltas; the 51 `HVAR` advance deltas are scaled
- The ten digits advance 655 units at every weight. At 32 px the browser measures 21.0 px against the 20.944 px target, 0.056 px off and inside the 0.5 px tolerance
- Intermediate weights 400, 500, 650 and 900 differ from the source advance times 0.85 by at most 1.1 units
- Adjacent-glyph ink overhang in font units matches the source scaled by 0.85 within 0.6 units at weights 400, 650 and 900. The only overlapping printable ASCII pair is `ij`, which overlaps in the source face too
- The zero and L are the source outlines scaled in x, point for point at the stored coordinates: every stored x equals round(0.85 times the source x) and every y is unchanged. At wght 400 the zero's bbox is 38..615 by 0..720 with its ink centre at 326.5/360, the L stem is 48..117, the gap is 80 units, the floor runs 48..660 (612 long, 81 thick in y, the source thickness; the stem is 69 wide in x because the scale is horizontal) and the L advance is 662. The crossbar's perpendicular thickness is 75.7 units at 400, 114.5 at 700 and 140.3 at 900, between the side strokes (69.5 / 105.8 / 130.0) and the top and bottom strokes (82.0 / 124.6 / 153.0): one horizontal scale leaves a diagonal stroke between the two
- A fresh build from the unchanged source reproduces `glyf`, `gvar`, `hmtx`, `HVAR`, `cmap`, `GPOS`, `GDEF`, `MVAR`, `STAT`, `gasp`, `post`, `name`, `maxp`, `hhea`, `loca`, `OS/2` and `fvar` byte for byte; only `head.modified` and `head.checkSumAdjustment` differ, so the shipped file stands
- The specimen page at 12, 16 and 32 px, DPR 1 and 2, shows the digits, uppercase, lowercase, punctuation and the full printable ASCII set; every glyph is distinguishable, no new ink overlap appears, and the strokes match the current face's vertical extents
- At 10 px the L gap is a fully empty row at wght 400 and a partial-coverage row at 700 and 900 on a 1x device scale, and a clear empty band from 2x; from 12 px it is an empty row at every weight on 1x. The zero's crossbar and ring render with no visible weight difference at 10, 12 and 32 px. No dashboard string collides: the digits keep their 78 unit zero-to-zero ink gap, and the only overlapping printable ASCII pair remains `ij`

## Files

- `GalaxaLabsTronNarrow-VF.ttf`: the variable font
- `GalaxaLabsTronNarrow-OFL.txt`: SIL OFL 1.1 text
- `GalaxaLabsTronNarrow-DERIVATION.md`: this note

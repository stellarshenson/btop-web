# GalaxaLabsTron derivation

GalaxaLabsTron-VF.ttf is the source face of the GalaxaTron font for the btop-web dashboard: the chamfer variant, where every curved segment is replaced by straight chamfer segments. It is not embedded in the binary and no stylesheet declares it; the page serves only `GalaxaLabsTronNarrow-VF.ttf`, the 85 percent width derivative. Orbitron is licensed under the SIL Open Font License 1.1 and its metadata declares no Reserved Font Name, so the family can be renamed and the derivative stays under the same licence.

## Source and licence

- Source: `tmp/preview/font2/GalaxaLabsTronChamfer-VF.ttf`, the chamfer variable font in the project's preview folder
- Source copyright: Copyright 2018 The Orbitron Project Authors (https://github.com/theleagueof/orbitron)
- Licence: SIL Open Font License 1.1, shipped in `OFL.txt`; name IDs 13 and 14 keep the OFL description and URL from the source
- Derivation date: 2026-10-28; renamed and anonymised 2026-10-29
- Build: fontTools 4.67.0, rebuilt from the main font's flattened `wght` 400 and 900 masters; 281 glyphs, 212 cmap codepoints

## Name records

- Family (IDs 1, 16): GalaxaLabsTron
- Subfamily (IDs 2, 17): Regular
- Full name (ID 4): GalaxaLabsTron
- PostScript name (ID 6): GalaxaLabsTron-Regular
- Unique ID (ID 3): 1.000;GLXL;GalaxaLabsTron-Regular
- Version (ID 5): Version 1.000; GalaxaLabsTron; derived from Orbitron
- Manufacturer (ID 8): Anonymous
- Designer (ID 9): Anonymous
- Vendor URL (ID 11): absent, so empty
- Designer URL (ID 12): empty
- Copyright (ID 0): kept as in the source, the notice OFL requires
- Licence (IDs 13, 14): kept exactly
- Axis: `wght` 400-900, default 400

## Anonymity

The font names no author. Manufacturer, designer, vendor URL and designer URL records (IDs 8, 9, 11, 12) read Anonymous or are empty. The copyright notice (ID 0) keeps the original Orbitron notice that OFL requires, and name IDs 13 and 14 keep the licence text and URL exactly. Only the name table changed; every other table is byte-identical to the source font except the four `checkSumAdjustment` bytes in `head`, which a save rewrites.

## Same as the main font

- All 281 glyphs draw with zero curve commands.
- Every advance matches the main GalaxaLabsTron font, and all ten digits advance 770 units at every weight.
- The main font's customisations are carried: Orbitron's rounded C and G and the squared S corners, all chamfered like every glyph in this face (ACC-THEME-116), the L stem and floor, the slashed zero with the widened pair gap, the narrower percent, tabular digits and the added arrows and middot.
- The customisation measurements are recorded beside the main font, in the preview folder's `GalaxaLabsTron-DERIVATION.md`.

## Acceptance checks

ACC-THEME-112-ZERO-BAR, ACC-THEME-113-LFOOT and ACC-THEME-115-ZERO-LINING are measured with fontTools 4.67.0 on instances at wght 400, 700 and 900. The crossbar thickness is the perpendicular distance from each long edge to the opposite short edge's endpoint; the ring stroke is the horizontal run at mid-height and the vertical run at mid-width; the ink centre is the bounding-box centre. The crossbar-to-ring tolerance is 1 font unit, which is 0.032 px at 32 px. The cross-glyph stroke tolerance is 2 units, because the digits' own axis-aligned strokes span 82 to 84 units at 400 and the zero's ring, on straight sides, sits at the low end; 2 units is 0.064 px at 32 px. The ink-centre tolerance is 2 units. All are below one device pixel at any rendered scale.

| Glyph | Measurement | 400 | 700 | 900 |
|-------|-------------|-----|-----|-----|
| zero | crossbar flanks | 82.32 / 81.63 | 124.42 / 124.15 | 152.44 / 152.44 |
| zero | ring stroke | 82.00 | 124.60 | 153.00 |
| zero | crossbar deviation from the ring | 0.37 | 0.47 | 0.56 |
| zero | vertical extent | 720 | 720 | 720 |
| digits 1-9 | vertical extent | 720-721 | 720-721 | 720-721 |
| digits 1-9 | axis-aligned stroke width | 82-84 | 124.6-126.0 | 153-154 |
| zero | ring stroke deviation from that range | 0-2.0 | 0-1.4 | 0-1.0 |
| zero | ink centre | 384.5 / 360.0 | 384.5 / 360.0 | 384.5 / 360.0 |
| digits 1-9 | ink centre | 384.5-385.5 / 360.0-360.5 | 384.5-386.0 / 360.0-360.5 | 384.5-385.5 / 360.0-360.5 |
| zero | advance | 770 | 770 | 770 |
| L | stem width | 81.0 | 125.4 | 155.0 |
| L | gap | 80.0 | 80.0 | 80.0 |
| L | floor x range and thickness | 57-777, 81.0 | 57-777, 125.4 | 57-777, 155.0 |
| L | advance | 779 | 779 | 779 |

The L floor starts at the stem's left edge and ends at Orbitron's arm end (x=777, 720 units long) at every weight. The rendered checks at 10, 12 and 32 px use the served narrow face and are recorded in `crates/btop-web/assets/fonts/GalaxaLabsTronNarrow-DERIVATION.md`.

## Files

This folder sits outside the embedded `assets/` tree, so rust-embed ships none of these files.

- `GalaxaLabsTron-VF.ttf`: the variable font
- `OFL.txt`: SIL OFL 1.1 text
- `GalaxaLabsTron-DERIVATION.md`: this note

# GalaxaLabsTron derivation

GalaxaLabsTron-VF.ttf is the GalaxaLabsTron display font for the btop-web dashboard: the chamfer variant, where every curved segment is replaced by straight chamfer segments. Orbitron is licensed under the SIL Open Font License 1.1 and its metadata declares no Reserved Font Name, so the family can be renamed and the derivative stays under the same licence.

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

The font names no author. Manufacturer, designer, vendor URL and designer URL records (IDs 8, 9, 11, 12) read Anonymous or are empty. The copyright notice (ID 0) keeps the original Orbitron notice that OFL requires, and name IDs 13 and 14 keep the licence text and URL exactly. Only the name table changed: outlines, metrics, advances and every other table are byte-identical to the source font.

## Same as the main font

- All 281 glyphs draw with zero curve commands.
- Every advance matches the main GalaxaLabsTron font, and all ten digits advance 770 units at every weight.
- The main font's customisations are carried: squared C and G, squared S corners, the L stem and floor, the slashed zero with the widened pair gap, the narrower percent, tabular digits and the added arrows and middot.
- The customisation measurements are recorded beside the main font, in the preview folder's `GalaxaLabsTron-DERIVATION.md`.

## Files

- `GalaxaLabsTron-VF.ttf`: the variable font
- `OFL.txt`: SIL OFL 1.1 text
- `GalaxaLabsTron-DERIVATION.md`: this note

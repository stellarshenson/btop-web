"""Build GalaxaLabsTronNarrow-VF.ttf from GalaxaLabsTron-VF.ttf (ACC-THEME-154, ACC-THEME-170).

One horizontal scale of 0.85 is applied to every x coordinate in glyf, every
x delta in gvar (phantom points included) and every advance in hmtx/HVAR. The
plus (U+002B) is then grown 1.22 in both axes about the centre of its scaled
default ink box, outline deltas included, so its 10 px arms reach a pixel and
it reads as a plus; its advance and phantom deltas stay put, so no string
reflows. Only the family name records are rewritten; name IDs 0, 13 and 14 are
left byte-identical, as the OFL requires. Idempotent: rerunning overwrites the
output from the shipped source, never from its own output.
"""

import io
import sys

from fontTools.misc.roundTools import otRound
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

SCALE = 0.85
PLUS = 0x2B
PLUS_SCALE = 1.22
SRC = sys.argv[1] if len(sys.argv) > 1 else "crates/btop-web/fonts/GalaxaLabsTron-VF.ttf"
DST = sys.argv[2] if len(sys.argv) > 2 else "crates/btop-web/assets/fonts/GalaxaLabsTronNarrow-VF.ttf"

FAMILY = "GalaxaLabsTron Narrow"
PS = "GalaxaLabsTronNarrow-Regular"
NAME_OVERRIDES = {
    1: FAMILY,
    2: "Regular",
    3: f"1.000;GLXL;{PS}",
    4: FAMILY,
    5: f"Version 1.000; {FAMILY}; derived from Orbitron",
    6: PS,
    8: "Anonymous",
    9: "Anonymous",
    10: (
        f"{FAMILY} is the GalaxaLabs derivative of Orbitron for the btop-web "
        "dashboard, with every curve replaced by straight chamfer segments and "
        "the outlines scaled to 85 percent in x."
    ),
    16: FAMILY,
    17: "Regular",
    19: f"{FAMILY} 0123456789 GiB MiB/s",
    25: "GalaxaLabsTronNarrow",
    404: "GalaxaLabsTronNarrow-Regular",
    405: "GalaxaLabsTronNarrow-Medium",
    406: "GalaxaLabsTronNarrow-Bold",
    407: "GalaxaLabsTronNarrow-Black",
}
KEEP = (0, 13, 14)


def scale_glyf(font):
    glyf = font["glyf"]
    for name in font.getGlyphOrder():
        glyph = glyf[name]
        if glyph.numberOfContours == 0:
            continue
        if glyph.isComposite():
            for comp in glyph.components:
                comp.x = otRound(comp.x * SCALE)
                if hasattr(comp, "transform"):
                    # [[xx, xy], [yx, yy]] -> diag(0.85, 1) * M
                    xx, xy = comp.transform[0]
                    yx, yy = comp.transform[1]
                    comp.transform = [[xx * SCALE, xy], [yx * SCALE, yy]]
        else:
            coords = glyph.coordinates
            for i in range(len(coords)):
                x, y = coords[i]
                coords[i] = (otRound(x * SCALE), y)
        glyph.recalcBounds(glyf)


def scale_gvar(font):
    for variations in font["gvar"].variations.values():
        for variation in variations:
            if variation.coordinates is None:
                continue
            variation.coordinates = [
                None if delta is None else (otRound(delta[0] * SCALE), delta[1])
                for delta in variation.coordinates
            ]


def grow_plus(font):
    """Scale the plus about its scaled ink centre; hmtx and the phantom deltas stay."""
    name = font.getBestCmap()[PLUS]
    glyf = font["glyf"]
    glyph = glyf[name]
    assert glyph.numberOfContours == 1 and len(glyph.coordinates) == 12, (
        "the plus is one 12 point contour"
    )
    cx = (glyph.xMin + glyph.xMax) / 2
    cy = (glyph.yMin + glyph.yMax) / 2
    for i in range(len(glyph.coordinates)):
        x, y = glyph.coordinates[i]
        glyph.coordinates[i] = (
            otRound(cx + PLUS_SCALE * (x - cx)),
            otRound(cy + PLUS_SCALE * (y - cy)),
        )
    glyph.recalcBounds(glyf)
    # The last four deltas are the phantom points; the advance lives there and
    # in HVAR, and both stay untouched.
    for variation in font["gvar"].variations[name]:
        coords = variation.coordinates
        if coords is None:
            continue
        assert len(coords) == len(glyph.coordinates) + 4, "the plus has four phantom deltas"
        for i in range(len(coords) - 4):
            delta = coords[i]
            if delta is not None:
                coords[i] = (otRound(delta[0] * PLUS_SCALE), otRound(delta[1] * PLUS_SCALE))


def scale_hmtx(font):
    hmtx = font["hmtx"]
    for name in font.getGlyphOrder():
        advance, lsb = hmtx.metrics[name]
        hmtx.metrics[name] = (otRound(advance * SCALE), otRound(lsb * SCALE))


def scale_hvar(font):
    hvar = font.get("HVAR")
    if hvar is None:
        return
    store = hvar.table.VarStore
    for var_data in store.VarData:
        for item in var_data.Item:
            for i in range(len(item)):
                item[i] = otRound(item[i] * SCALE)


def scale_metadata(font):
    font["OS/2"].recalcAvgCharWidth(font)
    font["hhea"].recalc(font)
    head = font["head"]
    glyphs = [
        font["glyf"][name]
        for name in font.getGlyphOrder()
        if font["glyf"][name].numberOfContours != 0
    ]
    head.xMin = min(g.xMin for g in glyphs)
    head.xMax = max(g.xMax for g in glyphs)
    head.yMin = min(g.yMin for g in glyphs)
    head.yMax = max(g.yMax for g in glyphs)


def rename(font):
    for record in font["name"].names:
        if record.nameID in KEEP:
            continue
        replacement = NAME_OVERRIDES.get(record.nameID)
        if replacement is not None:
            record.string = replacement


def build(with_plus):
    font = TTFont(SRC)
    if font["head"].unitsPerEm != 1000:
        sys.exit("expected 1000 units per em")
    scale_glyf(font)
    scale_gvar(font)
    if with_plus:
        grow_plus(font)
    scale_hmtx(font)
    scale_hvar(font)
    scale_metadata(font)
    rename(font)
    return font


def serialized(font):
    buffer = io.BytesIO()
    font.save(buffer)
    return buffer.getvalue()


def instanced(data, wght):
    return instantiateVariableFont(TTFont(io.BytesIO(data)), {"wght": wght})


def bars_centred(glyph):
    """The four corners nearest the ink centre bound the two bars; return their centre."""
    points = list(glyph.coordinates)
    cx = (glyph.xMin + glyph.xMax) / 2
    cy = (glyph.yMin + glyph.yMax) / 2
    inner = sorted(points, key=lambda p: (p[0] - cx) ** 2 + (p[1] - cy) ** 2)[:4]
    xs = [p[0] for p in inner]
    ys = [p[1] for p in inner]
    return (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2


def verify(font, baseline):
    """Check the plus and prove every other glyph, delta, advance and name record is untouched."""
    data = serialized(font)
    saved = TTFont(io.BytesIO(data))
    base_data = serialized(baseline)
    old = TTFont(io.BytesIO(base_data))
    plus = saved.getBestCmap()[PLUS]

    assert saved.getGlyphOrder() == old.getGlyphOrder(), "the glyph order changed"
    for name in saved.getGlyphOrder():
        if name == plus:
            continue
        new_glyph, old_glyph = saved["glyf"][name], old["glyf"][name]
        assert new_glyph.numberOfContours == old_glyph.numberOfContours, f"{name} contour count"
        assert (
            new_glyph.getCoordinates(saved["glyf"])[0]
            == old_glyph.getCoordinates(old["glyf"])[0]
        ), f"{name} outline"
    for name, variations in saved["gvar"].variations.items():
        if name == plus:
            continue
        previous = old["gvar"].variations[name]
        assert [v.axes for v in variations] == [v.axes for v in previous], f"{name} tuples"
        assert [v.coordinates for v in variations] == [v.coordinates for v in previous], (
            f"{name} deltas"
        )
    assert saved["hmtx"].metrics == old["hmtx"].metrics, "hmtx changed"
    for table in ("HVAR", "name", "fvar"):
        assert saved[table].compile(saved) == old[table].compile(old), f"{table} changed"

    glyph = saved["glyf"][plus]
    assert glyph.numberOfContours == 1 and len(glyph.coordinates) == 12, (
        "the plus is one 12 point contour"
    )
    assert saved["hmtx"][plus][0] == 368, "the plus advance is no longer 368"

    for wght in (400, 900):
        face = instanced(data, wght)
        grown = face["glyf"][plus]
        width = grown.xMax - grown.xMin
        height = grown.yMax - grown.yMin
        assert width >= 390, f"the plus is {width} units wide at {wght}"
        assert height >= 460, f"the plus is {height} units tall at {wght}"
        cx = (grown.xMin + grown.xMax) / 2
        cy = (grown.yMin + grown.yMax) / 2
        bx, by = bars_centred(grown)
        assert abs(bx - cx) <= 2, f"the bars sit {bx - cx} units off centre at {wght}"
        assert abs(by - cy) <= 2, f"the bars sit {by - cy} units off centre at {wght}"
        # The advance is hmtx 368 plus the source's own HVAR delta: 0 at 400,
        # 19 at 900. The plus rule leaves hmtx and HVAR alone, so it is the
        # pre-transform advance at every weight and the 400 advance never moves.
        advance = face["hmtx"][plus][0]
        assert advance == instanced(base_data, wght)["hmtx"][plus][0], (
            f"the plus advance changed at {wght}"
        )
    return data


def main():
    data = verify(build(True), build(False))
    with open(DST, "wb") as output:
        output.write(data)
    print(f"wrote {DST}")


if __name__ == "__main__":
    main()

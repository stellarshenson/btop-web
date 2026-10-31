//! ACC-THEME-112-ZERO-BAR, ACC-THEME-113-LFOOT, ACC-THEME-115-ZERO-LINING at
//! the default weight: the zero's crossbar matches the ring stroke, the
//! capital L is a stem, a gap and a floor, and the zero lines up with the
//! other digits. ACC-THEME-154-NARROW-FONT, ACC-THEME-167-NARROW-ONLY: the
//! generated 85%-width face is the only Galaxa face served; it carries the
//! renamed family, keeps the wght axis 400-900 and the OFL records, and its
//! digits advance 0.6545 em. Its derivation source sits outside the embedded
//! assets. ACC-THEME-170-PLUS-GLYPH: the plus is grown 1.22 about its ink
//! centre at every weight, with its bars centred and its 368 unit advance
//! unchanged.

use std::path::Path;

use ttf_parser::{Face, Tag, name_id};

/// The served face, read from the embedded asset folder.
fn font(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/fonts")
            .join(name),
    )
    .expect("font asset")
}

/// The derivation source, read from outside the embedded asset folder.
fn source_font(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fonts")
            .join(name),
    )
    .expect("font source")
}

/// The Windows Unicode record; the Mac records are not UTF-8 decodable.
fn name(face: &Face<'_>, id: u16) -> Option<String> {
    face.names()
        .into_iter()
        .find(|rec| rec.name_id == id && rec.is_unicode())
        .and_then(|rec| rec.to_string())
}

/// One contour's points in font units, in stored order. The Galaxa faces draw
/// every curved segment as a straight chamfer, so each contour is a polygon.
type Contour = Vec<(f64, f64)>;

struct Skeleton(Vec<Contour>);

impl ttf_parser::OutlineBuilder for Skeleton {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(vec![(f64::from(x), f64::from(y))]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0
            .last_mut()
            .expect("line_to follows move_to")
            .push((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.line_to(x1, y1);
        self.line_to(x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.line_to(x1, y1);
        self.line_to(x2, y2);
        self.line_to(x, y);
    }

    fn close(&mut self) {}
}

/// The contours of `ch` at the face's default instance, in font units.
fn contours(face: &Face<'_>, ch: char) -> Vec<Contour> {
    let id = face
        .glyph_index(ch)
        .unwrap_or_else(|| panic!("{ch} is in the cmap"));
    let mut skeleton = Skeleton(Vec::new());
    face.outline_glyph(id, &mut skeleton)
        .unwrap_or_else(|| panic!("{ch} has an outline"));
    for contour in &mut skeleton.0 {
        if contour.len() > 1 && contour[0] == contour[contour.len() - 1] {
            contour.pop();
        }
    }
    skeleton.0
}

fn bounds(polylines: &[Contour]) -> (f64, f64, f64, f64) {
    polylines.iter().flatten().fold(
        (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
        |(x0, y0, x1, y1), (x, y)| (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y)),
    )
}

fn contour_bounds(contour: &Contour) -> (f64, f64, f64, f64) {
    bounds(std::slice::from_ref(contour))
}

/// The horizontal ink runs at `y`, paired left to right.
fn row_runs(polylines: &[Contour], y: f64) -> Vec<(f64, f64)> {
    let mut xs = Vec::new();
    for contour in polylines {
        for i in 0..contour.len() {
            let (p, q) = (contour[i], contour[(i + 1) % contour.len()]);
            if (p.1 - y) * (q.1 - y) < 0.0 {
                let t = (y - p.1) / (q.1 - p.1);
                xs.push(p.0 + t * (q.0 - p.0));
            }
        }
    }
    xs.sort_by(|a, b| a.partial_cmp(b).expect("finite coordinates"));
    xs.as_chunks::<2>()
        .0
        .iter()
        .filter(|pair| pair[1] - pair[0] > 0.5)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

/// The zero's added crossbar: the only four-point contour with no
/// axis-aligned edge.
fn crossbar(polylines: &[Contour]) -> &Contour {
    polylines
        .iter()
        .find(|contour| {
            contour.len() == 4
                && (0..4).all(|i| {
                    let (p, q) = (contour[i], contour[(i + 1) % 4]);
                    (p.0 - q.0).abs() > 1e-9 && (p.1 - q.1).abs() > 1e-9
                })
        })
        .expect("the zero has a crossbar")
}

/// The perpendicular distances from the crossbar's two long edges, in font units.
fn crossbar_flanks(contour: &Contour) -> (f64, f64) {
    let distance = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).abs()
            / ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
    };
    (
        distance(contour[0], contour[1], contour[3]),
        distance(contour[2], contour[3], contour[1]),
    )
}

/// The centre of the four corners nearest the ink centre, which bound the two
/// bars of a plus whatever the weight does to its edges.
fn bar_centre(polylines: &[Contour]) -> (f64, f64) {
    let (x0, y0, x1, y1) = bounds(polylines);
    let centre = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let mut points: Vec<(f64, f64)> = polylines.iter().flatten().copied().collect();
    points.sort_by(|a, b| {
        let a = (a.0 - centre.0).powi(2) + (a.1 - centre.1).powi(2);
        let b = (b.0 - centre.0).powi(2) + (b.1 - centre.1).powi(2);
        a.partial_cmp(&b).expect("finite coordinates")
    });
    let (ix0, ix1) = points[..4]
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), p| {
            (lo.min(p.0), hi.max(p.0))
        });
    let (iy0, iy1) = points[..4]
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), p| {
            (lo.min(p.1), hi.max(p.1))
        });
    ((ix0 + ix1) / 2.0, (iy0 + iy1) / 2.0)
}

#[test]
fn narrow_face_keeps_the_family_axis_and_digit_advance() {
    let bytes = font("GalaxaLabsTronNarrow-VF.ttf");
    let face = Face::parse(&bytes, 0).expect("the narrow font parses");

    assert_eq!(face.units_per_em(), 1000);
    assert_eq!(
        name(&face, name_id::FAMILY).as_deref(),
        Some("GalaxaLabsTron Narrow")
    );
    assert_eq!(
        name(&face, name_id::TYPOGRAPHIC_FAMILY).as_deref(),
        Some("GalaxaLabsTron Narrow")
    );
    assert_eq!(name(&face, name_id::SUBFAMILY).as_deref(), Some("Regular"));
    assert_eq!(
        name(&face, name_id::FULL_NAME).as_deref(),
        Some("GalaxaLabsTron Narrow")
    );
    assert_eq!(
        name(&face, name_id::POST_SCRIPT_NAME).as_deref(),
        Some("GalaxaLabsTronNarrow-Regular")
    );

    let axes: Vec<_> = face.variation_axes().into_iter().collect();
    assert_eq!(axes.len(), 1);
    assert_eq!(axes[0].tag.to_bytes(), *b"wght");
    assert_eq!(axes[0].min_value, 400.0);
    assert_eq!(axes[0].def_value, 400.0);
    assert_eq!(axes[0].max_value, 900.0);

    // hmtx stores integer font units: 770 * 0.85 = 654.5 rounds to 655, half
    // a unit above the 0.6545 em target, which is 0.016 px at 32 px.
    for digit in '0'..='9' {
        let id = face
            .glyph_index(digit)
            .unwrap_or_else(|| panic!("{digit} is in the cmap"));
        let advance = face.glyph_hor_advance(id).expect("an hmtx advance");
        assert_eq!(advance, 655, "digit {digit} advances 0.655 em");
        assert!(
            (f64::from(advance) - 654.5).abs() <= 0.5,
            "digit {digit} advance is {advance}, not 654.5 of 1000 units"
        );
    }

    // The OFL keeps the source copyright and licence records byte for byte,
    // and the designer records stay anonymous.
    let source_bytes = source_font("GalaxaLabsTron-VF.ttf");
    let source = Face::parse(&source_bytes, 0).expect("the source font parses");
    for id in [
        name_id::COPYRIGHT_NOTICE,
        name_id::LICENSE,
        name_id::LICENSE_URL,
    ] {
        assert_eq!(
            name(&face, id),
            name(&source, id),
            "name ID {id} differs from the source"
        );
    }
    for id in [name_id::MANUFACTURER, name_id::DESIGNER] {
        assert_eq!(name(&face, id).as_deref(), Some("Anonymous"));
    }
}

/// ACC-THEME-112-ZERO-BAR, ACC-THEME-113-LFOOT, ACC-THEME-115-ZERO-LINING at
/// the default weight: the crossbar matches the ring stroke within 1 font
/// unit, the capital L is a stem, a 60-100 unit gap and a floor that ends at
/// Orbitron's arm, and the zero's height and ink centre line up with the
/// other digits. The derivation note records the 400/700/900 measurements.
#[test]
fn source_face_zero_and_l_meet_the_geometry_criteria() {
    let bytes = source_font("GalaxaLabsTron-VF.ttf");
    let face = Face::parse(&bytes, 0).expect("the source font parses");
    assert_eq!(face.units_per_em(), 1000);

    for digit in '0'..='9' {
        let id = face.glyph_index(digit).expect("a mapped digit");
        assert_eq!(
            face.glyph_hor_advance(id),
            Some(770),
            "digit {digit} advances 0.770 em"
        );
    }

    let zero = contours(&face, '0');
    assert_eq!(zero.len(), 4, "the ring, two counters and the crossbar");
    assert_eq!(bounds(&zero), (45.0, 0.0, 724.0, 720.0));

    let runs = row_runs(&zero, 360.0);
    let ring_left = runs.first().expect("the left ring stroke");
    let ring_right = runs.last().expect("the right ring stroke");
    let ring = ((ring_left.1 - ring_left.0) + (ring_right.1 - ring_right.0)) / 2.0;
    assert_eq!(ring, 82.0, "the ring stroke at mid-height");
    let (flank_a, flank_b) = crossbar_flanks(crossbar(&zero));
    for flank in [flank_a, flank_b] {
        assert!(
            (flank - ring).abs() <= 1.0,
            "a {flank}-unit crossbar flank differs from the {ring}-unit ring by more than 1 unit"
        );
    }

    let (zx0, zy0, zx1, zy1) = bounds(&zero);
    let zero_centre = ((zx0 + zx1) / 2.0, (zy0 + zy1) / 2.0);
    assert_eq!(zy1 - zy0, 720.0);
    for digit in '1'..='9' {
        let (x0, y0, x1, y1) = bounds(&contours(&face, digit));
        assert!(
            (720.0..=721.0).contains(&(y1 - y0)),
            "digit {digit} is {} units tall",
            y1 - y0
        );
        let centre = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        assert!(
            (centre.0 - zero_centre.0).abs() <= 2.0 && (centre.1 - zero_centre.1).abs() <= 2.0,
            "digit {digit} centre {centre:?} is more than 2 units from the zero's {zero_centre:?}"
        );
    }

    let l = contours(&face, 'L');
    assert_eq!(l.len(), 2, "the L is a stem and a floor");
    let stem = l
        .iter()
        .find(|contour| contour_bounds(contour).1 > 0.0)
        .expect("the stem");
    let floor = l
        .iter()
        .find(|contour| contour_bounds(contour).1 == 0.0)
        .expect("the floor");
    let (sx0, sy0, sx1, sy1) = contour_bounds(stem);
    let (fx0, fy0, fx1, fy1) = contour_bounds(floor);
    assert_eq!((sx0, sy0, sx1, sy1), (57.0, 161.0, 138.0, 721.0));
    assert_eq!((fx0, fy0, fx1, fy1), (57.0, 0.0, 777.0, 81.0));
    let gap = sy0 - fy1;
    assert!(
        (60.0..=100.0).contains(&gap),
        "the L gap {gap} is outside 60-100 units"
    );
    assert_eq!(fx0, sx0, "the floor starts at the stem's left edge");
    assert_eq!(fx1, 777.0, "the floor ends at Orbitron's arm end");
    assert_eq!(fx1 - fx0, 720.0, "the floor keeps Orbitron's arm length");
    assert_eq!(fy1 - fy0, sx1 - sx0, "the floor is as thick as the stem");
    let id = face.glyph_index('L').expect("L is in the cmap");
    assert_eq!(face.glyph_hor_advance(id), Some(779));
}

/// ACC-THEME-154-NARROW-FONT at the default weight: every zero and L point is
/// the source point scaled 85 percent in x with y unchanged, and the L gap,
/// the floor and the advance follow the same rule.
#[test]
fn narrow_face_zero_and_l_are_the_scaled_source() {
    let narrow_bytes = font("GalaxaLabsTronNarrow-VF.ttf");
    let narrow = Face::parse(&narrow_bytes, 0).expect("the narrow font parses");
    let source_bytes = source_font("GalaxaLabsTron-VF.ttf");
    let source = Face::parse(&source_bytes, 0).expect("the source font parses");

    for ch in ['0', 'L'] {
        let scaled = contours(&narrow, ch);
        let original = contours(&source, ch);
        assert_eq!(scaled.len(), original.len(), "{ch} contour count");
        for (s, o) in scaled.iter().zip(&original) {
            assert_eq!(s.len(), o.len(), "{ch} point count");
            for ((sx, sy), (ox, oy)) in s.iter().zip(o) {
                assert_eq!(
                    *sx,
                    (0.85 * ox).round(),
                    "{ch} x is 85 percent of the source"
                );
                assert_eq!(*sy, *oy, "{ch} y is unchanged");
            }
        }
    }

    assert_eq!(bounds(&contours(&narrow, '0')), (38.0, 0.0, 615.0, 720.0));
    let l = contours(&narrow, 'L');
    let stem = l
        .iter()
        .find(|contour| contour_bounds(contour).1 > 0.0)
        .expect("the stem");
    let floor = l
        .iter()
        .find(|contour| contour_bounds(contour).1 == 0.0)
        .expect("the floor");
    let (sx0, sy0, sx1, sy1) = contour_bounds(stem);
    let (fx0, _, fx1, fy1) = contour_bounds(floor);
    assert_eq!((sx0, sy0, sx1, sy1), (48.0, 161.0, 117.0, 721.0));
    assert_eq!((fx0, fx1), (48.0, 660.0));
    assert_eq!(sy0 - fy1, 80.0, "the L gap survives the width scale");
    let id = narrow.glyph_index('L').expect("L is in the cmap");
    assert_eq!(narrow.glyph_hor_advance(id), Some(662));
}

/// ACC-THEME-170-PLUS-GLYPH: the plus is grown 1.22 about the centre of its
/// scaled default ink box, so its 10 px arms reach a pixel and it reads as a
/// plus; its bars stay centred and its advance is the untouched scaled source
/// advance, so no string reflows.
#[test]
fn narrow_face_plus_glyph_reads_as_a_plus() {
    let bytes = font("GalaxaLabsTronNarrow-VF.ttf");
    let source_bytes = source_font("GalaxaLabsTron-VF.ttf");

    for wght in [400.0, 900.0] {
        let mut face = Face::parse(&bytes, 0).expect("the narrow font parses");
        face.set_variation(Tag::from_bytes(b"wght"), wght)
            .expect("wght is an axis");

        let plus = contours(&face, '+');
        assert_eq!(plus.len(), 1, "the plus is one contour at {wght}");
        assert_eq!(plus[0].len(), 12, "the plus keeps 12 points at {wght}");

        let (x0, y0, x1, y1) = bounds(&plus);
        assert!(
            x1 - x0 >= 390.0,
            "the plus is {} units wide at {wght}, under 0.39 em",
            x1 - x0
        );
        assert!(
            y1 - y0 >= 460.0,
            "the plus is {} units tall at {wght}, under 0.46 em",
            y1 - y0
        );

        let centre = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (bx, by) = bar_centre(&plus);
        assert!(
            (bx - centre.0).abs() <= 2.0,
            "the bars sit {} units off the ink centre in x at {wght}",
            bx - centre.0
        );
        assert!(
            (by - centre.1).abs() <= 2.0,
            "the bars sit {} units off the ink centre in y at {wght}",
            by - centre.1
        );

        let mut source = Face::parse(&source_bytes, 0).expect("the source font parses");
        source
            .set_variation(Tag::from_bytes(b"wght"), wght)
            .expect("wght is an axis");
        let source_id = source.glyph_index('+').expect("+ is in the source cmap");
        let source_advance = source
            .glyph_hor_advance(source_id)
            .expect("an hmtx advance");
        let id = face.glyph_index('+').expect("+ is in the cmap");
        let advance = face.glyph_hor_advance(id).expect("an hmtx advance");
        // The plus rule leaves hmtx, HVAR and the phantom deltas alone, so the
        // advance is the source's scaled by 0.85: 368 at 400 and 387 at 900,
        // the 900 value carrying the source's own +19 HVAR delta. The page's
        // 10 px rows use 400, so no string reflows.
        assert_eq!(
            advance,
            (0.85 * f64::from(source_advance)).round() as u16,
            "the plus advance at {wght} is not the untouched scaled source advance"
        );
        if wght == 400.0 {
            assert_eq!(advance, 368, "the plus default advance is unchanged");
        }
    }
}

//! WCAG 2.1 AA contrast for every shipped btop theme pair, in both modes.
//!
//! Body text and selection text need 4.5:1; large text, focus, borders and
//! status colours need 3:1, except the Default panel borders, which stay the
//! muted Steel border (ACC-THEME-66-BOX-BORDERS). The raw btop keys are
//! checked for body text, and the semantic tokens the page reads are checked
//! for both floors.

use std::collections::BTreeMap;

use btop_web::theme::{DEFAULT_PANEL_BORDERS, Palette, embedded_pairs};
use palette::color_difference::{Ciede2000, Wcag21RelativeContrast};
use palette::{Hsl, IntoColor, Lab, Mix, Srgb};

type Rgb = [u8; 3];

fn contrast(one: Rgb, other: Rgb) -> f64 {
    fn srgb([red, green, blue]: Rgb) -> Srgb<f64> {
        Srgb::new(red, green, blue).into_format()
    }
    srgb(one).relative_contrast(srgb(other))
}

fn semantic(palette: &Palette) -> BTreeMap<&'static str, Rgb> {
    palette.semantic().into_iter().collect()
}

fn lab([red, green, blue]: Rgb) -> Lab {
    Srgb::new(red, green, blue)
        .into_format::<f32>()
        .into_color()
}

fn hex(color: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

/// CSS colour compositing in sRGB: `top` at `weight` over `bottom`.
fn mix(top: Rgb, bottom: Rgb, weight: f64) -> Rgb {
    [0, 1, 2].map(|channel| {
        (f64::from(top[channel]) * weight + f64::from(bottom[channel]) * (1.0 - weight)).round()
            as u8
    })
}

#[test]
fn every_shipped_pair_has_both_modes() {
    let pairs = embedded_pairs();
    let ids: Vec<&str> = pairs.iter().map(|pair| pair.id.as_str()).collect();
    for expected in [
        "default",
        "everforest-medium",
        "flexoki",
        "gruvbox",
        "solarized",
        "synthwave",
        "terracotta",
        "ultraviolet",
        "cyberpunk",
        "steel",
    ] {
        assert!(
            ids.contains(&expected),
            "theme {expected} missing; shipped: {ids:?}"
        );
    }
}

/// ACC-THEME-152-STEEL: the pair carries the lab's surfaces and text ramp in
/// both modes; the contrast tests below then hold it to the same floors.
#[test]
fn steel_pair_matches_the_lab_tokens() {
    let pair = embedded_pairs()
        .into_iter()
        .find(|pair| pair.id == "steel")
        .expect("the Steel pair ships");
    assert_eq!(pair.label, "Steel");
    assert_eq!(hex(pair.dark.color("main_bg")), "#252b32");
    assert_eq!(hex(pair.light.color("main_bg")), "#cdd0d6");
    let ramp = |palette: &Palette| {
        ["main_fg", "inactive_fg", "graph_text"]
            .map(|key| hex(palette.color(key)))
            .join(" ")
    };
    assert_eq!(ramp(&pair.dark), "#c3c3c3 #a5a5a5 #7d8791");
    assert_eq!(ramp(&pair.light), "#23262a #2c3035 #3d4148");
}

#[test]
fn body_text_reaches_4_5_to_1_in_every_pair_and_mode() {
    for pair in embedded_pairs() {
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let surface = palette.color("main_bg");
            for key in ["main_fg", "title"] {
                let ratio = contrast(palette.color(key), surface);
                assert!(
                    ratio >= 4.5,
                    "{} {mode}: {key} {} on {} is {ratio:.2}:1",
                    pair.id,
                    hex(palette.color(key)),
                    hex(surface)
                );
            }
        }
    }
}

/// ok, warn and crit paint process and GPU percentages as small text, so
/// every light variant reaches the 4.5:1 text floor (DEF-THEME-20-LIGHT-GREEN).
#[test]
fn light_status_tokens_reach_4_5_to_1() {
    for pair in embedded_pairs() {
        let colors = semantic(&pair.light);
        let surface = colors["surface"];
        for token in ["ok", "warn", "crit"] {
            let ratio = contrast(colors[token], surface);
            assert!(
                ratio >= 4.5,
                "{} light: {token} {} on {} is {ratio:.2}:1",
                pair.id,
                hex(colors[token]),
                hex(surface)
            );
        }
    }
}

#[test]
fn semantic_tokens_reach_their_wcag_floors_in_every_pair_and_mode() {
    for pair in embedded_pairs() {
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let colors = semantic(palette);
            let surface = colors["surface"];
            for token in ["text", "text-strong", "muted"] {
                let ratio = contrast(colors[token], surface);
                assert!(
                    ratio >= 4.5,
                    "{} {mode}: {token} {} on {} is {ratio:.2}:1",
                    pair.id,
                    hex(colors[token]),
                    hex(surface)
                );
            }
            for token in [
                "accent",
                "border",
                "border-cpu",
                "border-mem",
                "border-net",
                "border-proc",
                "ok",
                "warn",
                "crit",
                "chart-cpu",
                "chart-gpu-0",
                "chart-gpu-1",
                "chart-gpu-2",
                "chart-mem-used",
                "chart-mem-free",
                "chart-mem-cached",
                "chart-net-down",
                "chart-net-up",
                "chart-proc",
            ] {
                let ratio = contrast(colors[token], surface);
                assert!(
                    ratio >= 3.0,
                    "{} {mode}: {token} {} on {} is {ratio:.2}:1",
                    pair.id,
                    hex(colors[token]),
                    hex(surface)
                );
            }
            let ratio = contrast(colors["selection-fg"], colors["selection-bg"]);
            assert!(
                ratio >= 4.5,
                "{} {mode}: selection-fg {} on selection-bg {} is {ratio:.2}:1",
                pair.id,
                hex(colors["selection-fg"]),
                hex(colors["selection-bg"])
            );
        }
    }
}

/// DEF-VIEW-84-TIP-VISIBLE: the tooltip reads as an overlay on its own
/// surface, one lightness step off the panel at 1.25:1 or more, with the
/// accent border at 3:1 on both the fill and the panel and a two-tone ramp:
/// the strong first line and the muted source lines at 4.5:1 on the fill.
#[test]
fn tooltip_surface_separates_and_keeps_its_text_readable() {
    for pair in embedded_pairs() {
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let colors = semantic(palette);
            let panel = colors["surface"];
            let fill = colors["tip-surface"];
            let ratio = contrast(fill, panel);
            assert!(
                ratio >= 1.25,
                "{} {mode}: tip-surface {} on surface {} is {ratio:.2}:1",
                pair.id,
                hex(fill),
                hex(panel)
            );
            for base in [panel, fill] {
                let ratio = contrast(colors["tip-border"], base);
                assert!(
                    ratio >= 3.0,
                    "{} {mode}: tip-border {} on {} is {ratio:.2}:1",
                    pair.id,
                    hex(colors["tip-border"]),
                    hex(base)
                );
            }
            for token in ["tip-text", "tip-muted"] {
                let ratio = contrast(colors[token], fill);
                assert!(
                    ratio >= 4.5,
                    "{} {mode}: {token} {} on tip-surface {} is {ratio:.2}:1",
                    pair.id,
                    hex(colors[token]),
                    hex(fill)
                );
            }
            assert!(
                contrast(colors["tip-text"], fill) >= contrast(colors["tip-muted"], fill),
                "{} {mode}: the strong line is dimmer than the muted one",
                pair.id
            );
        }
    }
}

/// Text does not sit on the flat surface alone. Alternate process rows and
/// their hover paint a derived tint over it, the network arrows use derived
/// text variants of the raw graph colours, a house chip paints 12% of its
/// hue, and the config popover paints its controls on the raised surface.
/// Every text token on those layers must reach the 4.5:1 text floor, on the
/// hover through its row-hover token
/// (DEF-THEME-21-TINT-CONTRAST, DEF-THEME-22-POPOVER-CONTRAST).
#[test]
fn text_on_tinted_layers_reaches_4_5_to_1() {
    for pair in embedded_pairs() {
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let colors = semantic(palette);
            let surface = colors["surface"];
            for token in ["text", "muted", "ok", "warn", "crit"] {
                let ratio = contrast(colors[token], colors["zebra"]);
                assert!(
                    ratio >= 4.5,
                    "{} {mode}: {token} {} on zebra {} is {ratio:.2}:1",
                    pair.id,
                    hex(colors[token]),
                    hex(colors["zebra"])
                );
            }
            // ACC-VIEW-198-ROW-HOVER: the hovered row reaches 1.25:1 against
            // both rows it sits among. A hovered cell paints its own token
            // where that holds 4.5:1 on the step and --tip-text where it cannot.
            let hover = colors["row-hover"];
            for row in ["surface", "zebra"] {
                let ratio = contrast(hover, colors[row]);
                assert!(
                    ratio >= 1.25,
                    "{} {mode}: row-hover {} on {row} {} is {ratio:.2}:1",
                    pair.id,
                    hex(hover),
                    hex(colors[row])
                );
            }
            for (cell, token) in [
                ("row-hover-text", "text"),
                ("row-hover-muted", "muted"),
                ("row-hover-ok", "ok"),
                ("row-hover-warn", "warn"),
                ("row-hover-crit", "crit"),
            ] {
                let ratio = contrast(colors[cell], hover);
                assert!(
                    ratio >= 4.5,
                    "{} {mode}: {cell} {} on row-hover {} is {ratio:.2}:1",
                    pair.id,
                    hex(colors[cell]),
                    hex(hover)
                );
                let expected = if contrast(colors[token], hover) >= 4.5 {
                    colors[token]
                } else {
                    colors["tip-text"]
                };
                assert_eq!(
                    hex(colors[cell]),
                    hex(expected),
                    "{} {mode}: {cell} is neither its token nor the stronger one",
                    pair.id
                );
            }
            for token in ["net-down-text", "net-up-text"] {
                let ratio = contrast(colors[token], surface);
                assert!(
                    ratio >= 4.5,
                    "{} {mode}: {token} {} on {} is {ratio:.2}:1",
                    pair.id,
                    hex(colors[token]),
                    hex(surface)
                );
            }
            // The shipped chips tint with --ok (live), --warn (paused and
            // read-only), --accent (config) and --muted (the tier chip).
            for hue in ["ok", "warn", "accent", "muted"] {
                let layer = mix(colors[hue], surface, 0.12);
                let ratio = contrast(colors["chip-fg"], layer);
                assert!(
                    ratio >= 4.5,
                    "{} {mode}: chip-fg {} on 12% {hue} {} is {ratio:.2}:1",
                    pair.id,
                    hex(colors["chip-fg"]),
                    hex(layer)
                );
            }
            // The config popover and the panel menu are control surfaces on --surface-raised
            // (app.css blends it 2% toward --text); its labels and options
            // read --text, and the selected option paints 12% --accent over
            // the raised surface under a --chip-fg label.
            let raised = mix(colors["text"], surface, 0.02);
            let ratio = contrast(colors["text"], raised);
            assert!(
                ratio >= 4.5,
                "{} {mode}: text {} on surface-raised {} is {ratio:.2}:1",
                pair.id,
                hex(colors["text"]),
                hex(raised)
            );
            // The Processes filter paints --surface-raised as well, barely off
            // the bar (ACC-VIEW-187-FILTER-SEGMENT). --muted sits at its floor
            // on --surface, so the hint mixes 20% --text back in and stays
            // dimmer than the typed --text (ACC-VIEW-188-FILTER-HINT).
            let ratio = contrast(raised, surface);
            assert!(
                (1.02..=1.12).contains(&ratio),
                "{} {mode}: filter {} on the bar {} is {ratio:.3}:1",
                pair.id,
                hex(raised),
                hex(surface)
            );
            let hint = mix(colors["text"], colors["muted"], 0.2);
            let ratio = contrast(hint, raised);
            assert!(
                (4.5..contrast(colors["text"], raised)).contains(&ratio),
                "{} {mode}: filter hint {} on {} is {ratio:.2}:1",
                pair.id,
                hex(hint),
                hex(raised)
            );
            let selected = mix(colors["accent"], raised, 0.12);
            let ratio = contrast(colors["chip-fg"], selected);
            assert!(
                ratio >= 4.5,
                "{} {mode}: chip-fg {} on 12% accent over surface-raised {} is {ratio:.2}:1",
                pair.id,
                hex(colors["chip-fg"]),
                hex(raised)
            );
        }
    }
}

/// ACC-VIEW-172-GAUGE-TICKS: the inner ticks are stroked with `--surface`
/// at [`TICK_OPACITY`] (app.css `.g-tick.inner`), so the composite stays
/// faint on the grey track (`--meter-track` is `--border`) and on the ok,
/// warn and crit fills they cross, in every pair and mode: the measured
/// range is 1.37:1 to 2.00:1, above the plot grid's floor and far below the
/// 3:1 a chart colour carries. The raw surface keeps that 3:1 separation, so
/// the opacity alone sets how slight the tick reads.
const TICK_OPACITY: f64 = 0.3;

#[test]
fn gauge_inner_ticks_are_faint_on_the_track_and_every_fill() {
    for pair in embedded_pairs() {
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let colors = semantic(palette);
            let tick = colors["surface"];
            for token in ["border", "ok", "warn", "crit"] {
                let fill = colors[token];
                let ratio = contrast(tick, fill);
                assert!(
                    ratio >= 3.0,
                    "{} {mode}: the inner tick {} on {token} {} is {ratio:.2}:1",
                    pair.id,
                    hex(tick),
                    hex(fill)
                );
                let drawn = mix(tick, fill, TICK_OPACITY);
                let ratio = contrast(drawn, fill);
                assert!(
                    (1.25..=2.05).contains(&ratio),
                    "{} {mode}: the faded inner tick {} on {token} {} is {ratio:.2}:1",
                    pair.id,
                    hex(drawn),
                    hex(fill)
                );
            }
        }
    }
}

/// ACC-GRAPH-68-PLOT-GRID: the plot grid is very faint, just above the plot
/// surface and far below the 3:1 of a chart colour, but still visible.
#[test]
fn plot_grid_is_very_faint_in_every_pair_and_mode() {
    for pair in embedded_pairs() {
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let colors = semantic(palette);
            let ratio = contrast(colors["grid"], colors["surface"]);
            assert!(
                (1.1..=1.6).contains(&ratio),
                "{} {mode}: grid {} on surface {} is {ratio:.2}:1",
                pair.id,
                hex(colors["grid"]),
                hex(colors["surface"])
            );
        }
    }
}

/// ACC-GRAPH-99-LANE-BORDER: the GPU lane rule paints the panel-border token
/// at 55% over the surface (app.css), so it stays above the plot grid and
/// below every data line and text token in every pair and mode, and inside
/// the measured 1.65-2.42:1 panel-border band in Default (DEF-VIEW-60).
#[test]
fn gpu_lane_border_sits_between_the_grid_and_the_text() {
    const LANE_BORDER_WEIGHT: f64 = 0.55;
    for pair in embedded_pairs() {
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let colors = semantic(palette);
            let surface = colors["surface"];
            let rule = mix(colors["border"], surface, LANE_BORDER_WEIGHT);
            let ratio = contrast(rule, surface);
            let grid = contrast(colors["grid"], surface);
            assert!(
                ratio > grid,
                "{} {mode}: lane border {} on {} is {ratio:.2}:1, at or below the grid's {grid:.2}:1",
                pair.id,
                hex(rule),
                hex(surface)
            );
            for token in [
                "chart-gpu-0",
                "chart-gpu-1",
                "chart-gpu-2",
                "text",
                "text-strong",
            ] {
                let line = contrast(colors[token], surface);
                assert!(
                    ratio < line,
                    "{} {mode}: lane border {} on {} is {ratio:.2}:1, at or above {token}'s {line:.2}:1",
                    pair.id,
                    hex(rule),
                    hex(surface)
                );
            }
            if pair.id == "default" {
                assert!(
                    (1.65..=2.42).contains(&ratio),
                    "{} {mode}: lane border {} on {} is {ratio:.2}:1, outside the panel-border band",
                    pair.id,
                    hex(rule),
                    hex(surface)
                );
            }
        }
    }
}

/// ACC-GRAPH-43-GPU-LOAD: the three GPU series sit on one plot, so each is a
/// graphic at 3:1 on the surface and every pair differs by CIEDE2000 above 20
/// (24.1 is the measured minimum over the ten variants, Default dark).
#[test]
fn gpu_series_tokens_are_distinct_and_reach_3_to_1() {
    const SERIES: [&str; 3] = ["chart-gpu-0", "chart-gpu-1", "chart-gpu-2"];
    for pair in embedded_pairs() {
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let colors = semantic(palette);
            for token in SERIES {
                let ratio = contrast(colors[token], colors["surface"]);
                assert!(
                    ratio >= 3.0,
                    "{} {mode}: {token} {} on surface {} is {ratio:.2}:1",
                    pair.id,
                    hex(colors[token]),
                    hex(colors["surface"])
                );
            }
            for (first, second) in [(0, 1), (0, 2), (1, 2)] {
                let (one, other) = (colors[SERIES[first]], colors[SERIES[second]]);
                let difference = lab(one).difference(lab(other));
                assert!(
                    difference > 20.0,
                    "{} {mode}: {} {} and {} {} differ by {difference:.1}",
                    pair.id,
                    SERIES[first],
                    hex(one),
                    SERIES[second],
                    hex(other)
                );
            }
        }
    }
}

/// ACC-VIEW-190-MEM-RECLAIM: the memory segments as app.css mixes them for the
/// bar and the keys, in table order, with free the track colour. The gauge arc
/// draws the first two against the track (ACC-VIEW-192-GAUGE-CONTAINER).
fn memory_segments(colors: &BTreeMap<&'static str, Rgb>) -> [(&'static str, Rgb); 5] {
    const BLACK: Rgb = [0, 0, 0];
    const WHITE: Rgb = [255, 255, 255];
    [
        ("used", mix(colors["ok"], BLACK, 0.80)),
        ("cache", mix(colors["ok"], WHITE, 0.50)),
        ("other", mix(colors["chart-mem-cached"], BLACK, 0.75)),
        ("other-cache", mix(colors["chart-mem-cached"], WHITE, 0.40)),
        ("free", colors["border"]),
    ]
}

/// ACC-VIEW-190-MEM-RECLAIM: each segment reaches 1.5:1 against the next, in
/// table order, and the reclaimable tone is lighter than its owner's colour.
/// A row that reads zero puts the two reclaimable tones beside the bare track,
/// and those pairs reach 1.5:1 too.
#[test]
fn memory_segments_reach_1_5_to_1_against_their_neighbours() {
    for pair in embedded_pairs() {
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let colors = semantic(palette);
            let segments = memory_segments(&colors);
            let neighbours = segments.windows(2).map(|two| (two[0], two[1]));
            let beside_track = [(segments[1], segments[4])];
            for (one, other) in neighbours.chain(beside_track) {
                let ratio = contrast(one.1, other.1);
                assert!(
                    ratio >= 1.5,
                    "{} {mode}: {} {} and {} {} are {ratio:.2}:1",
                    pair.id,
                    one.0,
                    hex(one.1),
                    other.0,
                    hex(other.1)
                );
            }
            let lightness = |color: Rgb| lab(color).l;
            for (base, tone) in [(0, 1), (2, 3)] {
                assert!(
                    lightness(segments[tone].1) > lightness(segments[base].1),
                    "{} {mode}: {} is not lighter than {}",
                    pair.id,
                    segments[tone].0,
                    segments[base].0
                );
            }
        }
    }
}

/// ACC-VIEW-56-CORE-COLOUR: a core value is drawn in the cpu gradient colour
/// at its value, which app.css mixes in linear light from --ok to --warn over
/// 0-50 and from --warn to --crit over 50-100. Every value 0-100 reaches the
/// 4.5:1 text floor on the surface in every variant.
#[test]
fn core_gradient_text_reaches_4_5_to_1_at_every_value() {
    fn linear_mix(from: Rgb, to: Rgb, weight: f64) -> Rgb {
        let linear = |[red, green, blue]: Rgb| {
            Srgb::new(red, green, blue)
                .into_format::<f64>()
                .into_linear()
        };
        let mixed: Srgb<f64> = Srgb::from_linear(linear(from).mix(linear(to), weight));
        let mixed: Srgb<u8> = mixed.into_format();
        [mixed.red, mixed.green, mixed.blue]
    }
    for pair in embedded_pairs() {
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let colors = semantic(palette);
            let surface = colors["surface"];
            for value in 0..=100u8 {
                let color = if value <= 50 {
                    linear_mix(colors["ok"], colors["warn"], f64::from(value) / 50.0)
                } else {
                    linear_mix(colors["warn"], colors["crit"], f64::from(value - 50) / 50.0)
                };
                let ratio = contrast(color, surface);
                assert!(
                    ratio >= 4.5,
                    "{} {mode}: core value {value} {} on {} is {ratio:.2}:1",
                    pair.id,
                    hex(color),
                    hex(surface)
                );
            }
        }
    }
}

/// ACC-THEME-66-BOX-BORDERS: each Default panel border is the Steel border
/// (dark `#4d5a65`, light `#b7bcc3`) with a small hue shift: hue within 35
/// degrees, HSL saturation at most 25% and within 12 points, lightness within
/// 5 points of the base. The four families differ by CIEDE2000 4 or more, and
/// cores, GPU and storage by 5 or more from every other panel border: removing
/// a panel (or a host without a GPU) lets them touch any family.
#[test]
fn default_panel_borders_are_muted_steel() {
    fn hsl([red, green, blue]: Rgb) -> Hsl {
        Srgb::new(red, green, blue)
            .into_format::<f32>()
            .into_color()
    }
    const FAMILIES: [&str; 4] = ["border-cpu", "border-mem", "border-net", "border-proc"];
    const ALL: [&str; 7] = [
        "border-cpu",
        "border-mem",
        "border-net",
        "border-proc",
        "border-cores",
        "border-gpu",
        "border-storage",
    ];
    const OWN: [&str; 3] = ["border-cores", "border-gpu", "border-storage"];
    for (mode, base) in [(0, [0x4d, 0x5a, 0x65]), (1, [0xb7, 0xbc, 0xc3])] {
        let colors: BTreeMap<&str, Rgb> = DEFAULT_PANEL_BORDERS
            .iter()
            .map(|(name, values)| (*name, values[mode]))
            .collect();
        let steel = hsl(base);
        for (token, color) in &colors {
            let border = hsl(*color);
            let hue = (border.hue - steel.hue).into_degrees().abs();
            let saturation = border.saturation * 100.0;
            let lightness = (border.lightness - steel.lightness).abs() * 100.0;
            assert!(
                hue <= 35.0
                    && saturation <= 25.0
                    && (saturation - steel.saturation * 100.0).abs() <= 12.0
                    && lightness <= 5.0,
                "mode {mode}: {token} {} is hue {hue:.1} deg, saturation {saturation:.1}%, \
                 lightness {lightness:.1} points from {}",
                hex(*color),
                hex(base)
            );
        }
        let pairs = FAMILIES
            .iter()
            .enumerate()
            .flat_map(|(index, one)| {
                FAMILIES[index + 1..]
                    .iter()
                    .map(move |other| (*one, *other, 4.0))
            })
            .chain(OWN.iter().flat_map(|panel| {
                ALL.iter()
                    .filter(move |other| *other != panel)
                    .map(move |other| (*panel, *other, 5.0))
            }));
        for (one, other, floor) in pairs {
            let difference = lab(colors[one]).difference(lab(colors[other]));
            assert!(
                difference >= floor,
                "mode {mode}: {one} {} and {other} {} differ by {difference:.1}",
                hex(colors[one]),
                hex(colors[other])
            );
        }
    }
}

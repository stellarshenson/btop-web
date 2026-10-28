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
    ] {
        assert!(
            ids.contains(&expected),
            "theme {expected} missing; shipped: {ids:?}"
        );
    }
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
                "chart-mem-used",
                "chart-mem-free",
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

/// Text does not sit on the flat surface alone. Alternate process rows and
/// their hover paint a derived tint over it, the network arrows use derived
/// text variants of the raw graph colours, a house chip paints 12% of its
/// hue, and the config popover paints its controls on the raised surface.
/// Every text token on those layers must reach the 4.5:1 text floor
/// (DEF-THEME-21-TINT-CONTRAST, DEF-THEME-22-POPOVER-CONTRAST).
#[test]
fn text_on_tinted_layers_reaches_4_5_to_1() {
    for pair in embedded_pairs() {
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let colors = semantic(palette);
            let surface = colors["surface"];
            for layer in ["zebra", "row-hover"] {
                for token in ["text", "muted", "ok", "warn", "crit"] {
                    let ratio = contrast(colors[token], colors[layer]);
                    assert!(
                        ratio >= 4.5,
                        "{} {mode}: {token} {} on {layer} {} is {ratio:.2}:1",
                        pair.id,
                        hex(colors[token]),
                        hex(colors[layer])
                    );
                }
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

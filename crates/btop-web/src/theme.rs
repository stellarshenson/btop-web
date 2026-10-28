//! btop `.theme` parsing and CSS custom-property rendering.
//!
//! Themes are read from the embedded `themes/` directory. Each file is paired
//! with its light or dark counterpart by name; a missing counterpart is
//! derived by inverting colour lightness. The built-in Default pair is always
//! available, so the page renders before any theme file exists.
//!
//! WCAG 2.1 relative luminance and contrast come from `palette`, so the theme
//! and its tests share one maintained implementation.

use std::collections::BTreeMap;

use palette::color_difference::Wcag21RelativeContrast;
use palette::{FromColor, Hsl, Mix, Srgb};
use rust_embed::RustEmbed;

type Rgb = [u8; 3];

/// The 48 keys of btop's theme format, in btop's own order.
pub const THEME_KEYS: [&str; 48] = [
    "main_bg",
    "main_fg",
    "title",
    "hi_fg",
    "selected_bg",
    "selected_fg",
    "inactive_fg",
    "graph_text",
    "meter_bg",
    "proc_misc",
    "cpu_box",
    "mem_box",
    "net_box",
    "proc_box",
    "div_line",
    "temp_start",
    "temp_mid",
    "temp_end",
    "cpu_start",
    "cpu_mid",
    "cpu_end",
    "free_start",
    "free_mid",
    "free_end",
    "cached_start",
    "cached_mid",
    "cached_end",
    "available_start",
    "available_mid",
    "available_end",
    "used_start",
    "used_mid",
    "used_end",
    "download_start",
    "download_mid",
    "download_end",
    "upload_start",
    "upload_mid",
    "upload_end",
    "process_start",
    "process_mid",
    "process_end",
    "proc_pause_bg",
    "proc_follow_bg",
    "proc_banner_bg",
    "proc_banner_fg",
    "followed_bg",
    "followed_fg",
];

/// btop's `Default_theme`.
const DEFAULT_DARK: [(&str, &str); 48] = [
    ("main_bg", "#00"),
    ("main_fg", "#cc"),
    ("title", "#ee"),
    ("hi_fg", "#b54040"),
    ("selected_bg", "#6a2f2f"),
    ("selected_fg", "#ee"),
    ("inactive_fg", "#40"),
    ("graph_text", "#60"),
    ("meter_bg", "#40"),
    ("proc_misc", "#0de756"),
    ("cpu_box", "#556d59"),
    ("mem_box", "#6c6c4b"),
    ("net_box", "#5c588d"),
    ("proc_box", "#805252"),
    ("div_line", "#30"),
    ("temp_start", "#4897d4"),
    ("temp_mid", "#5474e8"),
    ("temp_end", "#ff40b6"),
    ("cpu_start", "#77ca9b"),
    ("cpu_mid", "#cbc06c"),
    ("cpu_end", "#dc4c4c"),
    ("free_start", "#384f21"),
    ("free_mid", "#b5e685"),
    ("free_end", "#dcff85"),
    ("cached_start", "#163350"),
    ("cached_mid", "#74e6fc"),
    ("cached_end", "#26c5ff"),
    ("available_start", "#4e3f0e"),
    ("available_mid", "#ffd77a"),
    ("available_end", "#ffb814"),
    ("used_start", "#592b26"),
    ("used_mid", "#d9626d"),
    ("used_end", "#ff4769"),
    ("download_start", "#291f75"),
    ("download_mid", "#4f43a3"),
    ("download_end", "#b0a9de"),
    ("upload_start", "#620665"),
    ("upload_mid", "#7d4180"),
    ("upload_end", "#dcafde"),
    ("process_start", "#80d0a3"),
    ("process_mid", "#dcd179"),
    ("process_end", "#d45454"),
    ("proc_pause_bg", "#b54040"),
    ("proc_follow_bg", "#4040b5"),
    ("proc_banner_bg", "#7b407b"),
    ("proc_banner_fg", "#ee"),
    ("followed_bg", "#4040b5"),
    ("followed_fg", "#ee"),
];

/// ACC-THEME-65-STEEL-DEFAULT: the built-in Default dark, laid over btop's
/// table. `DEFAULT_DARK` stays btop's, because `with_defaults` fills the keys
/// a theme file omits from it. Neutrals come from the Galaxa Steel Dark
/// JupyterLab theme: page `#272b31` (owner; `--jp-layout-color0` `#252b32`),
/// text `--jp-ui-font-color0`, inner lines `--jp-border-color1`. The next ten
/// are btop keys above 70% HSL saturation capped to 70%, hue and lightness kept.
/// The last two replace btop's two violet network line keys with a muted steel
/// blue for download and a muted amber for upload, CIEDE2000 above 30 apart in
/// both modes (ACC-GRAPH-72-NET-COLOURS); `download_mid` stays btop's, because
/// GPU series 1 reads it, and the other download and upload keys have no reader.
const STEEL_DARK: [(&str, &str); 15] = [
    ("main_bg", "#272b31"),
    ("main_fg", "#c3c3c3"),
    ("div_line", "#404b54"),
    ("proc_misc", "#25cf5e"),
    ("temp_mid", "#5a77e2"),
    ("temp_end", "#e25daf"),
    ("free_end", "#d4ed97"),
    ("cached_mid", "#86daea"),
    ("cached_end", "#47b6de"),
    ("available_mid", "#ebcf8e"),
    ("available_end", "#dcaa37"),
    ("used_end", "#e3637a"),
    ("upload_start", "#59105b"),
    ("download_end", "#86acd6"),
    ("upload_end", "#d9a978"),
];

/// The built-in Default light, laid over the inverted Steel dark: the
/// JupyterLab light theme's page (`--jp-layout-color0` white) and strong text
/// (`--jp-ui-font-color0` `#000`), Steel Light's text `#23262a`, and inner lines
/// at JupyterLab's `--jp-border-color2` lightness with Steel Light's
/// `--jp-layout-color2` hue and saturation.
const STEEL_LIGHT: [(&str, &str); 4] = [
    ("main_bg", "#ffffff"),
    ("main_fg", "#23262a"),
    ("title", "#000000"),
    ("div_line", "#dde0e3"),
];

fn parse_table(table: &[(&str, &str)]) -> BTreeMap<String, Rgb> {
    table
        .iter()
        .filter_map(|(key, value)| parse_color(value).map(|rgb| (key.to_string(), rgb)))
        .collect()
}

#[derive(RustEmbed)]
#[folder = "themes/"]
struct ThemeAsset;

/// A parsed theme file: the colours it defines, keyed by btop's names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Palette {
    colors: BTreeMap<String, Rgb>,
}

impl Palette {
    /// Parse `theme[key] = "value"` lines. Values are `#rrggbb`, `#hh` or
    /// `'r g b'`; `#` starts a comment; unknown keys are ignored.
    ///
    /// Kept local: btop's reader scans to `[`, takes the name up to `]`, then
    /// a quoted value or the rest of the line, and ignores unknown names.
    /// rust-ini 0.21.3 parses `theme[key]` as a bare key but applies its own
    /// section and comment rules and no btop validation. configparser 3.3.0
    /// and `ini` 1.3.0 (a macro over configparser 1.0.0) treat the `#` of
    /// every hex colour as a comment and truncate the value; they also fold
    /// key case and keep the quotes. None knows the `theme[...]` key syntax or
    /// the three colour forms without re-implementing the scan around it.
    pub fn parse(text: &str) -> Result<Self, ThemeError> {
        let mut colors = BTreeMap::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(rest) = line.strip_prefix("theme[") else {
                continue;
            };
            let Some((key, rest)) = rest.split_once(']') else {
                return Err(ThemeError::Syntax { line: index + 1 });
            };
            let Some(value) = rest.trim().strip_prefix('=') else {
                return Err(ThemeError::Syntax { line: index + 1 });
            };
            let value = value.trim().trim_matches(['"', '\'']);
            if value.is_empty() || !THEME_KEYS.contains(&key.trim()) {
                continue;
            }
            let rgb = parse_color(value).ok_or_else(|| ThemeError::InvalidValue {
                line: index + 1,
                value: value.to_string(),
            })?;
            colors.insert(key.trim().to_string(), rgb);
        }
        Ok(Self { colors })
    }

    /// btop's Default theme, with every key present.
    pub fn default_dark() -> Self {
        Self {
            colors: parse_table(&DEFAULT_DARK),
        }
    }

    /// Fill keys the file omitted from the Default theme, as btop does.
    pub fn with_defaults(&self) -> Self {
        let mut colors = Self::default_dark().colors;
        for (key, rgb) in &self.colors {
            colors.insert(key.clone(), *rgb);
        }
        Self { colors }
    }

    /// The same hues with every lightness inverted, used when a pair is
    /// missing one mode.
    pub fn inverted(&self) -> Self {
        Self {
            colors: self
                .colors
                .iter()
                .map(|(key, rgb)| (key.clone(), invert_lightness(*rgb)))
                .collect(),
        }
    }

    pub fn color(&self, key: &str) -> Rgb {
        self.colors.get(key).copied().unwrap_or_default()
    }

    /// The semantic tokens components read. Raw btop keys stay available for
    /// meters that follow btop's gradients.
    ///
    /// btop paints on a terminal cell, so several of its colours are close to
    /// the background (`inactive_fg`, `div_line`) or low-contrast in the light
    /// variants. The values here are the btop colour moved towards black or
    /// white only as far as the web contrast floor requires: 4.5:1 for text,
    /// 3:1 for focus and borders. The raw keys are not changed.
    pub fn semantic(&self) -> Vec<(&'static str, Rgb)> {
        let surface = self.color("main_bg");
        let text = ensure_contrast(self.color("main_fg"), surface, TEXT_CONTRAST);
        let text_strong = ensure_contrast(self.color("title"), surface, TEXT_CONTRAST);
        let selection_bg = self.color("selected_bg");
        let accent = ensure_contrast(self.color("hi_fg"), surface, UI_CONTRAST);
        let border = ensure_contrast(self.color("div_line"), surface, UI_CONTRAST);
        // ok, warn and crit paint process percentages as small text in both
        // modes, so a dark surface needs the text floor too. They are also the
        // core value gradient's stops (ACC-VIEW-56-CORE-COLOUR), hence the
        // rounding margin.
        let ok = ensure_contrast(self.color("cpu_start"), surface, GRADIENT_TEXT_CONTRAST);
        let warn = ensure_contrast(self.color("cpu_mid"), surface, GRADIENT_TEXT_CONTRAST);
        let crit = ensure_contrast(self.color("cpu_end"), surface, GRADIENT_TEXT_CONTRAST);
        // A chip paints 12% of its hue over the surface, so its label is
        // clamped against the hue tint that leaves the least contrast.
        let chip_tint = [ok, warn, accent]
            .into_iter()
            .map(|hue| blend(surface, hue, CHIP_TINT))
            .min_by(|one, other| {
                contrast_ratio(text_strong, *one).total_cmp(&contrast_ratio(text_strong, *other))
            })
            .unwrap_or(surface);
        // Alternate rows and their hover tint away from the text colour, so
        // they separate without spending contrast. A pure black or white
        // surface has no room in that direction and carries no row tint.
        let away = if contrast_ratio([0, 0, 0], surface) >= contrast_ratio([255, 255, 255], surface)
        {
            [255, 255, 255]
        } else {
            [0, 0, 0]
        };
        vec![
            ("surface", surface),
            ("text", text),
            ("text-strong", text_strong),
            ("muted", dim_towards(text, surface, TEXT_CONTRAST)),
            ("accent", accent),
            ("selection-bg", selection_bg),
            (
                "selection-fg",
                ensure_contrast(self.color("selected_fg"), selection_bg, TEXT_CONTRAST),
            ),
            ("border", border),
            (
                "border-cpu",
                ensure_contrast(self.color("cpu_box"), surface, UI_CONTRAST),
            ),
            (
                "border-mem",
                ensure_contrast(self.color("mem_box"), surface, UI_CONTRAST),
            ),
            (
                "border-net",
                ensure_contrast(self.color("net_box"), surface, UI_CONTRAST),
            ),
            (
                "border-proc",
                ensure_contrast(self.color("proc_box"), surface, UI_CONTRAST),
            ),
            ("ok", ok),
            ("warn", warn),
            ("crit", crit),
            (
                "chip-fg",
                ensure_contrast(text_strong, chip_tint, TEXT_CONTRAST),
            ),
            ("zebra", blend(surface, away, ZEBRA_TINT)),
            ("row-hover", blend(surface, away, HOVER_TINT)),
            (
                "net-down-text",
                ensure_contrast(self.color("download_end"), surface, TEXT_CONTRAST),
            ),
            (
                "net-up-text",
                ensure_contrast(self.color("upload_end"), surface, TEXT_CONTRAST),
            ),
            // The GPU load plot's three series (ACC-GRAPH-43-GPU-LOAD): btop
            // draws GPU graphs with the cpu gradient, so series 0 is cpu_mid;
            // download_mid and temp_end keep CIEDE2000 above 20 between all
            // three in every shipped variant.
            (
                "chart-gpu-0",
                ensure_contrast(self.color("cpu_mid"), surface, UI_CONTRAST),
            ),
            (
                "chart-gpu-1",
                ensure_contrast(self.color("download_mid"), surface, UI_CONTRAST),
            ),
            (
                "chart-gpu-2",
                ensure_contrast(self.color("temp_end"), surface, UI_CONTRAST),
            ),
            (
                "chart-cpu",
                ensure_contrast(self.color("cpu_start"), surface, UI_CONTRAST),
            ),
            (
                "chart-mem-used",
                ensure_contrast(self.color("used_end"), surface, UI_CONTRAST),
            ),
            (
                "chart-mem-free",
                ensure_contrast(self.color("free_end"), surface, UI_CONTRAST),
            ),
            (
                "chart-net-down",
                ensure_contrast(self.color("download_end"), surface, UI_CONTRAST),
            ),
            (
                "chart-net-up",
                ensure_contrast(self.color("upload_end"), surface, UI_CONTRAST),
            ),
            // The Processes tile's count graph (ACC-VIEW-53-PROC-TILE).
            (
                "chart-proc",
                ensure_contrast(self.color("process_mid"), surface, UI_CONTRAST),
            ),
            // ACC-GRAPH-68-PLOT-GRID: very faint, just above the plot surface.
            ("grid", blend(surface, text, GRID_TINT)),
        ]
    }
}

/// One theme, always carrying both modes.
#[derive(Debug, Clone)]
pub struct ThemePair {
    pub id: String,
    pub label: String,
    pub dark: Palette,
    pub light: Palette,
}

impl ThemePair {
    pub fn default_theme() -> Self {
        let mut dark = Palette::default_dark();
        dark.colors.extend(parse_table(&STEEL_DARK));
        let mut light = dark.inverted();
        light.colors.extend(parse_table(&STEEL_LIGHT));
        Self {
            id: "default".to_string(),
            label: "Default".to_string(),
            dark,
            light,
        }
    }

    pub fn choice(&self) -> ThemeChoice {
        ThemeChoice {
            id: self.id.clone(),
            label: self.label.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeChoice {
    pub id: String,
    pub label: String,
}

/// Every theme in the embedded `themes/` directory, paired with its
/// counterpart, plus the built-in Default.
pub fn embedded_pairs() -> Vec<ThemePair> {
    let mut grouped: BTreeMap<String, (Option<Palette>, Option<Palette>)> = BTreeMap::new();
    for path in ThemeAsset::iter() {
        if !path.ends_with(".theme") {
            continue;
        }
        let Some(file) = ThemeAsset::get(&path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&file.data);
        let palette = match Palette::parse(&text) {
            Ok(palette) => palette.with_defaults(),
            Err(error) => {
                tracing::warn!(theme = %path, %error, "skipping theme file");
                continue;
            }
        };
        let Some((id, mode)) = split_mode(path.trim_end_matches(".theme")) else {
            tracing::warn!(theme = %path, "skipping theme name without a dark or light token");
            continue;
        };
        let entry = grouped.entry(id).or_default();
        match mode {
            Mode::Dark => entry.0 = Some(palette),
            Mode::Light => entry.1 = Some(palette),
        }
    }

    let mut pairs = vec![ThemePair::default_theme()];
    for (id, (dark, light)) in grouped {
        let (dark, light) = match (dark, light) {
            (Some(dark), Some(light)) => (dark, light),
            (Some(dark), None) => {
                let light = dark.inverted();
                (dark, light)
            }
            (None, Some(light)) => {
                let dark = light.inverted();
                (dark, light)
            }
            (None, None) => continue,
        };
        pairs.push(ThemePair {
            label: label_for(&id),
            id,
            dark,
            light,
        });
    }
    pairs
}

/// ACC-THEME-66-BOX-BORDERS: the Default panel borders, `[dark, light]`. Each
/// is the Steel border at its lightness with HSL saturation 20% and a hue
/// shift: dark base Steel Dark `--jp-border-color0` `#4d5a65`, light base
/// `#b7bcc3` (JupyterLab light `--jp-border-color1` lightness at Steel Light's
/// `--jp-layout-color2` hue). The families shift cpu -30, mem -10, net +10 and
/// proc +30 degrees. Cores, GPU and storage can touch any family once a panel
/// is removed, and the families fill the hue range at saturation 20%, so these
/// three keep the hue of their fallback family (cpu, proc, mem) at saturation
/// 2-7% with a lightness step of about 4 points: cores and GPU lighter, storage
/// darker. Each is CIEDE2000 5.9 or more from every other panel border. No
/// contrast floor: each panel is named by its title.
pub const DEFAULT_PANEL_BORDERS: [(&str, [Rgb; 2]); 7] = [
    ("border-cpu", [[0x47, 0x6b, 0x69], [0xb0, 0xc8, 0xca]]),
    ("border-mem", [[0x47, 0x60, 0x6b], [0xb0, 0xbf, 0xca]]),
    ("border-net", [[0x47, 0x55, 0x6b], [0xb0, 0xb6, 0xca]]),
    ("border-proc", [[0x47, 0x49, 0x6b], [0xb2, 0xb0, 0xca]]),
    ("border-cores", [[0x5f, 0x68, 0x67], [0xc4, 0xca, 0xcb]]),
    ("border-gpu", [[0x5f, 0x5f, 0x68], [0xc3, 0xc2, 0xca]]),
    ("border-storage", [[0x4c, 0x50, 0x51], [0xb1, 0xb3, 0xb4]]),
];

/// CSS for every theme and both modes. The Default dark palette also lands on
/// `:root`, so the page is coloured even before `theme.js` picks a theme.
pub fn render_css(pairs: &[ThemePair]) -> String {
    let mut css = String::new();
    for pair in pairs {
        let dark = format!("[data-theme=\"{}\"][data-mode=\"dark\"]", pair.id);
        let light = format!("[data-theme=\"{}\"][data-mode=\"light\"]", pair.id);
        if pair.id == "default" {
            css.push_str(&block(&format!(":root, {dark}"), &pair.dark));
        } else {
            css.push_str(&block(&dark, &pair.dark));
        }
        css.push_str(&block(&light, &pair.light));
        // Not on `:root`, so in the other themes app.css falls back from the
        // per-panel tokens to the family token.
        if pair.id == "default" {
            for (mode, selector) in [dark, light].iter().enumerate() {
                css.push_str(selector);
                css.push('{');
                for (name, colors) in DEFAULT_PANEL_BORDERS {
                    css.push_str(&format!("--{name}:{};", hex(colors[mode])));
                }
                css.push_str("}\n");
            }
        }
    }
    css
}

fn block(selector: &str, palette: &Palette) -> String {
    let mut css = String::new();
    css.push_str(selector);
    css.push('{');
    for (key, rgb) in &palette.colors {
        css.push_str(&format!("--{}:{};", key.replace('_', "-"), hex(*rgb)));
    }
    for (name, rgb) in palette.semantic() {
        css.push_str(&format!("--{name}:{};", hex(rgb)));
    }
    css.push_str("}\n");
    css
}

// One format! call; palette's LowerHex gives the same string only through an Srgb conversion.
fn hex([red, green, blue]: Rgb) -> String {
    format!("#{red:02x}{green:02x}{blue:02x}")
}

// Own parser: btop takes #hh, #rrggbb and 'r g b'; palette's FromStr takes 3 or 6 digits only.
fn parse_color(value: &str) -> Option<Rgb> {
    if let Some(hex) = value.strip_prefix('#') {
        return match hex.len() {
            2 => {
                let level = u8::from_str_radix(hex, 16).ok()?;
                Some([level, level, level])
            }
            6 => Some([
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            ]),
            _ => None,
        };
    }
    let parts: Vec<&str> = value.split_whitespace().collect();
    let [red, green, blue] = parts.as_slice() else {
        return None;
    };
    Some([red.parse().ok()?, green.parse().ok()?, blue.parse().ok()?])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Dark,
    Light,
}

/// Split `gruvbox_dark` or `everforest-light-medium` into a base id and mode.
fn split_mode(stem: &str) -> Option<(String, Mode)> {
    let normalized = stem.to_ascii_lowercase();
    let tokens: Vec<&str> = normalized
        .split(['-', '_'])
        .filter(|token| !token.is_empty())
        .collect();
    let index = tokens
        .iter()
        .position(|token| *token == "dark" || *token == "light")?;
    let mode = if tokens[index] == "dark" {
        Mode::Dark
    } else {
        Mode::Light
    };
    let id = tokens
        .iter()
        .enumerate()
        .filter(|(position, _)| *position != index)
        .map(|(_, token)| *token)
        .collect::<Vec<_>>()
        .join("-");
    (!id.is_empty()).then_some((id, mode))
}

// Own code: no title-case crate is in the tree, and btop has no theme label to copy.
fn label_for(id: &str) -> String {
    id.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// WCAG AA floors: 4.5:1 for body text, 3:1 for large text and UI.
const TEXT_CONTRAST: f64 = 4.5;
const UI_CONTRAST: f64 = 3.0;
/// app.css mixes ok, warn and crit in linear light for each core value, and
/// the browser paints the mix in 8-bit sRGB: rounding a channel costs up to
/// about 0.03 of ratio, so the stops sit this far above the text floor.
const GRADIENT_TEXT_CONTRAST: f64 = TEXT_CONTRAST + 0.05;
/// Margin so an independent contrast computation cannot round below the floor.
const CONTRAST_MARGIN: f64 = 0.01;
/// Tint weights the stylesheet paints over the surface: the house chip's hue
/// at 12%, the alternate process row at 3% and its hover at 7%.
const CHIP_TINT: f64 = 0.12;
const ZEBRA_TINT: f64 = 0.03;
const HOVER_TINT: f64 = 0.07;
/// The plot grid: the text colour at 12% over the surface, 1.1 to 1.6:1.
const GRID_TINT: f64 = 0.12;

/// Move `rgb` towards black or white until it reaches `ratio` against
/// `background`. The direction that needs the shorter move is chosen by which
/// of black and white contrasts more with the background.
// Own search: palette measures a WCAG ratio, but no crate in the tree solves for one.
fn ensure_contrast(rgb: Rgb, background: Rgb, ratio: f64) -> Rgb {
    let target = ratio + CONTRAST_MARGIN;
    if contrast_ratio(rgb, background) >= target {
        return rgb;
    }
    let toward =
        if contrast_ratio([0, 0, 0], background) >= contrast_ratio([255, 255, 255], background) {
            [0, 0, 0]
        } else {
            [255, 255, 255]
        };
    for step in 1..=255u16 {
        let candidate = blend(rgb, toward, f64::from(step) / 255.0);
        if contrast_ratio(candidate, background) >= target {
            return candidate;
        }
    }
    toward
}

/// The dimmest blend of `rgb` towards `background` that still reaches `ratio`.
/// Used for muted text, where btop's `inactive_fg` is often the background hue.
// Own search, as ensure_contrast: palette measures a ratio but does not solve for one.
fn dim_towards(rgb: Rgb, background: Rgb, ratio: f64) -> Rgb {
    let floor = ratio + CONTRAST_MARGIN;
    let mut best = rgb;
    for step in 1..=255u16 {
        let candidate = blend(rgb, background, f64::from(step) / 255.0);
        if contrast_ratio(candidate, background) < floor {
            break;
        }
        best = candidate;
    }
    best
}

fn blend(from: Rgb, to: Rgb, weight: f64) -> Rgb {
    let srgb = |rgb: Rgb| Srgb::from(rgb).into_format::<f64>();
    srgb(from).mix(srgb(to), weight).into_format::<u8>().into()
}

/// WCAG 2.1 contrast between two sRGB colours from `palette`'s
/// `Wcag21RelativeContrast`; the crate's 0.7.7 name for the ratio is
/// `relative_contrast`, computed from `relative_luminance`.
fn contrast_ratio(one: Rgb, other: Rgb) -> f64 {
    fn srgb([red, green, blue]: Rgb) -> Srgb<f64> {
        Srgb::new(red, green, blue).into_format()
    }
    srgb(one).relative_contrast(srgb(other))
}

/// Keep the hue and saturation, flip the lightness. Sufficient for the
/// built-in Default light and for a theme file whose pair is missing.
fn invert_lightness([red, green, blue]: Rgb) -> Rgb {
    let rgb = Srgb::new(red, green, blue).into_format::<f64>();
    let hsl = Hsl::from_color(rgb);
    let inverted = Hsl::new(hsl.hue, hsl.saturation, 1.0 - hsl.lightness);
    let srgb = Srgb::<f64>::from_color(inverted).into_format::<u8>();
    [srgb.red, srgb.green, srgb.blue]
}

#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("line {line}: expected `theme[key] = \"value\"`")]
    Syntax { line: usize },
    #[error("line {line}: invalid colour value `{value}`")]
    InvalidValue { line: usize, value: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_three_value_forms() {
        let palette = Palette::parse(
            "# comment\ntheme[main_bg]=\"#1d2021\"\ntheme[main_fg]=\"#dd\"\ntheme[title]='250 189 47'\n",
        )
        .unwrap();
        assert_eq!(palette.color("main_bg"), [0x1d, 0x20, 0x21]);
        assert_eq!(palette.color("main_fg"), [0xdd, 0xdd, 0xdd]);
        assert_eq!(palette.color("title"), [250, 189, 47]);
    }

    #[test]
    fn default_theme_has_every_key() {
        let palette = Palette::default_dark();
        assert_eq!(palette.colors.len(), THEME_KEYS.len());
    }

    #[test]
    fn inverted_default_is_light_on_dark_text() {
        let light = Palette::default_dark().inverted();
        let surface = light.color("main_bg");
        let text = light.color("main_fg");
        assert!(surface.iter().all(|channel| *channel > 200));
        assert!(text.iter().all(|channel| *channel < 80));
    }

    #[test]
    fn pairs_themed_files_and_derives_the_missing_mode() {
        let (id, mode) = split_mode("everforest-dark-medium").unwrap();
        assert_eq!(id, "everforest-medium");
        assert_eq!(mode, Mode::Dark);
        let light = Palette::default_dark().inverted();
        let dark = light.inverted();
        assert!(dark.color("main_bg").iter().all(|channel| *channel < 40));
    }

    /// ACC-THEME-65-STEEL-DEFAULT: Default dark sits on Steel Dark, Default
    /// light on the JupyterLab light page.
    #[test]
    fn default_uses_steel_surfaces() {
        let pair = ThemePair::default_theme();
        let dark: BTreeMap<_, _> = pair.dark.semantic().into_iter().collect();
        assert_eq!(dark["surface"], [0x27, 0x2b, 0x31]);
        assert_eq!(dark["text"], [0xc3, 0xc3, 0xc3]);
        assert_eq!(pair.dark.color("div_line"), [0x40, 0x4b, 0x54]);
        assert_eq!(dark["border"], [0x6c, 0x75, 0x7c]);
        assert_eq!(pair.light.color("main_bg"), [0xff, 0xff, 0xff]);
    }

    /// ACC-GRAPH-72-NET-COLOURS: the Default download and upload lines differ
    /// by CIEDE2000 30 or more in both modes; btop's two violets differ by 11.
    #[test]
    fn default_download_and_upload_lines_differ() {
        use palette::color_difference::Ciede2000;
        use palette::{IntoColor, Lab};
        let lab = |[red, green, blue]: Rgb| -> Lab {
            Srgb::new(red, green, blue)
                .into_format::<f32>()
                .into_color()
        };
        let pair = ThemePair::default_theme();
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            let tokens: BTreeMap<_, _> = palette.semantic().into_iter().collect();
            let (down, up) = (tokens["chart-net-down"], tokens["chart-net-up"]);
            let difference = lab(down).difference(lab(up));
            assert!(
                difference >= 30.0,
                "{mode}: {down:?} and {up:?} differ by {difference:.1}"
            );
        }
    }

    /// ACC-THEME-65-STEEL-DEFAULT "modernised a little": no Default key keeps
    /// btop's neon saturation; the cap is 70% plus u8 rounding.
    #[test]
    fn default_saturation_is_capped() {
        let pair = ThemePair::default_theme();
        for (mode, palette) in [("dark", &pair.dark), ("light", &pair.light)] {
            for (key, [red, green, blue]) in &palette.colors {
                let hsl = Hsl::from_color(Srgb::new(*red, *green, *blue).into_format::<f64>());
                assert!(
                    hsl.saturation <= 0.71,
                    "{mode} {key} saturation {:.3}",
                    hsl.saturation
                );
            }
        }
    }

    /// The Default light palette is the Steel dark with every lightness
    /// inverted, under the `STEEL_LIGHT` overrides from the JupyterLab light
    /// and Steel Light themes (ACC-THEME-65-STEEL-DEFAULT). Every token is pinned so a conversion swap cannot move one channel
    /// (DEF-LIB-19-HSL-CONVERT); the semantic tokens are the last thirty.
    /// The status tokens use the 4.5:1 text floor (DEF-THEME-20-LIGHT-GREEN),
    /// and chip-fg, zebra, row-hover and the net text variants carry the
    /// tinted-layer fix (DEF-THEME-21-TINT-CONTRAST).
    #[test]
    fn default_light_block_is_pinned() {
        const LIGHT: &str = concat!(
            "[data-theme=\"default\"][data-mode=\"light\"]{",
            "--available-end:#c89623;",
            "--available-mid:#715514;",
            "--available-start:#f1e2b1;",
            "--cached-end:#2190b8;",
            "--cached-mid:#156979;",
            "--cached-start:#afcce9;",
            "--cpu-box:#92aa96;",
            "--cpu-end:#b32323;",
            "--cpu-mid:#938834;",
            "--cpu-start:#358859;",
            "--div-line:#dde0e3;",
            "--download-end:#294f79;",
            "--download-mid:#685cbc;",
            "--download-start:#948ae0;",
            "--followed-bg:#4a4abf;",
            "--followed-fg:#111111;",
            "--free-end:#4f6812;",
            "--free-mid:#497a19;",
            "--free-start:#c7deb0;",
            "--graph-text:#9f9f9f;",
            "--hi-fg:#bf4a4a;",
            "--inactive-fg:#bfbfbf;",
            "--main-bg:#ffffff;",
            "--main-fg:#23262a;",
            "--mem-box:#b4b493;",
            "--meter-bg:#bfbfbf;",
            "--net-box:#7672a7;",
            "--proc-banner-bg:#bf84bf;",
            "--proc-banner-fg:#111111;",
            "--proc-box:#ad7f7f;",
            "--proc-follow-bg:#4a4abf;",
            "--proc-misc:#30da69;",
            "--proc-pause-bg:#bf4a4a;",
            "--process-end:#ab2b2b;",
            "--process-mid:#867b23;",
            "--process-start:#2f7f52;",
            "--selected-bg:#d09595;",
            "--selected-fg:#111111;",
            "--temp-end:#a21d6f;",
            "--temp-mid:#1d3aa5;",
            "--temp-start:#2b7ab7;",
            "--title:#000000;",
            "--upload-end:#875726;",
            "--upload-mid:#bb7fbe;",
            "--upload-start:#eda4ef;",
            "--used-end:#9c1c33;",
            "--used-mid:#9d2631;",
            "--used-start:#d9aba6;",
            "--surface:#ffffff;",
            "--text:#23262a;",
            "--text-strong:#000000;",
            "--muted:#747679;",
            "--accent:#bf4a4a;",
            "--selection-bg:#d09595;",
            "--selection-fg:#111111;",
            "--border:#929496;",
            "--border-cpu:#849a88;",
            "--border-mem:#96967b;",
            "--border-net:#7672a7;",
            "--border-proc:#ad7f7f;",
            "--ok:#348457;",
            "--warn:#80762d;",
            "--crit:#b32323;",
            "--chip-fg:#000000;",
            "--zebra:#ffffff;",
            "--row-hover:#ffffff;",
            "--net-down-text:#294f79;",
            "--net-up-text:#875726;",
            "--chart-gpu-0:#938834;",
            "--chart-gpu-1:#685cbc;",
            "--chart-gpu-2:#a21d6f;",
            "--chart-cpu:#358859;",
            "--chart-mem-used:#9c1c33;",
            "--chart-mem-free:#4f6812;",
            "--chart-net-down:#294f79;",
            "--chart-net-up:#875726;",
            "--chart-proc:#867b23;",
            "--grid:#e5e5e5;",
            "}",
        );

        let css = render_css(&[ThemePair::default_theme()]);
        let start = css
            .find("[data-theme=\"default\"][data-mode=\"light\"]")
            .expect("default light block rendered");
        let end = start + css[start..].find('}').expect("closed theme block") + 1;
        assert_eq!(&css[start..end], LIGHT);
    }

    #[test]
    fn renders_default_dark_and_light_blocks() {
        let css = render_css(&[ThemePair::default_theme()]);
        assert!(css.contains(":root, [data-theme=\"default\"][data-mode=\"dark\"]"));
        assert!(css.contains("[data-theme=\"default\"][data-mode=\"light\"]"));
        assert!(css.contains("--surface:"));
        assert!(css.contains("--accent:"));
        assert!(css.contains("--border-cpu:"));
        // ACC-THEME-66-BOX-BORDERS: the Steel panel borders follow on the
        // Default selectors, and `:root` carries no per-panel token.
        assert!(css.contains("[data-theme=\"default\"][data-mode=\"dark\"]{--border-cpu:#476b69;"));
        assert!(
            css.contains("[data-theme=\"default\"][data-mode=\"light\"]{--border-cpu:#b0c8ca;")
        );
        assert!(
            !css.lines()
                .next()
                .unwrap_or_default()
                .contains("--border-cores")
        );
    }
}

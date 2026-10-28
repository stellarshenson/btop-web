//! Server-side chart geometry.
//!
//! Every sparkline and history chart is drawn through knots that keep their
//! place in time (DEF-GRAPH-32-UNSTABLE-HISTORY). The samples fall into
//! buckets of at least one pixel column, anchored to the absolute sample
//! count rather than to the right edge, and each bucket's largest sample is a
//! knot; the newest sample closes the line at the right edge. A knot is a
//! real sample, so it keeps its value and moves left one sample step per tick,
//! and a complete bucket keeps its knot. Until the window is full the oldest
//! sample starts the line; once it is full the line starts at the first
//! complete bucket's knot, because the oldest sample is replaced every tick
//! (DEF-GRAPH-39-NET-JUMP, steady left edge, owner 2026-10-27). Where every
//! sample is a knot two or more columns apart, the PCHIP curve from the
//! `spline1d` crate adds vertices at fixed fractions of each interval; its
//! limiter keeps the curve inside the range of the samples around it, so no
//! drawn point leaves the series extremes.
//!
//! A history records only values the collector read. An unavailable sample is
//! skipped rather than drawn as zero, and a history with fewer than two samples
//! draws nothing.
//!
//! ACC-GRAPH-38-TEMPORAL: a history stores an exponentially smoothed series
//! with a 1 s time constant, so the PCHIP knots are the smoothed values. The
//! readouts never read a history; they show btop's raw values.
//!
//! [`History::with_window`] spans a shorter window, [`PROC_WINDOW_MS`], for
//! the process row graphs (ACC-VIEW-52-PROC-GRAPH).
//!
//! ACC-GRAPH-67-SMOOTH-SCALE: an auto-scaled chart moves to a new ceiling
//! over several frames through [`Scale`]; the server draws at the larger of
//! the two ceilings and the page scales the series group about the plot floor.
//!
//! The SVG path data is written by hand at two fixed decimals, which bounds the SSE payload.

use std::collections::VecDeque;
use std::f64::consts::PI;
use std::fmt::Write as _;

use spline1d::Spline;

/// The smoothing time constant: a step reaches 63.2% after one second at any
/// interval.
pub const SMOOTHING_TAU_MS: f64 = 1000.0;

/// The time every graph spans, matching the `240 s` axis of the 05 design.
pub const WINDOW_MS: u64 = 240_000;

/// The time a process row graph spans: btop's 10 samples at its 2000 ms
/// default (ACC-VIEW-52-PROC-GRAPH).
pub const PROC_WINDOW_MS: u64 = 20_000;

/// The history depth for one sampling interval: one [`WINDOW_MS`] window,
/// never fewer than btop's 120 samples and never more than 2400.
/// 250 ms gives 960 samples, 2000 ms gives 120.
pub fn capacity(update_ms: u64) -> usize {
    (WINDOW_MS / update_ms.max(1)).clamp(120, 2400) as usize
}

/// One step of the 1 s exponential moving average: the first value seeds it,
/// then each value moves it by `alpha = 1 - exp(-update_ms / 1 s)`.
// Own code: two lines; no crate in the tree smooths by a time constant over a variable interval.
pub fn smooth(previous: Option<f64>, value: f64, update_ms: u64) -> f64 {
    match previous {
        None => value,
        Some(previous) => {
            let alpha = 1.0 - (-(update_ms as f64) / SMOOTHING_TAU_MS).exp();
            previous + alpha * (value - previous)
        }
    }
}

/// A panel's SVG viewBox, in the design's own units. Templates stretch the
/// viewBox to the panel with `preserveAspectRatio="none"`.
#[derive(Debug, Clone, Copy)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// The CPU stat panel's plot strip at 1440x900 (05 `#cpu-stat .stat-chart`).
pub const CPU_HISTORY: Size = Size {
    width: 467.0,
    height: 28.0,
};

/// The Processes tile's count plot (ACC-VIEW-53-PROC-TILE): the cpu-stat
/// strip's height, 30 units across, so at 250 ms each knot is a bucket of 32
/// samples (8 s). Writing a knot costs about 1.7k instructions every tick, and
/// 120 knots added about 2 percent to the server's instructions per tick.
pub const PROC_COUNT: Size = Size {
    width: 30.0,
    height: 28.0,
};

/// The memory stat panel's plot strip at 1440x900 (05 `#mem-stat .stat-chart`).
pub const MEM_HISTORY: Size = Size {
    width: 467.0,
    height: 28.0,
};

/// One network plot's box at 1440x900 (05 `.net-plot`, down and up).
pub const NET_PLOT: Size = Size {
    width: 391.0,
    height: 77.0,
};

/// The GPU load plot's box at 1440x900 (05.06-adaptive-base `#gpu-load`), one
/// series per device on the network plot's time axis.
pub const GPU_PLOT: Size = Size {
    width: 334.0,
    height: 135.0,
};

/// A process row graph's viewBox. Ten units across make each time-anchored
/// bucket a tenth of the window, so the path keeps about ten knots at any
/// interval (80 samples at 250 ms in buckets of 8; btop's 10 at 2000 ms).
pub const PROC_GRAPH: Size = Size {
    width: 10.0,
    height: 12.0,
};

/// The two SVG paths one chart needs: the stroked curve and the filled area
/// below it. Both are empty when the history has fewer than two samples.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChartPath {
    pub line: String,
    pub area: String,
}

impl ChartPath {
    /// Build both paths for `samples`, right-aligned in `slots` sample
    /// positions, at the given viewBox size and range. `first` is the
    /// absolute sample count of `samples[0]`, which anchors the buckets.
    pub fn new(
        samples: &[f64],
        first: u64,
        slots: usize,
        width: f64,
        height: f64,
        min: f64,
        max: f64,
    ) -> Self {
        let (line, start) = line(samples, first, slots, width, height, min, max);
        let area = if line.is_empty() {
            String::new()
        } else {
            // The area closes under the line's first vertex.
            let mut area = line.clone();
            let _ = write!(area, "L{width:.2} {height:.2}L{start:.2} {height:.2}Z");
            area
        };
        Self { line, area }
    }
}

/// Turn a sample series into an SVG line path.
///
/// The newest sample sits at the right edge: sample `k` of `n` sits at
/// x = `(slots - n + k) * width / (slots - 1)`, so a partial history starts
/// mid-plot the way btop's graphs fill. The vertices are the [`knots`] for
/// buckets of `ceil(1 / slot width)` samples counted from absolute sample 0;
/// `first` is the absolute count of `samples[0]`. When a slot is two or more
/// units wide, every sample is a knot and the PCHIP curve adds
/// `floor(slot width) - 1` vertices at fixed fractions of each interval.
/// Once `samples` fill the window, the line starts at the first complete
/// bucket's knot, and with PCHIP one knot later, because the curve's end slope
/// at the oldest sample differs from its interior slope and the oldest
/// interval would change shape when its left neighbour leaves.
/// Empty and single-sample series, and a range without width, yield `""`.
pub fn path(
    samples: &[f64],
    first: u64,
    slots: usize,
    width: f64,
    height: f64,
    min: f64,
    max: f64,
) -> String {
    line(samples, first, slots, width, height, min, max).0
}

/// [`path`] and the x of its first vertex, where the area closes.
fn line(
    samples: &[f64],
    first: u64,
    slots: usize,
    width: f64,
    height: f64,
    min: f64,
    max: f64,
) -> (String, f64) {
    if samples.len() < 2
        || !width.is_finite()
        || width < 1.0
        || !height.is_finite()
        || height <= 2.0
        || !min.is_finite()
        || !max.is_finite()
        || max <= min
    {
        return (String::new(), 0.0);
    }
    if samples.iter().any(|value| !value.is_finite()) {
        return (String::new(), 0.0);
    }
    let full = samples.len() >= slots;
    let slots = slots.max(samples.len());
    let offset = (slots - samples.len()) as f64;
    let slot_width = width / (slots - 1) as f64;
    let inner_height = height - 2.0;
    let span = max - min;
    let x = |at: f64| (offset + at) * slot_width;
    let y = |value: f64| height - 1.0 - ((value - min) / span).clamp(0.0, 1.0) * inner_height;
    let bucket = (1.0 / slot_width).ceil().max(1.0) as usize;
    let steps = slot_width.floor().max(1.0) as usize;
    let mut knots = knots(samples, first, bucket, full);
    if full && steps > 1 && knots.len() > 2 {
        knots.remove(0);
    }
    let spline = (steps > 1).then(|| {
        let at: Vec<f64> = (0..samples.len()).map(|index| index as f64).collect();
        Spline::pchip(&at, samples)
    });
    let mut path = String::with_capacity(knots.len() * steps * 14);
    let start = x(knots[0] as f64);
    push_vertex(&mut path, 'M', start, y(samples[knots[0]]));
    for pair in knots.windows(2) {
        let (from, to) = (pair[0] as f64, pair[1] as f64);
        if let Some(spline) = &spline {
            for step in 1..steps {
                let at = from + (to - from) * step as f64 / steps as f64;
                let Some(value) = spline.interpolate(&at) else {
                    return (String::new(), 0.0);
                };
                push_vertex(&mut path, 'L', x(at), y(value));
            }
        }
        push_vertex(&mut path, 'L', x(to), y(samples[pair[1]]));
    }
    (path, start)
}

/// Append `command` and the vertex `x y`, each as `{:.2}` writes it.
fn push_vertex(path: &mut String, command: char, x: f64, y: f64) {
    path.push(command);
    push_fixed2(path, x);
    path.push(' ');
    push_fixed2(path, y);
}

/// Append `value` as `{value:.2}` writes it, byte for byte.
// Own code: `{:.2}` costs about 110 ns per vertex, 39 percent of the server's
// instructions per tick at 250 ms; whole hundredths through `itoa` cost 16 ns.
// A negative or huge value, or one within 1e-6 of a rounding tie, where the
// rounded product `value * 100` cannot decide the exact decimal, takes `{:.2}`.
fn push_fixed2(out: &mut String, value: f64) {
    let hundredths = value * 100.0;
    if !(value.is_sign_positive() && hundredths < 1e9) || (hundredths.fract() - 0.5).abs() < 1e-6 {
        let _ = write!(out, "{value:.2}");
        return;
    }
    let hundredths = hundredths.round() as u64;
    let fraction = (hundredths % 100) as u8;
    out.push_str(itoa::Buffer::new().format(hundredths / 100));
    out.push('.');
    out.push(char::from(b'0' + fraction / 10));
    out.push(char::from(b'0' + fraction % 10));
}

/// The indices of the samples a path draws as vertices: the oldest, the
/// largest sample of every bucket of `bucket` samples (buckets start at
/// absolute sample counts divisible by `bucket`; `first` is the count of
/// `samples[0]`), and the newest. A complete bucket's knot depends on its own
/// samples only, so it never changes while it scrolls; the partial bucket at
/// the right edge shows its largest sample so far, so a peak is drawn from the
/// tick it arrives until it leaves the window. With `full`, the window drops a
/// sample every tick, so the oldest sample and the partial bucket at the left
/// edge are left out and the first complete bucket starts the line.
fn knots(samples: &[f64], first: u64, bucket: usize, full: bool) -> Vec<usize> {
    let start = (bucket - (first % bucket as u64) as usize) % bucket;
    let (head, rest) = samples.split_at(start.min(samples.len()));
    let mut knots = if full { Vec::new() } else { vec![0] };
    let mut offset = if full { head.len() } else { 0 };
    for chunk in (!full)
        .then_some(head)
        .into_iter()
        .chain(rest.chunks(bucket))
    {
        let peak = chunk
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(at, _)| offset + at);
        if let Some(peak) = peak.filter(|peak| knots.last() != Some(peak)) {
            knots.push(peak);
        }
        offset += chunk.len();
    }
    if knots.last() != Some(&(samples.len() - 1)) {
        knots.push(samples.len() - 1);
    }
    knots
}

/// A ring buffer of the newest [`capacity`] smoothed samples.
#[derive(Debug, Clone, Default)]
pub struct History {
    samples: VecDeque<f64>,
    /// The exponential moving average; `None` until the first sample.
    ema: Option<f64>,
    /// The slot count of the last push, from its interval.
    capacity: usize,
    /// Every sample ever pushed, which anchors the chart's buckets in time.
    pushed: u64,
    /// The window this history spans; `None` is [`WINDOW_MS`] via [`capacity`].
    window_ms: Option<u64>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// A history over `window_ms` instead of [`WINDOW_MS`]: `window_ms /
    /// update_ms` slots, at least two.
    pub fn with_window(window_ms: u64) -> Self {
        Self {
            window_ms: Some(window_ms),
            ..Self::default()
        }
    }

    /// Record one tick taken `update_ms` after the previous one. `None` is a
    /// value the collector could not read and is skipped, never drawn as zero.
    /// The first sample seeds the average; after that each sample moves it by
    /// `alpha = 1 - exp(-update_ms / 1 s)`.
    pub fn push(&mut self, value: Option<f64>, update_ms: u64) {
        let Some(value) = value.filter(|value| value.is_finite()) else {
            return;
        };
        let smoothed = smooth(self.ema, value, update_ms);
        self.ema = Some(smoothed);
        self.capacity = self.window_ms.map_or_else(
            || capacity(update_ms),
            |window| (window / update_ms.max(1)).max(2) as usize,
        );
        while self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(smoothed);
        self.pushed += 1;
    }

    /// The newest smoothed sample; `None` before the first. The gauge arcs
    /// read it (ACC-GRAPH-47-GAUGE-SMOOTH).
    pub fn ema(&self) -> Option<f64> {
        self.ema
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// The largest stored smoothed sample; `None` before the first.
    pub fn max(&self) -> Option<f64> {
        self.samples.iter().copied().reduce(f64::max)
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The line of [`History::chart`] without its area, for a process row
    /// graph, which draws no area.
    pub fn line(&self, size: Size, min: f64, max: f64) -> String {
        let samples: Vec<f64> = self.samples.iter().copied().collect();
        let first = self.pushed - samples.len() as u64;
        path(
            &samples,
            first,
            self.capacity,
            size.width,
            size.height,
            min,
            max,
        )
    }

    /// The chart for this history at a fixed viewBox size and value range,
    /// right-aligned in the full window.
    pub fn chart(&self, size: Size, min: f64, max: f64) -> ChartPath {
        let samples: Vec<f64> = self.samples.iter().copied().collect();
        let first = self.pushed - samples.len() as u64;
        ChartPath::new(
            &samples,
            first,
            self.capacity,
            size.width,
            size.height,
            min,
            max,
        )
    }
}

/// The shortest scale change: the 1 s EMA's time constant, 4 frames at 250 ms.
pub const RESCALE_MS: u64 = 1000;

/// A ratio above e^2.5 takes longer, 400 ms per unit of `ln(ratio)`, so a
/// vertex of a 77.5 px plot moves at most about 3.2 px per 60 Hz frame.
pub const RESCALE_MS_PER_LN: f64 = 400.0;

/// The ceiling an auto-scaled chart shows (ACC-GRAPH-67-SMOOTH-SCALE). When
/// the collector's ceiling changes, the shown ceiling `S` moves from `from` to
/// `to` geometrically, `S = from * (to / from)^p` with `p = elapsed /
/// duration`, which keeps every vertex's move per frame the same over the
/// whole change. A change lasts `max(RESCALE_MS, RESCALE_MS_PER_LN *
/// |ln(to / from)|)`; a new ceiling during a change starts from the shown one.
// Own code: no crate animates a ceiling across server-rendered frames.
#[derive(Debug, Clone, Default)]
pub struct Scale {
    from: f64,
    to: f64,
    elapsed_ms: u64,
    duration_ms: u64,
    id: u64,
}

/// One frame of a [`Scale`]: the ceiling to draw the paths at, the label, and
/// the series group's attributes while a change runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScaleFrame {
    /// The ceiling the paths are drawn at: the larger end of a running change,
    /// so no vertex is clamped at either end of it.
    pub draw_max: f64,
    /// The shown ceiling this frame, for the scale label.
    pub label_max: Option<u64>,
    /// The series group's inline style; empty when no change runs. The page
    /// animates `scaleY(k0 * q^p)` about the floor with `p` from 0 to 1.
    pub style: String,
    /// `data-rescale`: the change id and the server's elapsed ms, from which
    /// the page clocks the change across swaps.
    pub rescale: String,
}

impl Scale {
    fn shown(&self) -> f64 {
        if self.elapsed_ms >= self.duration_ms {
            return self.to;
        }
        let p = self.elapsed_ms as f64 / self.duration_ms as f64;
        self.from * (self.to / self.from).powf(p)
    }

    /// Advance one tick of `update_ms` towards the collector's ceiling
    /// `target`. The first call and a ceiling of zero or less start idle.
    pub fn update(&mut self, target: f64, update_ms: u64, size: Size) -> ScaleFrame {
        if self.to <= 0.0 || target <= 0.0 {
            *self = Self {
                from: target,
                to: target,
                id: self.id,
                ..Self::default()
            };
        } else {
            self.elapsed_ms = self.elapsed_ms.saturating_add(update_ms);
            if target != self.to {
                let from = self.shown();
                let ln = (target / from).ln().abs();
                *self = Self {
                    from,
                    to: target,
                    elapsed_ms: 0,
                    duration_ms: RESCALE_MS.max((RESCALE_MS_PER_LN * ln).ceil() as u64),
                    id: self.id + 1,
                };
            }
        }
        let label_max = Some(self.shown() as u64);
        // One frame past the end, so the page holds the end state when the
        // plain path returns.
        if self.elapsed_ms >= self.duration_ms + update_ms || self.duration_ms == 0 {
            return ScaleFrame {
                draw_max: self.to,
                label_max,
                ..ScaleFrame::default()
            };
        }
        let draw_max = self.from.max(self.to);
        ScaleFrame {
            draw_max,
            label_max,
            style: format!(
                "--k0:{:.6};--q:{:.6};transform-origin:0 {:.2}px;animation:rescale {}ms linear both",
                draw_max / self.from,
                self.from / self.to,
                size.height - 1.0,
                self.duration_ms
            ),
            rescale: format!("{} {}", self.id, self.elapsed_ms),
        }
    }
}

/// The radial gauge's viewBox (`0 0 108 114`), centre, arc radius and stroke.
/// The arc band runs from radius 36 to 44; ticks start at 46, outside it, and
/// the readout stays within [`GAUGE_READOUT_RADIUS`] of the centre.
pub const GAUGE_SIZE: Size = Size {
    width: 108.0,
    height: 114.0,
};
pub const GAUGE_CX: f64 = 54.0;
pub const GAUGE_CY: f64 = 66.0;
pub const GAUGE_RADIUS: f64 = 40.0;
pub const GAUGE_STROKE: f64 = 8.0;
/// The readout's bound: the arc's inner radius 36 minus a 2-unit margin.
pub const GAUGE_READOUT_RADIUS: f64 = 34.0;
const TICK_INNER: f64 = 46.0;
const TICK_MINOR_OUTER: f64 = 50.0;
const TICK_MAJOR_OUTER: f64 = 53.0;
const TICK_LABEL_RADIUS: f64 = 62.0;

/// A point at percentage `pct` on radius `rho`: 0 at the lower left, 50 at the
/// top, 100 at the lower right, over 270 degrees.
fn gauge_point(pct: f64, rho: f64) -> (f64, f64) {
    let theta = (-135.0 + 2.7 * pct.clamp(0.0, 100.0)) * PI / 180.0;
    (GAUGE_CX + rho * theta.sin(), GAUGE_CY - rho * theta.cos())
}

/// The gauge arc from 0 to `pct`. `None`, a non-finite value and zero or less
/// draw nothing; the full track is `gauge_arc(Some(100.0))`.
pub fn gauge_arc(pct: Option<f64>) -> String {
    let Some(pct) = pct.filter(|pct| pct.is_finite() && *pct > 0.0) else {
        return String::new();
    };
    let pct = pct.min(100.0);
    let (x0, y0) = gauge_point(0.0, GAUGE_RADIUS);
    let (x1, y1) = gauge_point(pct, GAUGE_RADIUS);
    let large = u8::from(2.7 * pct > 180.0);
    format!("M{x0:.2} {y0:.2}A{GAUGE_RADIUS} {GAUGE_RADIUS} 0 {large} 1 {x1:.2} {y1:.2}")
}

/// Tick marks every 10 percent: the eight minor ticks, or with `major` the
/// three longer ones at 0, 50 and 100.
pub fn gauge_ticks(major: bool) -> String {
    let mut path = String::new();
    for step in 0..=10 {
        let is_major = step == 0 || step == 5 || step == 10;
        if is_major != major {
            continue;
        }
        let pct = f64::from(step) * 10.0;
        let outer = if major {
            TICK_MAJOR_OUTER
        } else {
            TICK_MINOR_OUTER
        };
        let (x0, y0) = gauge_point(pct, TICK_INNER);
        let (x1, y1) = gauge_point(pct, outer);
        let _ = write!(path, "M{x0:.2} {y0:.2}L{x1:.2} {y1:.2}");
    }
    path
}

/// The scale labels `0`, `50` and `100`, centred outside the major ticks.
pub fn gauge_labels() -> [(f64, f64, &'static str); 3] {
    [(0.0, "0"), (50.0, "50"), (100.0, "100")].map(|(pct, text)| {
        let (x, y) = gauge_point(pct, TICK_LABEL_RADIUS);
        (x, y, text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(path: &str) -> Vec<(f64, f64)> {
        path.split(['M', 'L'])
            .filter(|part| !part.is_empty())
            .map(|point| {
                let (x, y) = point.split_once(' ').expect("x and y");
                (x.parse().expect("x"), y.parse().expect("y"))
            })
            .collect()
    }

    /// Invert the pixel mapping to recover the value a point draws.
    fn value(y: f64, height: f64, min: f64, max: f64) -> f64 {
        min + (height - 1.0 - y) / (height - 2.0) * (max - min)
    }

    #[test]
    fn knots_land_on_the_sampled_values() {
        let samples = [10.0, 20.0, 5.0, 30.0];
        // One pixel column per interval makes every knot an evaluated column.
        let drawn = points(&path(&samples, 0, samples.len(), 3.0, 100.0, 0.0, 40.0));
        assert_eq!(drawn.len(), samples.len());
        for (index, sample) in samples.iter().enumerate() {
            let (x, y) = drawn[index];
            assert!((x - index as f64).abs() < 0.01, "x for knot {index}: {x}");
            let drawn_value = value(y, 100.0, 0.0, 40.0);
            assert!(
                (drawn_value - sample).abs() < 0.02,
                "knot {index}: {drawn_value} != {sample}"
            );
        }
    }

    #[test]
    fn a_monotone_series_stays_monotone() {
        let samples = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let drawn = points(&path(&samples, 0, samples.len(), 200.0, 100.0, 0.0, 5.0));
        for pair in drawn.windows(2) {
            assert!(pair[1].1 <= pair[0].1 + 1e-9, "y rose: {pair:?}");
        }
    }

    #[test]
    fn no_point_exceeds_the_sample_min_and_max() {
        let samples = [3.0, 100.0, 0.0, 47.0, 100.0, 2.0];
        // A wider drawing range proves the interpolation itself stays in
        // bounds instead of being hidden by the pixel clamp.
        let drawn = points(&path(
            &samples,
            0,
            samples.len(),
            300.0,
            100.0,
            -10.0,
            110.0,
        ));
        for (_, y) in drawn {
            let drawn_value = value(y, 100.0, -10.0, 110.0);
            assert!(
                (0.0..=100.0).contains(&drawn_value),
                "overshoot: {drawn_value}"
            );
        }
    }

    #[test]
    fn empty_and_single_sample_series_draw_nothing() {
        assert_eq!(path(&[], 0, 2, 100.0, 50.0, 0.0, 100.0), "");
        assert_eq!(path(&[42.0], 0, 2, 100.0, 50.0, 0.0, 100.0), "");
        let mut history = History::new();
        assert!(history.chart(CPU_HISTORY, 0.0, 100.0).line.is_empty());
        history.push(Some(42.0), 250);
        assert!(history.chart(CPU_HISTORY, 0.0, 100.0).line.is_empty());
    }

    /// A process row graph takes the line alone; it is the chart's line.
    #[test]
    fn line_is_the_charts_line() {
        let mut history = History::with_window(PROC_WINDOW_MS);
        for tick in 0..100 {
            history.push(Some(f64::from(tick % 7) * 9.0), 250);
            assert_eq!(
                history.line(PROC_GRAPH, 0.0, 100.0),
                history.chart(PROC_GRAPH, 0.0, 100.0).line
            );
        }
    }

    #[test]
    fn a_full_history_builds_the_curve_and_its_area() {
        let slots = capacity(2000);
        let samples: Vec<f64> = (0..slots).map(|index| (index % 37) as f64 * 2.5).collect();
        let chart = ChartPath::new(
            &samples,
            0,
            slots,
            CPU_HISTORY.width,
            CPU_HISTORY.height,
            0.0,
            100.0,
        );
        // The window is full: the line starts one knot after the oldest
        // sample (DEF-GRAPH-39-NET-JUMP).
        assert!(chart.line.starts_with("M3.92 "), "{}", &chart.line[..12]);
        // A slot is 467 / 119 = 3.92 units wide: the knot and two PCHIP
        // vertices per interval.
        assert_eq!(chart.line.matches('L').count(), (slots - 2) * 3);
        assert!(chart.area.ends_with("L3.92 28.00Z"), "{}", chart.area);
        assert!(chart.area.ends_with('Z'));
        assert!(!chart.line.contains("NaN"));
    }

    #[test]
    fn history_keeps_the_newest_capacity_samples() {
        let mut history = History::new();
        for value in 0..capacity(250) + 5 {
            history.push(Some(value as f64), 250);
        }
        assert_eq!(history.len(), 960);
        history.push(None, 250);
        assert_eq!(history.len(), 960);
        // A longer interval shrinks the window to its own capacity.
        history.push(Some(1.0), 2000);
        assert_eq!(history.len(), 120);
    }

    #[test]
    fn with_window_keeps_window_over_interval_samples() {
        let mut history = History::with_window(PROC_WINDOW_MS);
        for value in 0..200 {
            history.push(Some(value as f64), 250);
        }
        assert_eq!(history.len(), 80);
        history.push(Some(1.0), 2000);
        assert_eq!(history.len(), 10, "btop's 10 samples at 2000 ms");
    }

    /// The lowest y (the highest drawn value) of a line path.
    fn top(line: &str) -> f64 {
        points(line)
            .into_iter()
            .map(|(_, y)| y)
            .fold(f64::INFINITY, f64::min)
    }

    /// DEF-GRAPH-32-UNSTABLE-HISTORY: one raw spike, then 40 quiet ticks. The
    /// spike's smoothed peak must draw at the same height in every frame while
    /// it moves left.
    #[test]
    fn a_spike_keeps_its_plotted_height_in_every_frame() {
        let mut history = History::new();
        for _ in 0..capacity(250) {
            history.push(Some(10.0), 250);
        }
        let mut heights = Vec::new();
        for frame in 0..40 {
            history.push(Some(if frame == 0 { 90.0 } else { 10.0 }), 250);
            heights.push(top(&history.chart(NET_PLOT, 0.0, 100.0).line));
        }
        let (low, high) = heights
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), y| {
                (low.min(*y), high.max(*y))
            });
        assert!(high - low < 0.01, "the spike's top moved {low}..{high}");
    }

    /// DEF-GRAPH-32-UNSTABLE-HISTORY: over 40 ticks of a noisy series, every
    /// vertex sits on a sample, and each completed knot keeps its value while
    /// its x moves left by one sample step per tick.
    #[test]
    fn completed_knots_keep_their_value_and_move_left_one_step_per_tick() {
        let value = |tick: u64| ((tick * 7919) % 101) as f64;
        for size in [CPU_HISTORY, NET_PLOT, GPU_PLOT] {
            let slots = capacity(250);
            let step = size.width / (slots - 1) as f64;
            let bucket = (1.0 / step).ceil() as u64;
            let mut history = History::new();
            let mut tick = 0;
            let mut previous: Option<std::collections::BTreeMap<u64, (f64, f64)>> = None;
            for frame_index in 0..slots + 57 {
                history.push(Some(value(tick)), 250);
                tick += 1;
                if frame_index < slots + 17 {
                    continue;
                }
                let first = tick - slots as u64;
                let mut frame = std::collections::BTreeMap::new();
                for (x, y) in points(&history.chart(size, 0.0, 100.0).line) {
                    let at = x / step;
                    assert!((at - at.round()).abs() < 0.03, "vertex off a sample: {x}");
                    frame.insert(first + at.round() as u64, (x, y));
                }
                assert!(
                    frame.len() <= slots / bucket as usize + 3,
                    "{}",
                    frame.len()
                );
                if let Some(previous) = &previous {
                    let newest = *previous.keys().next_back().expect("a vertex");
                    for (&time, &(x, y)) in previous {
                        // The partial buckets at both edges are still filling or
                        // leaving; every other vertex is a completed knot.
                        let bucket_start = time / bucket * bucket;
                        if time == newest
                            || bucket_start < first
                            || bucket_start + bucket > newest + 1
                        {
                            continue;
                        }
                        let (moved_x, moved_y) = frame[&time];
                        assert_eq!(moved_y, y, "knot at sample {time} changed value");
                        assert!((x - step - moved_x).abs() < 0.011, "{x} -> {moved_x}");
                    }
                }
                previous = Some(frame);
            }
        }
    }

    /// DEF-GRAPH-39-NET-JUMP, steady left edge: once the window is full, every
    /// vertex of a frame left of the newest two buckets is a vertex of the
    /// previous frame one slot to its right with the same y, the first vertex
    /// included.
    #[test]
    fn a_full_window_keeps_every_vertex_but_the_newest() {
        let value = |tick: u64| ((tick * 7919) % 101) as f64;
        for update_ms in [250, 2000] {
            for size in [CPU_HISTORY, NET_PLOT, GPU_PLOT] {
                let slots = capacity(update_ms);
                let step = size.width / (slots - 1) as f64;
                let bucket = (1.0 / step).ceil();
                let mut history = History::new();
                for tick in 0..slots as u64 {
                    history.push(Some(value(tick)), update_ms);
                }
                let mut previous = points(&history.chart(size, 0.0, 100.0).line);
                for tick in slots as u64..slots as u64 + 40 {
                    history.push(Some(value(tick)), update_ms);
                    let frame = points(&history.chart(size, 0.0, 100.0).line);
                    let settled = size.width - 2.0 * bucket * step - 0.01;
                    for &(x, y) in frame.iter().filter(|(x, _)| *x < settled) {
                        assert!(
                            previous
                                .iter()
                                .any(|&(px, py)| (px - step - x).abs() < 0.011 && py == y),
                            "{update_ms} ms, {}: vertex ({x}, {y}) was not drawn one slot right",
                            size.width
                        );
                    }
                    previous = frame;
                }
            }
        }
    }

    /// ACC-GRAPH-67-SMOOTH-SCALE: the first ceiling is shown at once.
    #[test]
    fn a_scale_starts_idle_at_its_first_ceiling() {
        let mut scale = Scale::default();
        let frame = scale.update(100.0, 250, NET_PLOT);
        assert_eq!(frame.draw_max, 100.0);
        assert_eq!(frame.label_max, Some(100));
        assert!(frame.style.is_empty() && frame.rescale.is_empty());
        assert_eq!(scale.update(100.0, 250, NET_PLOT), frame);
    }

    /// ACC-GRAPH-67-SMOOTH-SCALE: a 10x change takes 1000 ms; the label moves
    /// geometrically over the four 250 ms frames and the series group carries
    /// the animation one frame past the end.
    #[test]
    fn a_scale_change_moves_geometrically_over_the_duration() {
        let mut scale = Scale::default();
        scale.update(100.0, 250, NET_PLOT);
        let frames: Vec<ScaleFrame> = (0..6)
            .map(|_| scale.update(1000.0, 250, NET_PLOT))
            .collect();
        let labels: Vec<Option<u64>> = frames.iter().map(|frame| frame.label_max).collect();
        assert_eq!(
            labels,
            [100, 177, 316, 562, 1000, 1000].map(Some),
            "elapsed 0 to 1250 ms"
        );
        for frame in &frames[..5] {
            assert_eq!(frame.draw_max, 1000.0);
            assert_eq!(
                frame.style,
                "--k0:10.000000;--q:0.100000;transform-origin:0 76.00px;animation:rescale 1000ms linear both"
            );
        }
        assert_eq!(frames[0].rescale, "1 0");
        assert_eq!(frames[4].rescale, "1 1000");
        assert_eq!(
            frames[5],
            ScaleFrame {
                draw_max: 1000.0,
                label_max: Some(1000),
                ..ScaleFrame::default()
            }
        );
    }

    /// ACC-GRAPH-67-SMOOTH-SCALE: a 100x fall lasts 400 ms * ln(100).
    #[test]
    fn a_large_ratio_takes_longer() {
        let mut scale = Scale::default();
        scale.update(100_000.0, 250, NET_PLOT);
        let frame = scale.update(1000.0, 250, NET_PLOT);
        assert!(
            frame
                .style
                .ends_with("animation:rescale 1843ms linear both"),
            "{}",
            frame.style
        );
        assert_eq!(frame.draw_max, 100_000.0);
    }

    /// ACC-GRAPH-67-SMOOTH-SCALE: a new ceiling during a change starts from
    /// the ceiling shown at that frame, so the label does not jump.
    #[test]
    fn a_second_change_starts_from_the_shown_ceiling() {
        let mut scale = Scale::default();
        scale.update(100.0, 250, NET_PLOT);
        scale.update(1000.0, 250, NET_PLOT);
        assert_eq!(scale.update(1000.0, 250, NET_PLOT).label_max, Some(177));
        // At 500 ms the first change shows 316; the fall to 50 starts there.
        let second = scale.update(50.0, 250, NET_PLOT);
        assert_eq!(second.label_max, Some(316));
        assert_eq!(second.rescale, "2 0");
        assert!(
            (second.draw_max - 316.23).abs() < 0.01,
            "{}",
            second.draw_max
        );
        assert_eq!(scale.update(50.0, 250, NET_PLOT).label_max, Some(199));
    }

    #[test]
    fn ema_is_the_newest_smoothed_sample() {
        let mut history = History::new();
        assert_eq!(history.ema(), None);
        history.push(Some(0.0), 250);
        history.push(Some(100.0), 250);
        assert_eq!(history.ema(), history.samples.back().copied());
        assert!((history.ema().expect("seeded") - 22.12).abs() < 0.01);
        history.push(None, 250);
        assert_eq!(history.ema(), history.samples.back().copied());
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn vertices_print_as_two_decimal_format_does() {
        let edges = [
            0.0, -0.0, 0.004, 0.005, 0.125, 0.375, 1.005, 2.675, 121.775, -1.5, 1e12,
        ];
        let sweep = (0..200_000).map(|step| step as f64 * 0.004_871_3);
        for value in edges.into_iter().chain(sweep).chain([f64::NAN]) {
            let mut out = String::new();
            push_fixed2(&mut out, value);
            assert_eq!(out, format!("{value:.2}"), "{value:e}");
        }
    }

    #[test]
    fn history_max_is_the_largest_stored_sample() {
        let mut history = History::new();
        assert_eq!(history.max(), None);
        history.push(Some(10.0), 250);
        history.push(Some(110.0), 250);
        history.push(Some(20.0), 250);
        let largest = history.samples.iter().copied().fold(f64::MIN, f64::max);
        assert_eq!(history.max(), Some(largest));
        assert!(
            largest > 10.0 && largest < 110.0,
            "the samples are smoothed"
        );
    }

    #[test]
    fn capacity_keeps_a_240_s_window() {
        assert_eq!(capacity(250), 960);
        assert_eq!(capacity(2000), 120);
        assert_eq!(capacity(100), 2400);
        assert_eq!(capacity(5000), 120);
    }

    #[test]
    fn ema_step_reaches_63_percent_after_one_time_constant() {
        let step = |count: usize, update_ms: u64| {
            let mut history = History::new();
            history.push(Some(0.0), update_ms);
            for _ in 0..count {
                history.push(Some(100.0), update_ms);
            }
            history.ema.expect("seeded")
        };
        assert!((step(4, 250) - 63.21).abs() < 0.01, "{}", step(4, 250));
        assert!((step(1, 2000) - 86.47).abs() < 0.01, "{}", step(1, 2000));
        assert!((step(10, 100) - 63.21).abs() < 0.01, "{}", step(10, 100));
    }

    #[test]
    fn first_sample_seeds_and_none_keeps_the_ema() {
        let mut history = History::new();
        history.push(Some(42.0), 250);
        assert_eq!(history.ema, Some(42.0));
        assert_eq!(history.samples.back(), Some(&42.0));
        history.push(None, 250);
        assert_eq!(history.ema, Some(42.0));
        assert_eq!(history.len(), 1);
        history.push(Some(f64::NAN), 250);
        assert_eq!(history.ema, Some(42.0));
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn a_partial_history_is_right_aligned() {
        let width = 467.0;
        let chart = ChartPath::new(&[10.0, 20.0], 0, 960, width, 28.0, 0.0, 100.0);
        let drawn = points(&chart.line);
        let first = drawn.first().expect("first point").0;
        let last = drawn.last().expect("last point").0;
        assert!(
            (first - width * 958.0 / 959.0).abs() < 0.01,
            "first x {first}"
        );
        assert!((last - width).abs() < 0.01, "last x {last}");
        assert!(
            chart.area.ends_with(&format!("L{first:.2} 28.00Z")),
            "{}",
            chart.area
        );
    }

    fn close(actual: (f64, f64), expected: (f64, f64)) -> bool {
        (actual.0 - expected.0).abs() < 0.01 && (actual.1 - expected.1).abs() < 0.01
    }

    /// The arc's end point and the SVG large-arc flag.
    fn arc_end(path: &str) -> ((f64, f64), u8) {
        let (_, arc) = path.split_once('A').expect("arc command");
        let fields: Vec<&str> = arc.split(' ').collect();
        let large = fields[3].parse().expect("large-arc flag");
        let end = (
            fields[5].parse().expect("end x"),
            fields[6].parse().expect("end y"),
        );
        (end, large)
    }

    #[test]
    fn gauge_arc_spans_270_degrees() {
        let (end, large) = arc_end(&gauge_arc(Some(50.0)));
        assert!(close(end, (54.0, 26.0)), "{end:?}");
        assert_eq!(large, 0);
        let (end, large) = arc_end(&gauge_arc(Some(100.0)));
        assert!(close(end, (82.28, 94.28)), "{end:?}");
        assert_eq!(large, 1);
        assert!(gauge_arc(Some(100.0)).starts_with("M25.72 94.28"));
        for none in [Some(0.0), None, Some(f64::NAN), Some(-5.0)] {
            assert_eq!(gauge_arc(none), "", "{none:?}");
        }
        assert_eq!(gauge_arc(Some(150.0)), gauge_arc(Some(100.0)));
    }

    #[test]
    fn gauge_ticks_are_eleven_with_three_major_outside_the_arc() {
        let minor = gauge_ticks(false);
        let major = gauge_ticks(true);
        assert_eq!(minor.matches('M').count(), 8);
        assert_eq!(major.matches('M').count(), 3);
        let outer_edge = GAUGE_RADIUS + GAUGE_STROKE / 2.0;
        let inner_edge = GAUGE_RADIUS - GAUGE_STROKE / 2.0;
        let radius = |(x, y): (f64, f64)| ((x - GAUGE_CX).powi(2) + (y - GAUGE_CY).powi(2)).sqrt();
        let mut ends = Vec::new();
        for path in [&minor, &major] {
            let drawn = points(path);
            for pair in drawn.chunks(2) {
                assert!(radius(pair[0]) >= 46.0 - 0.01 && radius(pair[0]) > outer_edge);
                ends.push(pair[1]);
            }
        }
        assert!(GAUGE_READOUT_RADIUS < inner_edge);
        // A label box is 0.6 em per character by 1 em at font-size 8, centred
        // on its point; it clears every tick end and stays in the viewBox.
        for (x, y, text) in gauge_labels() {
            let half_width = 0.6 * 8.0 * text.len() as f64 / 2.0;
            let (left, right, top, bottom) = (x - half_width, x + half_width, y - 4.0, y + 4.0);
            assert!(
                left >= 0.0 && right <= GAUGE_SIZE.width,
                "{text}: {left}..{right}"
            );
            assert!(
                top >= 0.0 && bottom <= GAUGE_SIZE.height,
                "{text}: {top}..{bottom}"
            );
            for (tx, ty) in &ends {
                let dx = (left - tx).max(tx - right).max(0.0);
                let dy = (top - ty).max(ty - bottom).max(0.0);
                assert!(
                    dx.hypot(dy) >= 2.0,
                    "{text} touches the tick at ({tx}, {ty})"
                );
            }
        }
    }
}

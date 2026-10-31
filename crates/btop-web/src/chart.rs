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
//! ACC-GRAPH-97-SMOOTH-REFRESH: the constant is 1 s of wall-clock time
//! whatever the interval, so at 250 ms it spans four samples and at 1 s one.
//! A sparse sampler therefore yields a choppier graph on purpose; no code
//! path may stretch tau to fake smoothness.
//!
//! [`History::with_window`] spans a fixed window, [`PROC_WINDOW_MS`], for the
//! process row graphs (ACC-VIEW-52-PROC-GRAPH); every panel history spans the
//! configured [`WINDOW_MS`] default, set by [`History::set_window`]
//! (ACC-VIEW-110-TIME-AXIS).
//!
//! ACC-GRAPH-67-SMOOTH-SCALE: an auto-scaled chart moves to a new ceiling
//! over several frames through [`Scale`]; the server draws at the larger of
//! the two ceilings and the page scales the series group about the plot floor.
//!
//! ACC-GRAPH-98-ANIM-CLOCK: a panel chart scrolls on the page's clock between
//! readouts, by one slow pass's share of the window, and the next slow pass's
//! frame arrives that far on. [`ChartPath::new`] extends the newest, smoothed
//! knot past the longest such scroll (DEF-GRAPH-125-TAIL-GAP), and the plot box
//! clips what is past its edge. The tail holds the value the curve already
//! draws, never a raw sample or a guess.
//!
//! The SVG path data is written by hand at two fixed decimals, which bounds the SSE payload.

use std::collections::VecDeque;
use std::f64::consts::PI;
use std::fmt::Write as _;

use spline1d::Spline;

/// The smoothing time constant: a step reaches 63.2% after one second at any
/// interval.
pub const SMOOTHING_TAU_MS: f64 = 1000.0;

/// The default time every graph spans: 3 minutes (ACC-VIEW-110-TIME-AXIS).
pub const WINDOW_MS: u64 = 180_000;

/// The time a process row graph spans: btop's 10 samples at its 2000 ms
/// default (ACC-VIEW-52-PROC-GRAPH).
pub const PROC_WINDOW_MS: u64 = 20_000;

/// The history depth for a `window_ms` window at one sampling interval: one
/// slot per sample, at least two. 180 s gives 720 slots at 250 ms and 90 at
/// 2000 ms; a 10 minute window at 100 ms gives 6000.
pub fn capacity(window_ms: u64, update_ms: u64) -> usize {
    (window_ms / update_ms.max(1)).max(2) as usize
}

/// The time the charts cover at an interval: `capacity` slots of `update_ms`,
/// which is the window itself whenever the interval divides it
/// (ACC-VIEW-110-TIME-AXIS). The axis labels read this, never the nominal
/// window, so a coarse interval never draws a longer span than it claims.
pub fn window(window_ms: u64, update_ms: u64) -> u64 {
    capacity(window_ms, update_ms) as u64 * update_ms
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

/// The GPU load plot's box at 1440x900 (05.06-adaptive-base `#gpu-load`), on
/// the network plot's time axis; `gpu_lane` splits its height into one lane
/// per device (DEF-GRAPH-51-GPU-LANES).
pub const GPU_PLOT: Size = Size {
    width: 334.0,
    height: 135.0,
};

/// One GPU lane's viewBox when `devices` lanes share the GPU plot's height
/// with the CSS 4 px gap between them: 135, 65, 42, 30 and 13 units for 1, 2,
/// 3, 4 and 8 devices. It is the lane's share of the plot at 1440x900; the
/// plot box is inset 3 px inside the lane to keep the 0% and 100% strokes off
/// the lane boundary (app.css), so the svg stretches that inset box a little.
pub fn gpu_lane(devices: usize) -> Size {
    let lanes = devices.max(1) as f64;
    Size {
        width: GPU_PLOT.width,
        height: ((GPU_PLOT.height - 4.0 * (lanes - 1.0)) / lanes).floor(),
    }
}

/// A process row graph's viewBox. Ten units across make each time-anchored
/// bucket a tenth of the window, so the path keeps about ten knots at any
/// interval (80 samples at 250 ms in buckets of 8; btop's 10 at 2000 ms).
pub const PROC_GRAPH: Size = Size {
    width: 10.0,
    height: 12.0,
};

/// The tail's reach, as `width / TAIL_SLOTS` (DEF-GRAPH-125-TAIL-GAP). The page
/// scrolls one slow pass between readouts, `every / capacity` of the width with
/// `every` ticks in a pass. The longest pass the config allows, 5000 ms rounded
/// up to a tick, is under 10 s, a sixth of the shortest window, 60 s; an
/// interval over 5000 ms is one tick to a pass, `1 / capacity`.
const TAIL_SLOTS: usize = 6;

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
    /// The line and the area extend past the newest knot by at least the most
    /// a panel chart scrolls between readouts (ACC-GRAPH-98-ANIM-CLOCK,
    /// [`TAIL_SLOTS`]), and the tail holds the smoothed value the curve already
    /// draws. The plot box clips it until the page moves. A process row graph
    /// draws [`path`] and carries no tail.
    pub fn new(
        samples: &[f64],
        first: u64,
        slots: usize,
        width: f64,
        height: f64,
        min: f64,
        max: f64,
    ) -> Self {
        let (mut line, start, newest_y) = line(samples, first, slots, width, height, min, max);
        if line.is_empty() {
            return Self::default();
        }
        let tail = width / slots.clamp(2, TAIL_SLOTS) as f64;
        push_vertex(&mut line, 'L', width + tail, newest_y);
        // The area closes under the line's first vertex, past the tail.
        let mut area = line.clone();
        let _ = write!(
            area,
            "L{:.2} {height:.2}L{start:.2} {height:.2}Z",
            width + tail
        );
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

/// [`path`], the x of its first vertex, where the area closes, and the y of
/// its newest vertex, where [`ChartPath::new`] anchors its tail.
fn line(
    samples: &[f64],
    first: u64,
    slots: usize,
    width: f64,
    height: f64,
    min: f64,
    max: f64,
) -> (String, f64, f64) {
    if samples.len() < 2
        || !width.is_finite()
        || width < 1.0
        || !height.is_finite()
        || height <= 2.0
        || !min.is_finite()
        || !max.is_finite()
        || max <= min
    {
        return (String::new(), 0.0, 0.0);
    }
    if samples.iter().any(|value| !value.is_finite()) {
        return (String::new(), 0.0, 0.0);
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
                    return (String::new(), 0.0, 0.0);
                };
                push_vertex(&mut path, 'L', x(at), y(value));
            }
        }
        push_vertex(&mut path, 'L', x(to), y(samples[pair[1]]));
    }
    // The newest knot is the last vertex, at the right edge.
    (path, start, y(samples[samples.len() - 1]))
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

    /// Report that this history spans `window_ms` from the next push
    /// (ACC-VIEW-110-TIME-AXIS). A wider window keeps the samples held, so the
    /// chart starts at their oldest and fills from the left; a narrower one
    /// drops the oldest samples on the next push.
    pub fn set_window(&mut self, window_ms: u64) {
        self.window_ms = Some(window_ms);
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
        self.capacity = capacity(self.window_ms.unwrap_or(WINDOW_MS), update_ms);
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
    /// right-aligned in the full window. The path carries the tail a panel
    /// chart scrolls over between readouts (ACC-GRAPH-98-ANIM-CLOCK); the
    /// process rows draw [`History::line`] instead.
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

/// The squircle gauge's viewBox (`-12 0 132 122`), centre, radius and stroke.
/// The band's centreline is a superellipse with semi-axes [`GAUGE_RADIUS`];
/// the stroke runs 4 units either side of it, the ticks start 6 units out,
/// and the readout stays within [`GAUGE_READOUT_RADIUS`] of the centre. The
/// canvas is wider and taller than the old circle's 108x114 to hold the scale
/// labels outside the squircle's corners (ACC-VIEW-132-GAUGE-SQUIRCLE), then
/// 20 units wider again for the eleven label boxes of ACC-VIEW-173: the
/// outermost, the 20 and 80 labels at 5 px, end 10.89 units left of the box
/// and 118.89 units right of it.
pub const GAUGE_SIZE: Size = Size {
    width: 132.0,
    height: 122.0,
};

/// The circle gauge's viewBox (`-12 0 132 114`), the same box the script drew
/// before DEF-LAYOUT-90-LOAD-SHIFT, widened like [`GAUGE_SIZE`]: its 20 and
/// 80 labels end at -10.15 and 118.15. The server now renders both shapes and
/// app.css shows the one the gauge body's height holds.
pub const GAUGE_CIRCLE_SIZE: Size = Size {
    width: 132.0,
    height: 114.0,
};

/// The gauge viewBoxes' left edge. Both boxes run to 120, which keeps the
/// centre 54 at the middle and every label box inside (ACC-VIEW-171).
pub const GAUGE_VIEW_X: f64 = -12.0;
/// The wide gauge's viewBox (DEF-LAYOUT-94-SHORT-VIEW), `(x, y, width,
/// height)` in [`GAUGE_SIZE`] units: the squircle band's geometry bounding
/// box (14..94 by 26..99.64) grown by one unit on each side and by about 6.3
/// units above and below, so an 8 px device-width stroke clears the gauge
/// body's content edge by about 3 px when the box is stretched to the body.
/// The short-view stylesheet stretches this box to the body, so the band
/// takes the body's aspect.
pub const GAUGE_WIDE_VIEWBOX: (f64, f64, f64, f64) = (13.0, 19.7, 82.0, 86.3);
pub const GAUGE_CX: f64 = 54.0;
pub const GAUGE_CY: f64 = 66.0;
pub const GAUGE_RADIUS: f64 = 40.0;
pub const GAUGE_STROKE: f64 = 8.0;
/// The readout's bound: the band's inner edge on the axes (36) minus a 2-unit
/// margin. The squircle's corners stand further out, never closer.
pub const GAUGE_READOUT_RADIUS: f64 = 34.0;
/// The superellipse's exponent: 4 is the classic squircle, a rectangle with
/// very round corners; 2 would be a circle.
const SQUIRCLE_EXPONENT: f64 = 4.0;
/// The circle's exponent, drawn by the circle the gauge fragment carries
/// (DEF-LAYOUT-90-LOAD-SHIFT).
const CIRCLE_EXPONENT: f64 = 2.0;
/// Tick and label distances from the centreline, along the shape's normal.
/// They are the old circle's radii 46, 50 and 53 and a label offset eight
/// units beyond the longest tick, so the stack keeps its spacing relative to
/// the band.
const TICK_INNER: f64 = 6.0;
const TICK_MINOR_OUTER: f64 = 10.0;
const TICK_MAJOR_OUTER: f64 = 13.0;
const TICK_LABEL_OFFSET: f64 = 21.0;
/// The gauge scale's per-character box: 0.78 em, 6.24 units at the 8 px
/// design size. It bounds the drawn advance, not the font's advance plus
/// tracking: Chromium lays the 8 px labels out per rendered pixel, so the
/// narrow face draws 0.758 em per character at 1280x720, 0.737 at 1440x900
/// and 0.703 at 1024x768, against the 0.7275 em advance, and the system face
/// draws 0.637 em. 0.78 em holds the widest with 0.02 em to spare while the
/// `100` box still ends inside the viewBox. assets/app.js carries the same
/// box as `GAUGE_SCALE_BOX.per_char` = 6.24.
pub const GAUGE_SCALE_CHAR_EM: f64 = 0.78;

/// The arc samples every two percent of the sweep: a chord strays under a
/// quarter of a rendered pixel from the squircle, and the payload stays half
/// the size of a one-percent sample. Its two end segments run along the exact
/// tangent, so the butt caps are square to the shape's normal.
const GAUGE_STEP: f64 = 2.0;
const GAUGE_CAP: f64 = 0.4;

/// A point on the superellipse of `exponent`, `out` units outward along the
/// shape's normal: 0 at the lower left, 50 at the top, 100 at the lower right,
/// over 270 degrees. Sampling by direction keeps the ticks evenly spaced, as
/// they were on the circle.
fn gauge_point_on(pct: f64, out: f64, exponent: f64) -> (f64, f64) {
    let theta = (-135.0 + 2.7 * pct.clamp(0.0, 100.0)) * PI / 180.0;
    let (sin, cos) = theta.sin_cos();
    let rho =
        GAUGE_RADIUS * (sin.abs().powf(exponent) + cos.abs().powf(exponent)).powf(-1.0 / exponent);
    let (x, y) = (rho * sin, -rho * cos);
    // Outward normal of |x|^n + |y|^n = R^n: (x|x|^(n-2), y|y|^(n-2)), normalised.
    let (nx, ny) = (
        x * x.abs().powf(exponent - 2.0),
        y * y.abs().powf(exponent - 2.0),
    );
    let norm = nx.hypot(ny);
    (
        GAUGE_CX + x + out * nx / norm,
        GAUGE_CY + y + out * ny / norm,
    )
}

/// A point on the squircle, the server's default shape; the tests read it.
#[cfg(test)]
fn gauge_point(pct: f64, out: f64) -> (f64, f64) {
    gauge_point_on(pct, out, SQUIRCLE_EXPONENT)
}

/// The unit tangent at percentage `pct`, pointing along the sweep.
fn gauge_tangent_on(pct: f64, exponent: f64) -> (f64, f64) {
    let step = 1e-4;
    let (x0, y0) = gauge_point_on((pct - step).max(0.0), 0.0, exponent);
    let (x1, y1) = gauge_point_on((pct + step).min(100.0), 0.0, exponent);
    let (dx, dy) = (x1 - x0, y1 - y0);
    let length = dx.hypot(dy);
    (dx / length, dy / length)
}

#[cfg(test)]
fn gauge_tangent(pct: f64) -> (f64, f64) {
    gauge_tangent_on(pct, SQUIRCLE_EXPONENT)
}

/// The arc from `from` to `to` percent along the superellipse of `exponent`.
/// A non-finite end or an empty span draws nothing; the full track is the
/// span from 0 to 100.
fn gauge_span_on(from: f64, to: f64, exponent: f64) -> String {
    if !(from.is_finite() && to.is_finite()) {
        return String::new();
    }
    let (from, to) = (from.max(0.0), to.min(100.0));
    if from >= to {
        return String::new();
    }
    let (x0, y0) = gauge_point_on(from, 0.0, exponent);
    let (x1, y1) = gauge_point_on(to, 0.0, exponent);
    let cap = GAUGE_CAP.min((x1 - x0).hypot(y1 - y0) / 2.0);
    let mut path = format!("M{x0:.3} {y0:.3}");
    let (tx, ty) = gauge_tangent_on(from, exponent);
    let _ = write!(path, "L{:.3} {:.3}", x0 + cap * tx, y0 + cap * ty);
    let mut at = (from / GAUGE_STEP).floor() * GAUGE_STEP + GAUGE_STEP;
    while at < to {
        let (x, y) = gauge_point_on(at, 0.0, exponent);
        let _ = write!(path, "L{x:.2} {y:.2}");
        at += GAUGE_STEP;
    }
    let (tx, ty) = gauge_tangent_on(to, exponent);
    let _ = write!(path, "L{:.3} {:.3}", x1 - cap * tx, y1 - cap * ty);
    let _ = write!(path, "L{x1:.3} {y1:.3}");
    path
}

/// The arc from 0 to `pct`. `None`, a non-finite value and zero or less draw
/// nothing.
fn gauge_arc_on(pct: Option<f64>, exponent: f64) -> String {
    pct.map_or_else(String::new, |pct| gauge_span_on(0.0, pct, exponent))
}

/// The gauge arc from 0 to `pct` along the squircle. `None`, a non-finite
/// value and zero or less draw nothing; the full track is `gauge_arc(Some(100.0))`.
pub fn gauge_arc(pct: Option<f64>) -> String {
    gauge_arc_on(pct, SQUIRCLE_EXPONENT)
}

/// The same arc along the circle, the preferred shape.
pub fn gauge_arc_circle(pct: Option<f64>) -> String {
    gauge_arc_on(pct, CIRCLE_EXPONENT)
}

/// One memory arc segment from `from` to `to` percent along the squircle
/// (ACC-VIEW-190-MEM-RECLAIM).
// shortcut: the squircle's percent runs by direction like its ticks, so a
// segment's length is within 1.05 points of its share while its ends sit on
// the printed scale; a length-true squircle moves every tick and label of both
// gauges, upgrade only if those lengths are read against the rows.
pub fn gauge_span(from: f64, to: f64) -> String {
    gauge_span_on(from, to, SQUIRCLE_EXPONENT)
}

/// The same segment along the circle, where a segment's length is its share.
pub fn gauge_span_circle(from: f64, to: f64) -> String {
    gauge_span_on(from, to, CIRCLE_EXPONENT)
}

/// Tick marks every 10 percent: the eight minor ticks, or with `major` the
/// three longer ones at 0, 50 and 100. Each runs along the shape's normal.
fn gauge_ticks_on(major: bool, exponent: f64) -> String {
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
        let (x0, y0) = gauge_point_on(pct, TICK_INNER, exponent);
        let (x1, y1) = gauge_point_on(pct, outer, exponent);
        let _ = write!(path, "M{x0:.2} {y0:.2}L{x1:.2} {y1:.2}");
    }
    path
}

/// The squircle's tick marks.
pub fn gauge_ticks(major: bool) -> String {
    gauge_ticks_on(major, SQUIRCLE_EXPONENT)
}

/// The circle's tick marks.
pub fn gauge_ticks_circle(major: bool) -> String {
    gauge_ticks_on(major, CIRCLE_EXPONENT)
}

/// The faint tick across the band itself at every 10 percent: the segment
/// from the band's inner edge to its outer edge along the shape's normal
/// (ACC-VIEW-172). Drawn over the track and the fill, so app.css strokes it
/// with `--surface` at 0.3 opacity, which theme_contrast.rs holds faint but
/// visible on the track and on every fill in both modes. The wide gauge draws
/// no ticks and no labels: its viewBox is the
/// band's geometry box, cropped to the band, and app.css stretches it to the
/// body's aspect, so a mark or a glyph would be cut at the edge or distorted
/// along one axis; the short-view stylesheet drops the scale for the same
/// reason (DEF-LAYOUT-94-SHORT-VIEW).
fn gauge_inner_ticks_on(exponent: f64) -> String {
    let mut path = String::new();
    for step in 0..=10 {
        let pct = f64::from(step) * 10.0;
        let (x0, y0) = gauge_point_on(pct, -GAUGE_STROKE / 2.0, exponent);
        let (x1, y1) = gauge_point_on(pct, GAUGE_STROKE / 2.0, exponent);
        let _ = write!(path, "M{x0:.2} {y0:.2}L{x1:.2} {y1:.2}");
    }
    path
}

/// The squircle's inner ticks.
pub fn gauge_inner_ticks() -> String {
    gauge_inner_ticks_on(SQUIRCLE_EXPONENT)
}

/// The circle's inner ticks.
pub fn gauge_inner_ticks_circle() -> String {
    gauge_inner_ticks_on(CIRCLE_EXPONENT)
}

/// The eleven scale labels `0`, `10`, ..., `100%` at [`TICK_LABEL_OFFSET`]:
/// the last carries the percent unit and is the only label with a unit
/// (ACC-VIEW-171), and the eight at 10, 20, 30, 40 and 60, 70, 80, 90 are
/// minor, drawn in the smaller `.g-scale.minor` size (ACC-VIEW-173).
fn gauge_labels_on(exponent: f64) -> [(f64, f64, &'static str, bool); 11] {
    const TEXT: [&str; 11] = [
        "0", "10", "20", "30", "40", "50", "60", "70", "80", "90", "100%",
    ];
    std::array::from_fn(|index| {
        let (x, y) = gauge_point_on(index as f64 * 10.0, TICK_LABEL_OFFSET, exponent);
        (x, y, TEXT[index], index != 0 && index != 5 && index != 10)
    })
}

/// The squircle's scale labels.
pub fn gauge_labels() -> [(f64, f64, &'static str, bool); 11] {
    gauge_labels_on(SQUIRCLE_EXPONENT)
}

/// The circle's scale labels.
pub fn gauge_labels_circle() -> [(f64, f64, &'static str, bool); 11] {
    gauge_labels_on(CIRCLE_EXPONENT)
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

    /// The design box of a scale label: [`GAUGE_SCALE_CHAR_EM`] per character
    /// wide by 1.175 em tall at the label's own design size, 8 px for a major
    /// and 5 px for a minor label (ACC-VIEW-173).
    fn label_box((x, y, text, minor): (f64, f64, &str, bool)) -> (f64, f64, f64, f64) {
        let design = if minor { 5.0 } else { 8.0 };
        let half_width = GAUGE_SCALE_CHAR_EM * design * text.len() as f64 / 2.0;
        let half_height = 1.175 * design / 2.0;
        (
            x - half_width,
            x + half_width,
            y - half_height,
            y + half_height,
        )
    }

    /// ACC-VIEW-171, ACC-VIEW-173: eleven labels, the 100 one carrying the
    /// percent sign, eight minor and three major, all at [`TICK_LABEL_OFFSET`].
    #[test]
    fn gauge_labels_are_eleven_with_one_unit_eight_minor_and_three_major() {
        const EXPECTED: [&str; 11] = [
            "0", "10", "20", "30", "40", "50", "60", "70", "80", "90", "100%",
        ];
        for labels in [gauge_labels(), gauge_labels_circle()] {
            assert_eq!(labels.map(|label| label.2), EXPECTED);
            let minor: Vec<&str> = labels
                .iter()
                .filter(|label| label.3)
                .map(|label| label.2)
                .collect();
            assert_eq!(minor, ["10", "20", "30", "40", "60", "70", "80", "90"]);
            let major: Vec<&str> = labels
                .iter()
                .filter(|label| !label.3)
                .map(|label| label.2)
                .collect();
            assert_eq!(major, ["0", "50", "100%"]);
        }
        for (index, label) in gauge_labels().iter().enumerate() {
            assert!(close(
                (label.0, label.1),
                gauge_point(index as f64 * 10.0, TICK_LABEL_OFFSET)
            ));
        }
    }

    /// ACC-VIEW-172: a tick crosses the band itself at every 10 percent, from
    /// its inner edge to its outer edge, in each exponent.
    #[test]
    fn gauge_inner_ticks_cross_the_band_at_every_tenth() {
        for (path, exponent) in [
            (gauge_inner_ticks(), SQUIRCLE_EXPONENT),
            (gauge_inner_ticks_circle(), CIRCLE_EXPONENT),
        ] {
            let drawn = points(&path);
            assert_eq!(drawn.len(), 22);
            for (index, pair) in drawn.chunks(2).enumerate() {
                let pct = index as f64 * 10.0;
                assert!(close(
                    pair[0],
                    gauge_point_on(pct, -GAUGE_STROKE / 2.0, exponent)
                ));
                assert!(close(
                    pair[1],
                    gauge_point_on(pct, GAUGE_STROKE / 2.0, exponent)
                ));
            }
        }
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

    /// DEF-GRAPH-51-GPU-LANES, DEF-GRAPH-54-LANES-MERGED: a lane is the GPU
    /// plot's height split by the device count with the CSS 4-unit gaps, at
    /// the plot's width.
    #[test]
    fn gpu_lanes_split_the_plot_height() {
        for (devices, height) in [
            (0, 135.0),
            (1, 135.0),
            (2, 65.0),
            (3, 42.0),
            (4, 30.0),
            (8, 13.0),
        ] {
            let lane = gpu_lane(devices);
            assert_eq!(
                (lane.width, lane.height),
                (GPU_PLOT.width, height),
                "{devices}"
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

    /// A process row graph takes the line alone: the chart's line without the
    /// scrolling tail (ACC-GRAPH-98-ANIM-CLOCK).
    #[test]
    fn a_row_graph_is_the_chart_line_without_its_tail() {
        let mut history = History::with_window(PROC_WINDOW_MS);
        for tick in 0..100 {
            history.push(Some(f64::from(tick % 7) * 9.0), 250);
            let row = history.line(PROC_GRAPH, 0.0, 100.0);
            let chart = history.chart(PROC_GRAPH, 0.0, 100.0).line;
            if row.is_empty() {
                assert!(chart.is_empty());
                continue;
            }
            assert!(chart.starts_with(&row), "{chart}");
            assert!(chart.len() > row.len(), "{chart}");
        }
    }

    #[test]
    fn a_full_history_builds_the_curve_and_its_area() {
        let slots = capacity(WINDOW_MS, 2000);
        let step = CPU_HISTORY.width / (slots - 1) as f64;
        let steps = step.floor() as usize;
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
        assert!(
            chart.line.starts_with(&format!("M{step:.2} ")),
            "{}",
            &chart.line[..12]
        );
        // A slot is 467 / 89 = 5.25 units wide: the knot and four PCHIP
        // vertices per interval, then the tail.
        assert_eq!(chart.line.matches('L').count(), (slots - 2) * steps + 1);
        assert!(
            chart.area.ends_with(&format!("L{step:.2} 28.00Z")),
            "{}",
            chart.area
        );
        assert!(chart.area.ends_with('Z'));
        assert!(!chart.line.contains("NaN"));
    }

    #[test]
    fn history_keeps_the_newest_capacity_samples() {
        let mut history = History::new();
        for value in 0..capacity(WINDOW_MS, 250) + 5 {
            history.push(Some(value as f64), 250);
        }
        assert_eq!(history.len(), 720);
        history.push(None, 250);
        assert_eq!(history.len(), 720);
        // A longer interval shrinks the window to its own capacity.
        history.push(Some(1.0), 2000);
        assert_eq!(history.len(), 90);
    }

    /// ACC-VIEW-110-TIME-AXIS: a wider window keeps the samples held, so the
    /// line starts at the oldest and fills from the left; a narrower one drops
    /// the oldest samples on the next push.
    #[test]
    fn a_window_change_resizes_the_ring_without_moving_a_sample() {
        let mut history = History::new();
        for value in 0..300 {
            history.push(Some(value as f64), 250);
        }
        assert_eq!(history.len(), 300);
        history.set_window(60_000);
        history.push(Some(300.0), 250);
        assert_eq!(history.len(), 240, "the 1 minute window keeps 240 slots");
        history.set_window(600_000);
        history.push(Some(301.0), 250);
        assert_eq!(history.len(), 241, "the held samples are kept");
        let drawn = points(&history.chart(CPU_HISTORY, 0.0, 400.0).line);
        assert!(
            drawn[0].0 > CPU_HISTORY.width / 2.0,
            "a partial window starts at its oldest sample, mid-plot: {:?}",
            drawn[0]
        );
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
        for _ in 0..capacity(WINDOW_MS, 250) {
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
            let slots = capacity(WINDOW_MS, 250);
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
                    // The tail past the right edge is no knot: it
                    // moves with the newest knot, not with the samples.
                    if x > size.width + 0.001 {
                        continue;
                    }
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
                let slots = capacity(WINDOW_MS, update_ms);
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
    fn capacity_keeps_the_default_window() {
        assert_eq!(capacity(WINDOW_MS, 250), 720);
        assert_eq!(capacity(WINDOW_MS, 2000), 90);
        assert_eq!(capacity(WINDOW_MS, 100), 1800);
        assert_eq!(capacity(WINDOW_MS, 5000), 36);
        // The window option sizes the ring: 1, 2, 3, 5 and 10 minutes.
        assert_eq!(capacity(60_000, 250), 240);
        assert_eq!(capacity(600_000, 250), 2400);
        assert_eq!(window(600_000, 250), 600_000);
        assert_eq!(window(180_000, 250), 180_000);
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

    /// ACC-GRAPH-97-SMOOTH-REFRESH: the constant is 1 s of wall-clock time at
    /// every interval, so a step moves by `1 - exp(-update_ms / 1000)`,
    /// 0.221 at 250 ms and 0.632 at 1 s, and never by a fixed per-sample
    /// fraction. A sparse sampler graphs choppier on purpose.
    #[test]
    fn smoothing_constant_stays_one_second_at_every_interval() {
        assert_eq!(SMOOTHING_TAU_MS, 1000.0);
        for update_ms in [100, 250, 500, 1000, 2000] {
            // `smooth` from 0 towards 1 moves by exactly alpha.
            let alpha = smooth(Some(0.0), 1.0, update_ms);
            let expected = 1.0 - (-(update_ms as f64) / 1000.0).exp();
            assert!(
                (alpha - expected).abs() < 1e-12,
                "{update_ms} ms: {alpha} != {expected}"
            );
        }
        // A wall-clock constant composes over time: ten 100 ms steps reach
        // exactly what one 1000 ms step reaches.
        let mut stepped = 0.0;
        for _ in 0..10 {
            stepped = smooth(Some(stepped), 1.0, 100);
        }
        let one_step = smooth(Some(0.0), 1.0, 1000);
        assert!(
            (stepped - one_step).abs() < 1e-12,
            "ten 100 ms steps reached {stepped}, one 1000 ms step {one_step}"
        );
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
        let newest = drawn[drawn.len() - 2].0;
        let tail = drawn.last().expect("last point").0;
        assert!(
            (first - width * 958.0 / 959.0).abs() < 0.01,
            "first x {first}"
        );
        assert!((newest - width).abs() < 0.01, "newest x {newest}");
        // The tail reaches a sixth of the plot past the right edge.
        assert!((tail - (width + width / 6.0)).abs() < 0.01, "tail x {tail}");
        assert!(
            chart.area.ends_with(&format!("L{first:.2} 28.00Z")),
            "{}",
            chart.area
        );
    }

    /// DEF-GRAPH-125-TAIL-GAP: the tail covers the page's whole scroll, one slow
    /// pass over the window (`--shift-to` in app.js), for a history that takes
    /// one sample per tick and one that takes one per pass, at every interval
    /// up to the point where a pass is one tick, and at every window and
    /// process refresh period the config offers. The scroll is the sampler's
    /// `every * update_ms` over `window`.
    #[test]
    fn a_tail_covers_the_longest_scroll_the_config_allows() {
        let tail = |slots: usize| {
            let chart = ChartPath::new(&[1.0, 2.0], 0, slots, 467.0, 28.0, 0.0, 100.0);
            points(&chart.line).last().expect("a tail").0 - 467.0
        };
        for update_ms in (100..12_000)
            .step_by(37)
            .chain([4616, 4999, 40_000, 86_400_000])
        {
            for proc_refresh_ms in [250, 500, 1000, 2000, 5000_u64] {
                for window_s in [60, 120, 180, 300, 600_u64] {
                    let window_ms = window_s * 1000;
                    let every = proc_refresh_ms.max(update_ms).div_ceil(update_ms);
                    let pass = every * update_ms;
                    let scroll = 467.0 * pass as f64 / window(window_ms, update_ms) as f64;
                    for slots in [capacity(window_ms, update_ms), capacity(window_ms, pass)] {
                        assert!(
                            tail(slots) >= scroll - 0.005,
                            "{update_ms} ms, {proc_refresh_ms} ms, {window_s} s: tail {} under scroll {scroll}",
                            tail(slots)
                        );
                    }
                }
            }
        }
    }

    /// ACC-GRAPH-98-ANIM-CLOCK: a panel chart extends its newest, smoothed
    /// knot past the most the page scrolls the series between readouts; the
    /// tail holds the value the curve already draws. A process row graph is
    /// never scrolled and has no tail.
    #[test]
    fn a_panel_chart_extends_its_newest_knot_past_its_scroll() {
        let mut history = History::new();
        history.push(Some(20.0), 250);
        history.push(Some(80.0), 250);
        let chart = history.chart(CPU_HISTORY, 0.0, 100.0);
        let drawn = points(&chart.line);
        let newest = drawn[drawn.len() - 2];
        let tail = drawn[drawn.len() - 1];
        assert_eq!(tail.1, newest.1, "the tail holds the newest knot's value");
        assert!((newest.0 - CPU_HISTORY.width).abs() < 0.01, "{newest:?}");
        assert!(
            (tail.0 - (CPU_HISTORY.width + CPU_HISTORY.width / 6.0)).abs() < 0.01,
            "{tail:?}"
        );
        assert!(
            chart
                .line
                .ends_with(&format!("L{:.2} {:.2}", tail.0, tail.1)),
            "{}",
            chart.line
        );
        assert!(
            chart.area.contains(&format!("L{:.2} 28.00L", tail.0)),
            "{}",
            chart.area
        );
        // A 60 s interval keeps 3 samples: a scroll is a third of the plot.
        history.push(Some(80.0), 60_000);
        assert_eq!(history.capacity, 3);
        let wide = history.chart(CPU_HISTORY, 0.0, 100.0);
        let drawn = points(&wide.line);
        assert!((drawn.last().expect("a tail").0 - 467.0 - 467.0 / 3.0).abs() < 0.01);
        // A process row graph draws `line` and reaches only the right edge.
        let mut row = History::with_window(PROC_WINDOW_MS);
        row.push(Some(30.0), 250);
        row.push(Some(70.0), 250);
        let graph = points(&row.line(PROC_GRAPH, 0.0, 100.0));
        assert!(
            graph.last().expect("a last vertex").0 <= PROC_GRAPH.width,
            "{graph:?}"
        );
    }

    fn close(actual: (f64, f64), expected: (f64, f64)) -> bool {
        (actual.0 - expected.0).abs() < 0.01 && (actual.1 - expected.1).abs() < 0.01
    }

    /// ACC-VIEW-132-GAUGE-SQUIRCLE: the arc is a 270-degree segment of a
    /// superellipse, 0 at the lower left, 50 at the top, 100 at the lower
    /// right; the radius at 45 degrees beats the axes' by at least 8 percent.
    #[test]
    fn gauge_arc_follows_the_squircle_over_270_degrees() {
        let drawn = points(&gauge_arc(Some(100.0)));
        assert!(close(drawn[0], gauge_point(0.0, 0.0)), "{:?}", drawn[0]);
        assert!(close(
            *drawn.last().expect("an end"),
            gauge_point(100.0, 0.0)
        ));
        let half = points(&gauge_arc(Some(50.0)));
        assert!(close(*half.last().expect("an end"), gauge_point(50.0, 0.0)));
        assert!(close(gauge_point(50.0, 0.0), (54.0, 26.0)));
        // Every vertex lies on the implicit superellipse.
        for (x, y) in &drawn {
            let (u, v) = ((x - GAUGE_CX) / GAUGE_RADIUS, (y - GAUGE_CY) / GAUGE_RADIUS);
            let f = u.abs().powf(SQUIRCLE_EXPONENT) + v.abs().powf(SQUIRCLE_EXPONENT);
            assert!((f - 1.0).abs() < 1e-3, "{f} at ({x}, {y})");
        }
        let (x, y) = gauge_point((45.0 + 135.0) / 2.7, 0.0);
        let diagonal = (x - GAUGE_CX).hypot(y - GAUGE_CY) / GAUGE_RADIUS;
        assert!(diagonal >= 1.08, "45 degrees: {diagonal}");
        for none in [Some(0.0), None, Some(f64::NAN), Some(-5.0)] {
            assert_eq!(gauge_arc(none), "", "{none:?}");
        }
        assert_eq!(gauge_arc(Some(150.0)), gauge_arc(Some(100.0)));
    }

    /// The length of a drawn gauge path.
    fn path_length(path: &str) -> f64 {
        points(path)
            .windows(2)
            .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
            .sum()
    }

    /// DEF-VIEW-112-WIDE-ARC: the stylesheet's px length of the stretched band
    /// holds 0.7 percent of the length drawn, from a 270 px box to an 1800 px box.
    #[test]
    fn wide_band_length_in_the_stylesheet_follows_the_stretch() {
        assert!(include_str!("../assets/app.css").contains("calc(1.096cqw + 1.185cqh)"));
        let (_, _, view_width, view_height) = GAUGE_WIDE_VIEWBOX;
        let band = points(&gauge_arc(Some(100.0)));
        for width in [270.0, 481.0, 609.0, 689.0, 1240.0, 1800.0] {
            let (scale_x, scale_y) = (width / view_width, 96.0 / view_height);
            let drawn: f64 = band
                .windows(2)
                .map(|pair| {
                    ((pair[1].0 - pair[0].0) * scale_x).hypot((pair[1].1 - pair[0].1) * scale_y)
                })
                .sum();
            let formula = 1.096 * width + 1.185 * 96.0;
            assert!(
                (formula / drawn - 1.0).abs() < 0.007,
                "{width}: {formula} {drawn}"
            );
        }
    }

    /// ACC-VIEW-175-MEM-SHARE, ACC-VIEW-190-MEM-RECLAIM: the four segments
    /// run in table order from 0, each starting where the last one ended. On
    /// the circle each segment's length is its row's share of the track; on
    /// the squircle each boundary lands on the printed scale at the running
    /// share, and the length stays within 1.05 points of the share, the limit
    /// `gauge_span` records.
    #[test]
    fn each_memory_arc_segment_is_its_rows_share() {
        const SHARES: [f64; 4] = [18.64, 16.17, 8.9, 55.2];
        let track = path_length(&gauge_arc_circle(Some(100.0)));
        let squircle_track = path_length(&gauge_arc(Some(100.0)));
        let mut at = 0.0;
        for share in SHARES {
            let circle = gauge_span_circle(at, at + share);
            let measured = path_length(&circle) * 100.0 / track;
            assert!((measured - share).abs() < 0.1, "{share}: {measured}");
            let start = gauge_point_on(at, 0.0, CIRCLE_EXPONENT);
            let end = gauge_point_on(at + share, 0.0, CIRCLE_EXPONENT);
            let drawn = points(&circle);
            assert!(close(drawn[0], start) && close(*drawn.last().expect("end"), end));

            let squircle = gauge_span(at, at + share);
            let drawn = points(&squircle);
            assert!(close(drawn[0], gauge_point(at, 0.0)), "{at}");
            assert!(close(
                *drawn.last().expect("end"),
                gauge_point(at + share, 0.0)
            ));
            let measured = path_length(&squircle) * 100.0 / squircle_track;
            assert!((measured - share).abs() < 1.05, "{share}: {measured}");
            at += share;
        }
        assert_eq!(gauge_span(0.0, 40.0), gauge_arc(Some(40.0)));
        for (from, to) in [
            (50.0, 50.0),
            (60.0, 50.0),
            (f64::NAN, 10.0),
            (0.0, f64::NAN),
        ] {
            assert_eq!(gauge_span(from, to), "", "{from} {to}");
        }
    }

    /// ACC-VIEW-132: both end segments run within half a degree of the shape's
    /// tangent, so the butt caps stay square to the normal at 0 and 100
    /// percent and the value reads as a clean cut.
    #[test]
    fn gauge_arc_ends_square_to_the_normal() {
        for pct in [100.0, 50.0, 12.3, 0.5] {
            let drawn = points(&gauge_arc(Some(pct)));
            assert!(close(drawn[0], gauge_point(0.0, 0.0)), "{pct}");
            assert!(close(*drawn.last().expect("an end"), gauge_point(pct, 0.0)));
            for (pair, at) in [
                ((drawn[0], drawn[1]), 0.0),
                ((drawn[drawn.len() - 2], drawn[drawn.len() - 1]), pct),
            ] {
                let (sx, sy) = (pair.1.0 - pair.0.0, pair.1.1 - pair.0.1);
                let (tx, ty) = gauge_tangent(at);
                let cos = (sx * tx + sy * ty) / (sx.hypot(sy) * tx.hypot(ty));
                let angle = cos.clamp(-1.0, 1.0).acos().to_degrees();
                assert!(angle < 0.5, "{pct} at {at}: {angle} degrees");
            }
        }
    }

    #[test]
    fn gauge_ticks_are_eleven_with_three_major_outside_the_arc() {
        // The stack's geometry: ticks start 2 units outside the band's edge,
        // the readout clears the band's inner edge, and the labels clear the
        // longest ticks.
        const {
            assert!(TICK_INNER > GAUGE_STROKE / 2.0);
            assert!(GAUGE_READOUT_RADIUS < GAUGE_RADIUS - GAUGE_STROKE / 2.0);
            assert!(TICK_LABEL_OFFSET - 4.0 > TICK_MAJOR_OUTER);
        }
        let minor = gauge_ticks(false);
        let major = gauge_ticks(true);
        assert_eq!(minor.matches('M').count(), 8);
        assert_eq!(major.matches('M').count(), 3);
        let mut ends = Vec::new();
        for (path, outer, steps) in [
            (
                &minor,
                TICK_MINOR_OUTER,
                &[10.0, 20.0, 30.0, 40.0, 60.0, 70.0, 80.0, 90.0][..],
            ),
            (&major, TICK_MAJOR_OUTER, &[0.0, 50.0, 100.0][..]),
        ] {
            let drawn = points(path);
            assert_eq!(drawn.len(), steps.len() * 2);
            for (index, pair) in drawn.chunks(2).enumerate() {
                let pct = steps[index];
                assert!(close(pair[0], gauge_point(pct, TICK_INNER)), "{pct}");
                assert!(close(pair[1], gauge_point(pct, outer)), "{pct}");
                ends.push(pair[1]);
            }
        }
        // A label box clears every tick end and stays in the viewBox; two
        // label boxes never overlap (ACC-VIEW-173).
        let labels = gauge_labels();
        let boxes: Vec<(f64, f64, f64, f64)> =
            labels.iter().map(|label| label_box(*label)).collect();
        for (index, label) in labels.iter().enumerate() {
            let (left, right, top, bottom) = boxes[index];
            let text = label.2;
            assert!(
                left >= GAUGE_VIEW_X && right <= GAUGE_VIEW_X + GAUGE_SIZE.width,
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
                    dx.hypot(dy) >= 0.5,
                    "{text} touches the tick at ({tx}, {ty})"
                );
            }
            for (other, boxed) in boxes.iter().take(index).enumerate() {
                let (oleft, oright, otop, obottom) = *boxed;
                assert!(
                    right <= oleft || oright <= left || bottom <= otop || obottom <= top,
                    "{text} overlaps {}",
                    labels[other].2
                );
            }
        }
    }

    /// DEF-LAYOUT-94-SHORT-VIEW: the wide gauge's viewBox is the squircle
    /// band's geometry box plus a stroke margin, so the stretched band fills
    /// the gauge body and its 8 px stroke clears the content edge. Every
    /// point of the full arc stays inside it, and the ring's centre sits at
    /// half the width and 53.65 percent of the height, where the short-view
    /// stylesheet centres the HTML readout.
    #[test]
    fn wide_gauge_viewbox_is_the_band_geometry() {
        let (x, y, width, height) = GAUGE_WIDE_VIEWBOX;
        for (px, py) in points(&gauge_arc(Some(100.0))) {
            assert!(
                px >= x && px <= x + width && py >= y && py <= y + height,
                "({px}, {py}) outside {GAUGE_WIDE_VIEWBOX:?}"
            );
        }
        let across = (GAUGE_CX - x) / width;
        let down = (GAUGE_CY - y) / height;
        assert!((across - 0.5).abs() < 0.001, "across {across}");
        assert!((down - 0.5365).abs() < 0.001, "down {down}");
    }

    /// DEF-LAYOUT-90-LOAD-SHIFT: the circle is the same 270-degree band with
    /// exponent 2. Its 0 and 100 ends sit level at (25.716, 94.284) and
    /// (82.284, 94.284), its body vertices trace radius 40 about (54, 66), and
    /// its scale labels fit the 112x114 viewBox the stylesheet shows.
    #[test]
    fn circle_gauge_is_the_circle_inside_its_own_box() {
        let drawn = points(&gauge_arc_circle(Some(100.0)));
        assert!(close(drawn[0], (25.716, 94.284)), "{:?}", drawn[0]);
        assert!(close(*drawn.last().expect("an end"), (82.284, 94.284)));
        assert!(close(
            gauge_point_on(50.0, 0.0, CIRCLE_EXPONENT),
            (54.0, 26.0)
        ));
        for (x, y) in &drawn[2..drawn.len() - 2] {
            let radius = (x - GAUGE_CX).hypot(y - GAUGE_CY);
            assert!(
                (radius - GAUGE_RADIUS).abs() < 0.01,
                "{radius} at ({x}, {y})"
            );
        }
        // The 100 percent fill ends on the track's end, and the two shapes
        // never draw the same path.
        assert_eq!(gauge_arc_circle(Some(100.0)), gauge_arc_circle(Some(150.0)));
        assert_ne!(gauge_arc_circle(Some(50.0)), gauge_arc(Some(50.0)));
        assert_eq!(gauge_arc_circle(None), "");
        // The label boxes, GAUGE_SCALE_CHAR_EM em per character by 1.175 em
        // at each label's own size, fit the widened viewBox.
        for label in gauge_labels_circle() {
            let (left, right, top, bottom) = label_box(label);
            let text = label.2;
            assert!(
                left >= GAUGE_VIEW_X && right <= GAUGE_VIEW_X + GAUGE_CIRCLE_SIZE.width,
                "{text}: {left}..{right}"
            );
            assert!(
                top >= 0.0 && bottom <= GAUGE_CIRCLE_SIZE.height,
                "{text}: {top}..{bottom}"
            );
        }
        assert_eq!(gauge_ticks_circle(false).matches('M').count(), 8);
        assert_eq!(gauge_ticks_circle(true).matches('M').count(), 3);
        assert_eq!(gauge_inner_ticks_circle().matches('M').count(), 11);
    }
}

//! Router integration: in-process `oneshot` requests for every route and one
//! real socket, so the axum/hyper path is exercised at least once.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use btop_web::AppState;
use btop_web::chart::{
    ChartPath, PROC_GRAPH, gauge_arc, gauge_arc_circle, gauge_span, gauge_span_circle, gpu_lane,
};
use btop_web::collector::frame::{Frame, fragments};
use btop_web::collector::{
    CoreUsage, CpuSnapshot, GpuDevice, MemSnapshot, Snapshot, cpu, gpu, mem, net, proc,
};
use btop_web::config::{
    CONFIG_PAGES, Config, Connect, ConnectKey, Control, LoadScale, SERVER_OPTIONS,
    default_config_text,
};
use btop_web::routes::READ_ONLY_REFUSAL;
use btop_web::routes::dashboard::PANELS;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;
use tower::ServiceExt;

fn app(snapshot: Option<Snapshot>, readonly: bool) -> Router {
    let (_sender, receiver) =
        watch::channel(snapshot.map(|snapshot| Arc::new(Frame::new(snapshot))));
    btop_web::app(AppState::new(receiver, readonly))
}

/// One snapshot of a recorded tree, collected `ticks` times so the histories
/// hold that many samples.
fn recorded_snapshot(relative: &str, ticks: usize) -> Snapshot {
    recorded_snapshot_with(relative, ticks, Config::load(None).expect("config"))
}

fn recorded_snapshot_with(relative: &str, ticks: usize, config: Config) -> Snapshot {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let mut mem = mem::MemCollector::default();
    let mut snapshot = Snapshot::default();
    for _ in 0..ticks {
        let mut gpu = gpu::collect(&root, &config);
        gpu::attach_charts(
            &root,
            &mut gpu.devices,
            config.update_ms,
            config.window_ms(),
        );
        snapshot = Snapshot {
            cpu: cpu::collect(&root, &config),
            mem: mem.collect(&root, &config),
            net: net::collect(&root, &config),
            proc: proc::collect(&root, &config),
            gpu,
            update_ms: config.update_ms,
            window_ms: btop_web::chart::window(config.window_ms(), config.update_ms),
            slow_ms: config.proc_refresh_ms.max(config.update_ms),
            slow_seq: 1,
            container: config.container.is_some(),
        };
    }
    snapshot
}

/// The empty fixture tree the DEF-VIEW-8-UNAVAILABLE-BAR repro selects with
/// `BTOP_WEB_PROC_ROOT`: no collector finds a value.
fn unavailable_snapshot() -> Snapshot {
    recorded_snapshot("tests/fixtures/cpu/unavailable/proc", 1)
}

/// The recorded host after two ticks, so every history draws, plus one GPU
/// with a utilization series (the recorded tree has none).
fn fixture_snapshot() -> Snapshot {
    let mut snapshot = recorded_snapshot("tests/fixtures/proc", 2);
    snapshot.gpu.devices.push(GpuDevice {
        name: "RTX PRO 4000 Blackwell".to_string(),
        utilization_pct: Some(37.0),
        chart: ChartPath::new(
            &[12.0, 37.0],
            0,
            960,
            gpu_lane(1).width,
            gpu_lane(1).height,
            0.0,
            100.0,
        ),
        ..GpuDevice::default()
    });
    snapshot.proc.entries[0].graph = ChartPath::new(
        &[0.0, 5.0],
        0,
        80,
        PROC_GRAPH.width,
        PROC_GRAPH.height,
        0.0,
        100.0,
    );
    snapshot.proc.entries[0].graph_tone = "cpu-ok";
    snapshot
}

/// Every element whose opening tag carries `attribute`, as its opening tag
/// and its inner HTML. Enough HTML for the server's own templates: the
/// matching close tag is found by counting nested tags of the same name.
fn elements_with<'a>(html: &'a str, attribute: &str) -> Vec<(&'a str, &'a str)> {
    let needle = format!(" {attribute}=");
    let mut found = Vec::new();
    let mut offset = 0;
    while let Some(position) = html[offset..].find(&needle) {
        let at = offset + position;
        let open = html[..at].rfind('<').expect("tag start");
        let name_end = open + 1 + html[open + 1..].find([' ', '>']).expect("tag name");
        let name = &html[open + 1..name_end];
        let body = at + html[at..].find('>').expect("tag end") + 1;
        let (opening, closing) = (format!("<{name}"), format!("</{name}>"));
        let (mut depth, mut cursor) = (1, body);
        let end = loop {
            let close = cursor + html[cursor..].find(&closing).expect("close tag");
            match html[cursor..].find(&opening).map(|index| cursor + index) {
                Some(nested) if nested < close => {
                    depth += 1;
                    cursor = nested + 1;
                }
                _ => {
                    depth -= 1;
                    if depth == 0 {
                        break close;
                    }
                    cursor = close + 1;
                }
            }
        };
        found.push((&html[open..body], &html[body..end]));
        offset = body;
    }
    found
}

/// The `id` of every out-of-band wrapper in an event fragment.
fn oob_targets(fragment: &str) -> Vec<&str> {
    elements_with(fragment, "hx-swap-oob")
        .into_iter()
        .map(|(tag, _)| {
            let start = tag.find("id=\"").expect("oob wrapper id") + 4;
            let end = start + tag[start..].find('"').expect("id end");
            &tag[start..end]
        })
        .collect()
}

async fn request(app: &Router, method: Method, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec();
    (status, headers, body)
}

async fn get(app: &Router, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    request(app, Method::GET, uri).await
}

/// ACC-NET-144-ACCESS-KEY: a GET carrying the agent's key, if any.
async fn request_with_key(
    app: &Router,
    uri: &str,
    key: Option<&str>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    if let Some(key) = key {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec();
    (status, headers, body)
}

/// Status only, for a stream body that never ends.
async fn status_with_key(app: &Router, uri: &str, key: Option<&str>) -> StatusCode {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    if let Some(key) = key {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response")
        .status()
}

/// Status only: a permitted `/events` body never ends, so it is not read.
async fn status(app: &Router, method: Method, uri: &str) -> StatusCode {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    app.clone()
        .oneshot(request)
        .await
        .expect("response")
        .status()
}

fn content_type(headers: &HeaderMap) -> &str {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}

#[tokio::test]
async fn page_has_every_panel_and_theme_option() {
    let (status, headers, body) = get(&app(Some(Snapshot::default()), true), "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type(&headers).starts_with("text/html"));
    let page = String::from_utf8(body).expect("utf-8 page");
    for marker in [
        "btop-web",
        "sse-connect=\"events\"",
        "sse-swap=\"cpu\"",
        "sse-swap=\"mem\"",
        "sse-swap=\"net\"",
        "sse-swap=\"proc\"",
        "sse-swap=\"gpu\"",
    ] {
        assert!(page.contains(marker), "page lacks {marker}");
    }
    for theme in [
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
            page.contains(&format!("value=\"{theme}\"")),
            "theme picker lacks {theme}"
        );
    }
    // ACC-THEME-155-FONT-TOGGLE: the font is a control of its own, so the
    // picker carries no face pair any more.
    assert!(
        !page.contains("default-galaxatron") && !page.contains("Default GalaxaTron"),
        "the theme picker still offers the removed face pair"
    );
    // ACC-THEME-101-CYBERPUNK, ACC-THEME-152-STEEL: the appended pairs list
    // last, cyberpunk ninth and Steel after it, so a stored number keeps its
    // theme.
    let at = page.find("id=\"theme-picker\"").expect("theme picker");
    let select = &page[at..at + page[at..].find("</select>").expect("closed picker")];
    let options: Vec<&str> = select
        .match_indices("value=\"")
        .map(|(start, needle)| {
            let value = &select[start + needle.len()..];
            &value[..value.find('"').expect("option end")]
        })
        .collect();
    assert_eq!(options.len(), 10, "theme options: {options:?}");
    assert_eq!(options[0], "default", "theme options: {options:?}");
    let position = |id: &str| {
        options
            .iter()
            .position(|option| *option == id)
            .unwrap_or_else(|| panic!("theme options lack {id}: {options:?}"))
    };
    assert_eq!(position("cyberpunk"), 8, "theme options: {options:?}");
    assert_eq!(position("steel"), 9, "theme options: {options:?}");
    assert!(
        page.contains("--surface:"),
        "theme variables are not inlined"
    );
}

/// ACC-THEME-69-MODE-AUTO: the mode row offers dark, light and auto, and a
/// fresh viewer starts on auto.
#[tokio::test]
async fn mode_row_offers_dark_light_and_auto() {
    let (_, _, body) = get(&app(Some(Snapshot::default()), true), "/").await;
    let page = String::from_utf8(body).expect("utf-8 page");
    let start = page.find("data-group=\"mode\"").expect("mode row");
    let row = &page[start..start + page[start..].find("</div>").expect("closed row")];
    for value in ["dark", "light", "auto"] {
        assert!(
            row.contains(&format!("data-value=\"{value}\"")),
            "mode row lacks {value}"
        );
    }
    assert_eq!(row.matches("class=\"opt on\"").count(), 1, "{row}");
    assert!(row.contains("class=\"opt on\" type=\"button\" data-value=\"auto\""));
}

/// The page is served behind the JupyterLab proxy at a nested path, so every
/// asset and the SSE endpoint resolve relative to the trailing-slash URL.
#[tokio::test]
async fn page_references_assets_and_events_relatively() {
    let page = String::from_utf8(get(&app(Some(Snapshot::default()), true), "/").await.2)
        .expect("utf-8 page");
    for marker in [
        "href=\"assets/app.css?v=",
        "src=\"assets/htmx.min.js?v=",
        "src=\"assets/htmx-sse.min.js?v=",
        "src=\"assets/theme.js?v=",
        "src=\"assets/app.js?v=",
        "sse-connect=\"events\"",
    ] {
        assert!(page.contains(marker), "page lacks {marker}");
    }
    for absolute in [
        "\"/assets",
        "\"/events",
        "hx-get=\"/agents",
        "href=\"/?agent",
    ] {
        assert!(
            !page.contains(absolute),
            "page carries the absolute reference {absolute}"
        );
    }
}

/// DEF-VIEW-98-ASSET-STALE: one build stamp on every asset URL, and it is the
/// stamp this build computes, so a reload after a deploy cannot pair an old
/// script with new markup. The icon travels on the same stamp.
#[tokio::test]
async fn every_asset_url_carries_the_one_build_stamp() {
    let page = String::from_utf8(get(&app(Some(Snapshot::default()), true), "/").await.2)
        .expect("utf-8 page");
    let stamp = btop_web::routes::asset_stamp();
    let mut urls = 0;
    for (at, _) in page.match_indices("assets/") {
        urls += 1;
        let rest = &page[at..];
        let end = rest.find(['"', '\'']).unwrap_or(rest.len());
        let url = &rest[..end];
        assert!(
            url.contains(&format!("?v={stamp}")),
            "asset URL without this build's stamp: {url}"
        );
    }
    // icon.svg, app.css, htmx.min.js, htmx-sse.min.js, theme.js and app.js.
    assert_eq!(urls, 6, "the page names {urls} asset URLs");
}

/// DEF-VIEW-7-CPU-NAME: the collected model name is one titled row that CSS
/// truncates; when the collector has no name there is no row at all.
#[tokio::test]
async fn cpu_name_row_renders_only_when_collected() {
    const NAME: &str = "AMD Ryzen 9 7950X 16-Core Processor";
    let named = Snapshot {
        cpu: CpuSnapshot {
            name: Some(NAME.to_string()),
            ..CpuSnapshot::default()
        },
        ..Snapshot::default()
    };
    let page = String::from_utf8(get(&app(Some(named), true), "/").await.2).expect("utf-8 page");
    assert!(
        page.contains(&format!(
            r#"<div class="cpu-name" data-p="3" data-field="cpu-name" title="{NAME}">{NAME}</div>"#
        )),
        "the collected CPU name is not a titled row"
    );

    let page = String::from_utf8(get(&app(Some(Snapshot::default()), true), "/").await.2)
        .expect("utf-8 page");
    assert!(!page.contains("cpu-name"), "an absent name rendered a row");
}

/// DEF-VIEW-8-UNAVAILABLE-BAR: the empty fixture tree leaves every meter
/// without a value, so each draws the unknown state and none draws a zero.
#[tokio::test]
async fn unavailable_meters_render_their_unknown_state() {
    let page = String::from_utf8(get(&app(Some(unavailable_snapshot()), true), "/").await.2)
        .expect("utf-8 page");
    for gauge in ["cpu", "mem"] {
        let (tag, inner) = elements_with(&page, "data-gauge")
            .into_iter()
            .find(|(tag, _)| tag.contains(&format!(r#"data-gauge="{gauge}""#)))
            .expect("gauge");
        assert!(
            tag.contains(r#"data-state="unavailable""#),
            "the unavailable {gauge} gauge lacks its state: {tag}"
        );
        assert!(!inner.contains("g-fill"), "the {gauge} gauge drew a fill");
        assert!(inner.contains(">unavailable</text>"), "{inner}");
        // DEF-LAYOUT-94-SHORT-VIEW: the short-view HTML readout says the same.
        assert!(
            inner.contains(
                r#"<div class="g-read" aria-hidden="true"><span>unavailable</span></div>"#
            ),
            "the {gauge} gauge lacks its unavailable HTML readout"
        );
    }
    assert_eq!(
        page.matches(r#"data-state="unavailable""#).count(),
        3,
        "the CPU and memory gauges and the memory bar are the meters the empty tree leaves"
    );
    assert!(
        page.contains(r#"<span>swap</span><b>unavailable</b>"#),
        "the unavailable swap row does not say so"
    );
    // data-w="100%" is the reserved width of a percent cell, not a value
    // (ACC-LAYOUT-74-RESERVED-WIDTH), and the gauge scale's 100% label is a
    // scale (ACC-VIEW-171), not a value either.
    let page = page
        .replace(r#" data-w="100%""#, "")
        .replace(">100%</text>", "");
    assert!(
        !page.contains("0%"),
        "an unavailable value rendered as zero"
    );
    assert!(
        !page.contains("--value:0"),
        "an unavailable meter rendered a zero fill"
    );
}

/// DEF-VIEW-50-MEM-TERMS: one bar with a segment and a legend key per part,
/// in bar order, then one available line.
fn assert_mem_bar(html: &str, parts: &[(&str, &str)]) {
    let segments: Vec<&str> = html
        .match_indices(r#"<span class="mem-seg" data-seg=""#)
        .map(|(at, open)| {
            let name = &html[at + open.len()..];
            &name[..name.find('"').expect("quote")]
        })
        .collect();
    assert_eq!(
        segments,
        parts.iter().map(|(part, _)| *part).collect::<Vec<_>>(),
        "bar segments"
    );
    for (part, label) in parts {
        assert!(
            html.contains(&format!(
                r#"<span><i class="mem-key" data-seg="{part}"></i>{label}"#
            )),
            "legend lacks {part} labelled {label}"
        );
    }
    assert_eq!(html.matches(r#"class="mem-key""#).count(), parts.len());
    assert_eq!(html.matches("<span>free + reclaimable</span>").count(), 1);
    assert!(!html.contains("<span>available</span>"));
    assert!(!html.contains(">cached<"), "the cached row is still drawn");
}

/// DEF-VIEW-50-MEM-TERMS: on a host the bar is used | cache | free.
#[tokio::test]
async fn host_memory_bar_has_three_segments() {
    let snapshot = fixture_snapshot();
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    for html in page_and_event(&page, &fragments(&snapshot), "mem") {
        assert_mem_bar(
            html,
            &[("used", "used"), ("cache", "reclaimable"), ("free", "free")],
        );
    }
}

/// R23: with `show_swap` and `swap_disk` both off btop hides swap entirely,
/// so the memory rows draw no swap row and storage lists no mount; a swap key
/// that is on but unreadable keeps the row as unavailable.
#[tokio::test]
async fn swap_block_follows_the_swap_keys() {
    let hidden = Snapshot {
        mem: MemSnapshot {
            swap_enabled: false,
            ..MemSnapshot::default()
        },
        ..Snapshot::default()
    };
    let page = String::from_utf8(get(&app(Some(hidden), true), "/").await.2).expect("utf-8 page");
    assert!(
        !page.contains(r#"class="mem-row swap""#),
        "swap drawn with both keys off"
    );
    assert!(
        !page.contains(r#"class="mount""#),
        "storage lists a mount it was not given"
    );

    let unavailable = Snapshot {
        mem: MemSnapshot {
            swap_enabled: true,
            ..MemSnapshot::default()
        },
        ..Snapshot::default()
    };
    let page =
        String::from_utf8(get(&app(Some(unavailable), true), "/").await.2).expect("utf-8 page");
    assert!(
        page.contains(r#"<span>swap</span><b>unavailable</b>"#),
        "a requested swap is hidden or not marked unavailable"
    );
}

/// ACC-DESIGN-33-RENDER, ACC-VIEW-41-TOP-BAR and ACC-VIEW-40-GADGETS: the
/// page has the 05 panel set once each, the top bar carries the app status
/// and one CONFIG button, and the popover lists every panel theme.js knows.
#[tokio::test]
async fn page_renders_the_05_panel_set() {
    let page = String::from_utf8(get(&app(Some(fixture_snapshot()), true), "/").await.2)
        .expect("utf-8 page");
    let ids: Vec<&str> = PANELS.iter().map(|(id, _)| *id).collect();
    let attribute_values = |attribute: &str| -> Vec<&str> {
        let needle = format!(" {attribute}=\"");
        page.match_indices(&needle)
            .map(|(at, _)| {
                let start = at + needle.len();
                &page[start..start + page[start..].find('"').expect("value end")]
            })
            .collect()
    };
    assert_eq!(attribute_values("data-panel"), ids, "page panels");
    assert_eq!(attribute_values("data-panel-id"), ids, "popover rows");
    assert_eq!(
        page.matches("config-toggle").count(),
        1,
        "one CONFIG button"
    );

    let bar_start = page.find(r#"<header class="utility">"#).expect("top bar");
    let bar = &page[bar_start..bar_start + page[bar_start..].find("</header>").expect("bar end")];
    for marker in [
        "btop<span>-web</span>",
        env!("CARGO_PKG_VERSION"),
        ">250 ms<",
        r#"class="live-word up" aria-live="polite">live<"#,
        "READ-ONLY",
        "config-toggle",
    ] {
        assert!(bar.contains(marker), "top bar lacks {marker}");
    }
    // DEF-VIEW-55-CHIP-BLANK: the live chip reserves no width, so it hugs
    // its state word and sample age and shows no blank band.
    let chip = bar.find(r#"id="live-chip""#).expect("live chip");
    let chip = &bar[chip..chip + bar[chip..].find("</b></span>").expect("live chip end")];
    assert!(
        !chip.contains("data-w"),
        "the live chip reserves a width: {chip}"
    );
    let main = &page[page.find("<main").expect("main")..];
    for repeated in [">250 ms<", "live-word", "READ-ONLY", "hdr-clock"] {
        assert!(!main.contains(repeated), "a panel repeats {repeated}");
    }

    let theme_js =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/theme.js"))
            .expect("theme.js");
    for id in ids {
        assert!(
            theme_js.contains(&format!("\"{id}\": ")) || theme_js.contains(&format!(" {id}: ")),
            "theme.js has no band for {id}"
        );
    }
}

/// DEF-VIEW-55-CHIP-BLANK: the live chip is painted in the browser, so the
/// server markup, app.css and app.js must agree on the three state names.
/// The markup supplies the targets and the polite live word, the stylesheet
/// colours PAUSED and STALE, and the script derives the state, writes the
/// word, formats the STALE time and re-arms the stale timer at two intervals.
#[tokio::test]
async fn live_chip_carries_the_live_paused_and_stale_hooks() {
    let page = String::from_utf8(get(&app(Some(fixture_snapshot()), true), "/").await.2)
        .expect("utf-8 page");
    let at = page
        .find(r#"<span class="chip chip-live" id="live-chip""#)
        .expect("live chip");
    let chip = &page[at..at + page[at..].find("</b></span>").expect("chip end")];
    for marker in [
        r#"class="chip chip-live""#,
        r#"data-tip="Sampler state, live, paused or stale"#,
        r#"<i class="lamp"></i>"#,
        r#"<span class="live-word up" aria-live="polite">live</span>"#,
        r#"<b class="live-age" id="sample-age" aria-hidden="true">"#,
    ] {
        assert!(
            chip.contains(marker),
            "the live chip lacks {marker}: {chip}"
        );
    }
    let asset = |name: &str| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join(name),
        )
        .expect(name)
    };
    let css: String = asset("app.css").split_whitespace().collect();
    for rule in [
        ".chip-live.paused,.chip-live.stale{--chip-hue:var(--warn);border-color:var(--warn);}",
        ".chip-live.paused.lamp,.chip-live.stale.lamp{background:var(--warn);}",
    ] {
        assert!(css.contains(rule), "app.css lacks {rule}");
    }
    let js = asset("app.js");
    for hook in [
        r#"var state = stale ? "stale" : paused ? "paused" : "live";"#,
        r#"liveChip.classList.toggle("paused", state === "paused");"#,
        r#"liveChip.classList.toggle("stale", state === "stale");"#,
        r#"if (words[index].textContent !== state) words[index].textContent = state;"#,
        r#"sampleAge.textContent = "since " + new Date(lastFrame).toTimeString().slice(0, 8);"#,
        "}, 2 * updateMs);",
    ] {
        assert!(js.contains(hook), "app.js lacks {hook}");
    }
}

/// ACC-GRAPH-24-UNOBSCURED: no number, label or control sits inside a plot
/// box, on the page or in any SSE fragment; gauges are not plot boxes.
#[tokio::test]
async fn no_text_inside_chart_boxes() {
    let snapshot = fixture_snapshot();
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let mut documents = vec![("page", page)];
    documents.extend(fragments(&snapshot));
    for (name, html) in &documents {
        let boxes = elements_with(html, "data-chart");
        assert!(
            *name == "proc" || !boxes.is_empty(),
            "{name} has no plot box"
        );
        for (tag, inner) in boxes {
            let mut text = String::new();
            let mut rest = inner;
            while let Some(open) = rest.find('<') {
                text.push_str(&rest[..open]);
                let close = open + rest[open..].find('>').expect("tag end");
                let element = rest[open + 1..close].trim_start_matches('/');
                let element = element.split([' ', '/']).next().unwrap_or_default();
                assert!(
                    ["svg", "path", "g"].contains(&element),
                    "{name}: <{element}> inside {tag}"
                );
                rest = &rest[close + 1..];
            }
            text.push_str(rest);
            assert!(text.trim().is_empty(), "{name}: text {text:?} inside {tag}");
        }
        for (tag, _) in elements_with(html, "data-gauge") {
            assert!(!tag.contains("data-chart"), "{name}: a gauge is a plot box");
        }
    }
    assert!(
        documents[0].1.matches("data-gauge=").count() == 2,
        "the page has two gauges"
    );
    // ACC-GRAPH-68-PLOT-GRID: the grid is the boxes' background, not a path.
    for (name, html) in &documents {
        assert!(!html.contains(r#"<path class="grid""#), "{name}: grid path");
    }
}

/// ACC-VIEW-80-STAT-TIPS: every stat carries its tooltip text from the
/// server: at most 4 short lines, each text naming a unit and a source; no
/// plot box carries one; one shared tip element serves the page.
#[tokio::test]
async fn stat_tips_are_short_and_name_unit_and_source() {
    let snapshot = fixture_snapshot();
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let mut documents = vec![("page", page)];
    documents.extend(fragments(&snapshot));
    // The container tooltips add the scope lines; they hold the same budget.
    let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/container/proc");
    let mut config = Config::load(None).expect("config");
    config.container = btop_web::collector::container::detect(&tree);
    let container = recorded_snapshot_with("tests/fixtures/container/proc", 1, config);
    let page = String::from_utf8(get(&app(Some(container.clone()), true), "/").await.2)
        .expect("utf-8 page");
    documents.push(("container page", page));
    // This tree records no GPU, so the GPU fragment carries no tooltip.
    documents.extend(
        fragments(&container)
            .into_iter()
            .filter(|(name, _)| *name == "mem"),
    );
    for (name, html) in &documents {
        let tips = elements_with(html, "data-tip");
        assert!(!tips.is_empty(), "{name} has no stat tooltip");
        for (tag, _) in tips {
            assert!(
                !tag.contains("data-chart"),
                "{name}: a plot carries a tooltip"
            );
            // DEF-VIEW-74: a browser title beside the house box is a second
            // box.
            assert!(
                !tag.contains(" title=\""),
                "{name}: a tooltip carries a browser title: {tag}"
            );
            let value = &tag[tag.find("data-tip=\"").expect("tip") + 10..];
            let value = &value[..value.find('"').expect("tip end")];
            // A template writes its line breaks as &#10;; a tip built in Rust
            // carries them as newlines.
            let lines: Vec<&str> = value
                .split("&#10;")
                .flat_map(|line| line.split('\n'))
                .collect();
            assert!(
                (1..=4).contains(&lines.len()),
                "{name}: {} lines: {value}",
                lines.len()
            );
            let lower = value.to_lowercase();
            assert!(lower.contains("unit"), "{name}: no unit: {value}");
            assert!(lower.contains("source:"), "{name}: no source: {value}");
            for line in lines {
                let text = line.replace("&lt;", "<").replace("&gt;", ">");
                assert!(!text.is_empty(), "{name}: empty line: {value}");
                assert!(text.chars().count() <= 48, "{name}: long line: {text}");
            }
        }
    }
    let page = &documents[0].1;
    let all = page.match_indices("data-tip=\"").count();
    assert!(all >= 30, "the page carries only {all} stat tooltips");
    assert!(
        page.contains(r#"<div class="tip" id="tip" role="tooltip" hidden></div>"#),
        "the page lacks the shared tooltip element"
    );
}

/// ACC-VIEW-80-STAT-TIPS: the anchors batch 3 gave a tooltip each still carry
/// one: the top-bar chips, every stat tile and gauge, the memory bar and its
/// rows, the network header, interface and legend, the GPU header, legend
/// value and bars, every core row and the four numeric process column
/// headers. The tip text itself is held by
/// `stat_tips_are_short_and_name_unit_and_source`; this test holds presence,
/// which a dropped attribute would otherwise take with it.
#[tokio::test]
async fn stat_tips_sit_on_every_tip_anchor() {
    let page = String::from_utf8(get(&app(Some(fixture_snapshot()), true), "/").await.2)
        .expect("utf-8 page");
    for anchor in [
        r#"id="tier-chip" data-tip="Priority tiers on screen"#,
        r#"id="live-chip" data-tip="Sampler state"#,
        r#"data-tip="Copy telemetry as JSON"#,
        r#"data-tip="Server mode, no unit"#,
        r#"class="stat-tile" data-tip="CPU total"#,
        r#"class="stat-tile" data-tip="Memory used"#,
        r#"class="stat-tile" data-tip="Process total"#,
        r#"data-gauge="cpu" data-tip="CPU total"#,
        r#"data-gauge="mem" data-tip="Memory shares"#,
        r#"class="mem-bar" data-tip="Memory bar"#,
        r#"class="mem-row" data-tip="Memory used"#,
        r#"class="mem-row" data-tip="Reclaimable cache"#,
        r#"data-field="net-interface" data-tip="Selected interface"#,
        r#"data-field="net-axis-note" data-tip="Plot scale"#,
        r#"class="legend-item" data-tip="Download rate"#,
        r#"class="legend-item" data-tip="Upload rate"#,
        r#"data-field="gpu-count" data-tip="GPU count"#,
        r#"class="legend-value" data-tip="GPU 0 utilisation"#,
        r#"class="gpu-bars" data-tip="GPU 0 bars"#,
        r#"class="core-row" data-tip="Core cpu0"#,
        r#"id="proc-count" data-tip="Shown / total processes"#,
        r#"<th class="num" scope="col" data-tip="Pid"#,
        r#"data-field="proc-threads" scope="col" data-tip="Threads"#,
        r#"<th class="num" scope="col" data-tip="Resident memory"#,
        r#"<th class="num" scope="col" data-tip="CPU use of the process"#,
    ] {
        assert!(page.contains(anchor), "no tooltip on {anchor}");
    }
    // ACC-VIEW-111: the sampling interval in the top bar is a terse stat too.
    let at = page
        .find(r#"id="update-interval""#)
        .expect("sampling interval");
    let span = &page[at..at + page[at..].find("</span>").expect("span end")];
    assert!(
        span.contains(r#"data-tip="Sampling interval"#),
        "the sampling interval carries no tooltip: {span}"
    );
}

/// ACC-GRAPH-67-SMOOTH-SCALE: each network plot draws its area and line in
/// one series group, which a scale change transforms.
#[tokio::test]
async fn network_plots_draw_in_a_series_group() {
    let snapshot = fixture_snapshot();
    assert!(!snapshot.net.down_chart.line.is_empty());
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    // ACC-GRAPH-164-NET-SCALE: the axis label prints each direction's shown
    // ceiling with its unit.
    let down = btop_web::collector::axis_rate(snapshot.net.scale_down.label_max);
    let up = btop_web::collector::axis_rate(snapshot.net.scale_up.label_max);
    assert!(
        page.contains(&format!(
            "scale ▼ <span data-w=\"9.99 MiB/s\">{down}</span> ▲ <span data-w=\"9.99 MiB/s\">{up}</span>"
        )),
        "the axis label does not print the shown ceilings"
    );
    let events = fragments(&snapshot);
    for html in page_and_event(&page, &events, "net") {
        let plots: Vec<_> = elements_with(html, "data-chart")
            .into_iter()
            .filter(|(tag, _)| tag.contains(r#"data-chart="net-"#))
            .collect();
        assert_eq!(plots.len(), 2);
        for (tag, inner) in plots {
            assert!(
                inner.contains(r#"<g class="series"><path class="area""#)
                    && inner.trim_end().ends_with("</path></g></svg>"),
                "{tag}: {inner}"
            );
        }
    }
}

/// DEF-GRAPH-61-NET-LANES: the network panel stacks the download lane above
/// the upload lane with one visible boundary between them, and both lanes
/// share one time axis.
#[tokio::test]
async fn network_panel_draws_two_lanes_with_a_boundary() {
    let snapshot = fixture_snapshot();
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let events = fragments(&snapshot);
    for html in page_and_event(&page, &events, "net") {
        let down = html
            .find(r#"data-chart="net-down""#)
            .expect("download lane");
        let boundary = html.find(r#"class="net-boundary""#).expect("lane boundary");
        let up = html.find(r#"data-chart="net-up""#).expect("upload lane");
        assert!(
            down < boundary,
            "the boundary is not after the download lane"
        );
        assert!(boundary < up, "the boundary is not before the upload lane");
        assert_eq!(
            html.matches(r#"class="net-boundary""#).count(),
            1,
            "one boundary between the two lanes"
        );
        assert_eq!(
            html.matches(r#"data-field="net-x-axis""#).count(),
            1,
            "one shared time axis"
        );
    }
}

/// ACC-VIEW-128-TICK-LABELS: each sparkline carries faint seconds labels
/// under its interior vertical ticks, the shared axis's wording, and `now` at
/// the right edge; the labels follow the configured window.
#[tokio::test]
async fn sparklines_carry_seconds_axis_labels() {
    let snapshot = fixture_snapshot();
    assert_eq!(
        snapshot.axis_labels(),
        ["180 s", "135 s", "90 s", "45 s"],
        "the shared axis keeps the left-edge label"
    );
    let labels: Vec<String> = snapshot.tick_labels();
    assert_eq!(labels, ["135 s", "90 s", "45 s"], "one per interior tick");
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let panel = |id: &str| -> String {
        elements_with(&page, "data-panel")
            .into_iter()
            .find(|(tag, _)| tag.contains(&format!(r#"id="{id}""#)))
            .map(|(_, inner)| inner.to_string())
            .expect(id)
    };
    let markup = format!(
        r#"<div class="xaxis stat-axis"{} aria-hidden="true"><span>{}</span><span>{}</span><span>{}</span><span>now</span></div>"#,
        r#" data-p="3" data-field="{field}""#, labels[0], labels[1], labels[2]
    );
    for (id, field) in [
        ("p-cpu-stat", "cpu-stat-axis"),
        ("p-mem-stat", "mem-stat-axis"),
    ] {
        let expected = markup.replace("{field}", field);
        assert!(panel(id).contains(&expected), "{id}: {expected}");
    }
    // The Processes tile draws no P3 field of its own: its panel is the field.
    let bare = markup.replace(r#" data-p="3" data-field="{field}""#, "");
    assert!(panel("p-proc-stat").contains(&bare), "p-proc-stat: {bare}");
    // The window row offers the five windows as house chips.
    let page =
        String::from_utf8(get(&app(Some(snapshot), false), "/").await.2).expect("utf-8 page");
    let row = elements_with(&page, "data-key")
        .into_iter()
        .find(|(tag, _)| tag.contains(r#"data-key="window_s""#))
        .map(|(_, inner)| inner.to_string())
        .expect("the time axis row");
    for (value, label) in [
        ("60", "1m"),
        ("120", "2m"),
        ("180", "3m"),
        ("300", "5m"),
        ("600", "10m"),
    ] {
        assert!(
            row.contains(&format!(r#"name="value" value="{value}""#)),
            "window_s lacks {value}"
        );
        assert!(
            row.contains(&format!(">{label}<")),
            "window_s lacks {label}"
        );
    }
}

/// ACC-VIEW-174-NET-STRIP: the legend ends in the link line, with a fault chip
/// only for a count above zero. DEF-GRAPH-102-NET-SCALE: no rate in bits.
#[tokio::test]
async fn network_legend_states_the_link_and_no_rate_in_bits() {
    let mut snapshot = fixture_snapshot();
    let page = |snapshot: &Snapshot| {
        let html = fragments(snapshot);
        html.iter()
            .find(|(name, _)| *name == "net")
            .expect("net fragment")
            .1
            .clone()
    };
    let clean = page(&snapshot);
    assert!(clean.contains(r#"<span class="legend-meta">link 10 Gb/s · full · up</span></div>"#));
    assert!(!clean.contains("Mibps"), "a rate in bits");
    snapshot.net.link.errors = Some(3);
    snapshot.net.link.drops = Some(9);
    assert!(page(&snapshot).contains(
        r#"up</span><span class="chip up">err 3</span><span class="chip up">drop 9</span></div>"#
    ));
}

/// DEF-GRAPH-63-GRID-UNITS: every plot labels its grid lines with their value
/// and unit and states the total scale at its top corner, in a gutter outside
/// the [data-chart] box (ACC-GRAPH-24): percent levels for the GPU lanes, the
/// shown ceiling, its half and zero for the network lanes.
#[tokio::test]
async fn plots_label_their_grid_lines_and_total_scale() {
    let snapshot = fixture_snapshot();
    let down = snapshot.net.down_scale_labels();
    let up = snapshot.net.up_scale_labels();
    // DEF-GRAPH-102-NET-SCALE: every rendered label names bytes per second.
    for label in down.iter().chain(up.iter()) {
        assert!(label.ends_with("B/s"), "{label} has no unit");
    }
    let gutter = |ceiling: &str, half: &str| {
        format!(
            r#"<div class="plot-scale" aria-hidden="true"><span class="scale-total">{ceiling}</span><span class="scale-mid" style="--at:50">{half}</span><span class="scale-zero">0 B/s</span></div>"#
        )
    };
    let (down_gutter, up_gutter) = (gutter(&down[0], &down[1]), gutter(&up[0], &up[1]));
    let gpu_gutter = r#"<div class="plot-scale" aria-hidden="true"><span class="scale-total">100%</span><span class="scale-mid" style="--at:75">75%</span><span class="scale-mid" style="--at:50">50%</span><span class="scale-mid" style="--at:25">25%</span><span class="scale-zero">0%</span></div>"#;
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let events = fragments(&snapshot);
    let [_, gpu_event] = page_and_event(&page, &events, "gpu");
    let [_, net_event] = page_and_event(&page, &events, "net");
    for (name, html) in [("page", page.as_str()), ("gpu event", gpu_event)] {
        let lanes = elements_with(html, "data-chart")
            .into_iter()
            .filter(|(tag, _)| tag.contains(r#"data-chart="gpu""#))
            .count();
        assert!(lanes > 0, "{name} has no GPU lane");
        assert_eq!(
            html.matches(gpu_gutter).count(),
            lanes,
            "{name}: one scale gutter per GPU lane"
        );
        assert!(
            html.find(gpu_gutter).expect("GPU gutter")
                < html.find(r#"data-chart="gpu""#).expect("GPU plot"),
            "{name}: the GPU gutter is not before its plot"
        );
    }
    for (name, html) in [("page", page.as_str()), ("net event", net_event)] {
        let down_at = html.find(r#"data-chart="net-down""#).expect("down plot");
        let up_at = html.find(r#"data-chart="net-up""#).expect("up plot");
        assert_eq!(
            html[..down_at].matches(&down_gutter).count(),
            1,
            "{name}: one download gutter before its plot"
        );
        // The two directions can share a ceiling in a quiet fixture, so the
        // segments, not the whole-document counts, place each gutter.
        assert_eq!(
            html[down_at..up_at].matches(&up_gutter).count(),
            1,
            "{name}: one upload gutter between the two plots"
        );
    }
    // No counter, no ceiling: the gutter carries no text, so the word
    // "unavailable" never reaches the plot (ACC-GRAPH-24).
    let empty = String::from_utf8(get(&app(Some(unavailable_snapshot()), true), "/").await.2)
        .expect("utf-8 page");
    assert_eq!(
        empty
            .matches(r#"<span class="scale-total"></span>"#)
            .count(),
        2,
        "the empty network scale prints no word"
    );
}

/// DEF-TEST-53-HOST-IFACE: the page names the interface the recorded tree
/// lists. The tree holds no eth0, so an interface of the machine that runs
/// the test must not reach the page.
#[tokio::test]
async fn page_names_the_interface_the_recorded_tree_lists() {
    let snapshot = recorded_snapshot("tests/fixtures/net/host-names/proc", 1);
    let page = String::from_utf8(get(&app(Some(snapshot), true), "/").await.2).expect("utf-8 page");
    assert!(
        page.contains(">enp0s3<"),
        "the tree's interface is not named"
    );
    assert!(!page.contains("eth0"), "a host interface reached the page");
}

/// The page and the proc fragment of `snapshot`.
async fn proc_documents(snapshot: &Snapshot) -> [String; 2] {
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let (_, fragment) = fragments(snapshot)
        .into_iter()
        .find(|(name, _)| *name == "proc")
        .expect("proc fragment");
    [page, fragment]
}

/// The value of `name="..."` in an opening tag, `None` without one.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    Some(&tag[start..start + tag[start..].find('"')?])
}

/// ACC-VIEW-52-PROC-GRAPH: a row's graph sits in its Cpu% cell before the
/// number; a row without one carries no svg, and the header stays one Cpu%.
/// ACC-VIEW-58-PROC-VIEW: a parent's summed graph follows its own one.
#[tokio::test]
async fn process_rows_draw_a_graph_left_of_cpu() {
    let mut snapshot = fixture_snapshot();
    let parent = snapshot
        .proc
        .entries
        .iter_mut()
        .find_map(|entry| entry.sum.as_mut())
        .expect("a parent row");
    parent.graph = ChartPath::new(
        &[0.0, 9.0],
        0,
        80,
        PROC_GRAPH.width,
        PROC_GRAPH.height,
        0.0,
        100.0,
    );
    parent.graph_tone = "cpu-ok";
    let entries = &snapshot.proc.entries;
    for html in proc_documents(&snapshot).await {
        assert_eq!(html.matches(">CPU%</button></th>").count(), 1);
        let rows: Vec<&str> = html.split("<tr").skip(2).collect();
        assert_eq!(rows.len(), entries.len());
        let (mut own, mut summed) = (0, 0);
        for (row, entry) in rows.iter().zip(entries) {
            let cpu_cell = &row[row.rfind("<td").expect("Cpu% cell")..];
            let has_own = !entry.graph.line.is_empty();
            let has_sum = entry
                .sum
                .as_ref()
                .is_some_and(|sum| !sum.graph.line.is_empty());
            assert_eq!(
                cpu_cell.contains(r#"<svg class="cpu-ok" data-chart="proc" viewBox="0 0 10 12""#),
                has_own
            );
            assert_eq!(
                cpu_cell
                    .contains(r#"<svg class="sum cpu-ok" data-chart="proc" viewBox="0 0 10 12""#),
                has_sum
            );
            if has_own || has_sum {
                let after = &cpu_cell[cpu_cell.rfind("</svg>").expect("svg end") + 6..];
                assert!(after.starts_with(&entry.cpu_label()), "{cpu_cell}");
            } else {
                assert!(!cpu_cell.contains("<svg"));
            }
            own += usize::from(has_own);
            summed += usize::from(has_sum);
        }
        assert_eq!((own, summed), (1, 1));
        assert_eq!(html.matches("data-chart=\"proc\"").count(), 2);
    }
}

/// ACC-VIEW-59-PROC-SORT: every column header is a sort button.
#[tokio::test]
async fn process_headers_are_sort_buttons() {
    for html in proc_documents(&fixture_snapshot()).await {
        let start = html.find("<thead>").expect("thead");
        let head = &html[start..start + html[start..].find("</thead>").expect("thead end")];
        let cells: Vec<&str> = head.split("<th ").skip(1).collect();
        assert_eq!(cells.len(), 7);
        for (cell, name) in cells.iter().zip([
            "PID", "PROGRAM", "COMMAND", "THREADS", "USER", "MEM", "CPU%",
        ]) {
            assert_eq!(cell.matches("<button").count(), 1, "{cell}");
            assert!(
                cell.contains(&format!(r#"<button type="button">{name}</button></th>"#)),
                "{cell}"
            );
        }
    }
}

/// ACC-VIEW-58-PROC-VIEW: each row comes on one line with its depth and raw
/// values (`-` when unavailable, never 0); rows with children also carry
/// their subtree labels.
#[tokio::test]
async fn process_rows_carry_tree_data() {
    let snapshot = fixture_snapshot();
    let entries = &snapshot.proc.entries;
    assert!(entries.iter().any(|entry| entry.sum.is_some()));
    assert!(
        entries.iter().any(|entry| entry.mem_bytes.is_none()),
        "pid 333 has no stat"
    );
    for html in proc_documents(&snapshot).await {
        let rows: Vec<&str> = html
            .lines()
            .filter(|line| line.starts_with("<tr "))
            .collect();
        assert_eq!(html.matches("<tr ").count(), entries.len());
        assert_eq!(rows.len(), entries.len());
        for (row, entry) in rows.iter().zip(entries) {
            assert!(row.ends_with("</tr>"), "{row}");
            let raw: Vec<&str> = attribute(row, "data-raw")
                .expect("data-raw")
                .split(' ')
                .collect();
            let token = |value: Option<String>| value.unwrap_or_else(|| "-".to_string());
            assert_eq!(
                raw,
                [
                    entry.depth.to_string(),
                    token(entry.cpu_pct.map(|value| value.to_string())),
                    token(entry.mem_bytes.map(|value| value.to_string())),
                    token(entry.threads.map(|value| value.to_string())),
                ],
                "{row}"
            );
            match attribute(row, "data-sum") {
                Some(sum) => {
                    assert!(entry.sum.is_some());
                    assert_eq!(sum.split(' ').count(), 4, "{row}");
                }
                None => assert!(entry.sum.is_none(), "{row}"),
            }
        }
    }
}

/// ACC-VIEW-180-PROC-TREE, DEF-VIEW-103-CHEVRON, DEF-VIEW-105-TOGGLE-GONE:
/// the tree is drawn as elements at one fixed px step, not typed as box
/// glyphs, so its alignment does not depend on the current face; the Program
/// column's reserved width reads that step and the toggle is a filled-triangle
/// button carrying aria-expanded, whose path points right when collapsed and
/// down when expanded, so no transform can clip it.
#[tokio::test]
async fn process_tree_draws_rules_at_a_fixed_px_step() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let css = std::fs::read_to_string(base.join("assets/app.css")).expect("app.css");
    let js = std::fs::read_to_string(base.join("assets/app.js")).expect("app.js");
    assert!(
        css.contains("--tree-step: 12px;"),
        "the tree step is not one 12 px value"
    );
    assert_eq!(
        css.matches("(var(--tree-level, 3) + 2) * var(--tree-step)")
            .count(),
        2,
        "a reserved width does not read the px step"
    );
    for line in [
        "border-left: 1px solid currentColor",
        "border-top: 1px solid currentColor",
    ] {
        assert!(
            css.contains(line),
            "a connector is not a 1 px border: {line}"
        );
    }
    for glyph in [
        "\\u2502", "\\u251c", "\\u2514", "\\u250c", "\\u25be", "\\u25b8",
    ] {
        assert!(!js.contains(glyph), "app.js still types the glyph {glyph}");
    }
    assert!(js.contains("tstep tbranch"), "app.js builds no branch step");
    assert!(
        js.contains("aria-expanded"),
        "the toggle carries no aria-expanded"
    );
    assert!(
        js.contains(r#"svg.setAttribute("viewBox", "0 0 4 5")"#)
            && js.contains(r#"path.setAttribute("d", "M0 0L4 2.5L0 5Z")"#),
        "the toggle icon is not a closed 4 by 5 triangle"
    );
    let rule = |selector: &str| {
        css.split(selector)
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .unwrap_or_default()
    };
    let icon = rule(".ptable .twisty svg path {");
    assert!(
        icon.contains("fill: currentColor") && !icon.contains("stroke"),
        "the toggle icon is not a filled path in the cell's text colour: {icon}"
    );
    let size = rule(".ptable .twisty svg {");
    assert!(
        size.contains("width: 4px") && size.contains("height: 5px"),
        "the triangle is not 4 by 5 px: {size}"
    );
    let turned = rule(r#".ptable .twisty[aria-expanded="true"] svg {"#);
    assert!(
        turned.is_empty() && !css.contains("rotate(90deg)"),
        "the toggle still turns by a transform, which clips the triangle at the svg's origin: {turned}"
    );
    assert!(
        !css.contains(r#".ptable tr[data-collapsed="deep"] svg {"#)
            && css.contains(r#".ptable tr[data-collapsed="deep"] svg[data-chart="proc"] {"#),
        "the deep collapsed row's graph rule hides every svg in the row, the toggle's icon included"
    );
    assert!(
        js.contains(
            r#"p.twisty.firstChild.firstChild.setAttribute("d", p.collapsed ? "M0 0L4 2.5L0 5Z" : "M0 0L4 0L2 5Z")"#
        ),
        "the toggle path does not swap between a right and a down triangle with aria-expanded"
    );
}

/// ACC-VIEW-198-ROW-HOVER: the hovered row paints the row-hover background and
/// nothing else, and a hovered cell the row-hover token of its own colour.
#[test]
fn hovered_row_cells_paint_the_row_hover_tokens() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let css = std::fs::read_to_string(base.join("assets/app.css")).expect("app.css");
    let rule = |selector: &str| {
        css.split(selector)
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .unwrap_or_default()
    };
    let layers = rule(".ptable tbody tr:hover,\n.ptable tbody tr:hover td {");
    assert_eq!(
        layers.trim(),
        "background: var(--row-hover);",
        "the hovered row and its cells do not paint the background alone"
    );
    for (selector, token) in [
        (".ptable tbody tr:hover {", "--row-hover-text"),
        (".ptable tbody tr:hover .cpu-ok {", "--row-hover-ok"),
        (".ptable tbody tr:hover .cpu-warn {", "--row-hover-warn"),
        (".ptable tbody tr:hover .cpu-crit {", "--row-hover-crit"),
        (".ptable tbody tr:hover .cpu-zero {", "--row-hover-muted"),
    ] {
        assert!(
            rule(selector).contains(&format!("color: var({token});")),
            "{selector} does not paint {token}"
        );
    }
}

/// ACC-CONF-63-TITLE-FLAG: the page title is btop-web unless set, and a set
/// title is HTML-escaped.
#[tokio::test]
async fn title_flag_text_is_escaped() {
    let titled = {
        let (_sender, receiver) = watch::channel(Some(Arc::new(Frame::new(Snapshot::default()))));
        let mut state = AppState::new(receiver, true);
        state.title = r#"lab <1> & "x""#.into();
        btop_web::app(state)
    };
    let default = String::from_utf8(get(&app(Some(Snapshot::default()), true), "/").await.2)
        .expect("utf-8 page");
    assert!(default.contains("<title>btop-web</title>"));
    let page = String::from_utf8(get(&titled, "/").await.2).expect("utf-8 page");
    let start = page.find("<title>").expect("title") + 7;
    let title = &page[start..start + page[start..].find("</title>").expect("title end")];
    assert_eq!(title, "lab &#60;1&#62; &#38; &#34;x&#34;");
}

/// ACC-CONF-60-VIEW-FLAG: the Processes panel carries the starting view for
/// a viewer with no stored choice.
#[tokio::test]
async fn page_carries_the_starting_proc_view() {
    let page = |view: &'static str| {
        let (_sender, receiver) = watch::channel(Some(Arc::new(Frame::new(Snapshot::default()))));
        let mut state = AppState::new(receiver, true);
        state.proc_view_flag = Some(view);
        btop_web::app(state)
    };
    for (app, view) in [
        (app(Some(Snapshot::default()), true), "top"),
        (page("tree"), "tree"),
    ] {
        let html = String::from_utf8(get(&app, "/").await.2).expect("utf-8 page");
        let (tag, _) = elements_with(&html, "data-proc-view")
            .into_iter()
            .next()
            .expect("the Processes panel");
        assert!(tag.contains(r#"id="p-proc""#), "{tag}");
        assert_eq!(attribute(tag, "data-proc-view"), Some(view));
    }
}

/// ACC-VIEW-58-PROC-VIEW, ACC-VIEW-103-TREE-DEPTH: the config menu offers the
/// three views and the tree display level as house-chip radios.
#[tokio::test]
async fn config_offers_process_view_and_tree_level() {
    let page = String::from_utf8(get(&app(Some(Snapshot::default()), true), "/").await.2)
        .expect("utf-8 page");
    let config = &page[page.find(r#"id="config""#).expect("config popover")..];
    for (group, values) in [
        ("proc-view", &["top", "all", "tree"][..]),
        (
            "proc-level",
            &["1", "2", "3", "4", "5", "6", "7", "8", "9"][..],
        ),
    ] {
        let (tag, inner) = elements_with(config, "data-group")
            .into_iter()
            .find(|(tag, _)| attribute(tag, "data-group") == Some(group))
            .expect("group");
        assert_eq!(attribute(tag, "role"), Some("radiogroup"));
        let chips: Vec<&str> = inner.matches("<button").collect();
        assert_eq!(chips.len(), values.len(), "{group}");
        for value in values {
            assert!(
                inner.contains(&format!(
                    r#"<button class="opt" type="button" role="radio" aria-checked="false" data-value="{value}""#
                )),
                "{group}: {value}"
            );
        }
    }
}

/// ACC-GRAPH-43-GPU-LOAD, DEF-GRAPH-51-GPU-LANES: the GPU panel holds one
/// lane per device, each its own plot box with one series beside its legend
/// entry, and one shared axis with the network plot's labels. A device
/// without a utilization reading draws no line and reads unavailable, never
/// zero.
#[tokio::test]
async fn gpu_panel_draws_one_lane_per_device() {
    let mut snapshot = recorded_snapshot("tests/fixtures/proc", 2);
    let lane = gpu_lane(3);
    let chart = || ChartPath::new(&[12.0, 37.0], 0, 960, lane.width, lane.height, 0.0, 100.0);
    snapshot.gpu.devices = vec![
        GpuDevice {
            name: "RTX PRO 4000 Blackwell".to_string(),
            utilization_pct: Some(37.0),
            vram_used: Some(452 * 1024 * 1024),
            vram_total: Some(24 * 1024 * 1024 * 1024),
            temperature_c: Some(41.0),
            power_w: Some(3.0),
            chart: chart(),
            ..GpuDevice::default()
        },
        GpuDevice {
            name: "RTX PRO 6000 Blackwell Max-Q Workstation Edition".to_string(),
            utilization_pct: Some(12.0),
            power_w: Some(150.0),
            power_max_w: Some(200.0),
            chart: chart(),
            ..GpuDevice::default()
        },
        GpuDevice {
            name: "RTX PRO 5000 Blackwell".to_string(),
            ..GpuDevice::default()
        },
    ];
    let first = snapshot.gpu.devices[0].clone();
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let fragment = fragments(&snapshot)
        .into_iter()
        .find(|(name, _)| *name == "gpu")
        .expect("gpu fragment")
        .1;
    // The page's panels carry data-panel and the fragment's wrapper carries
    // hx-swap-oob; a plain id scan would reach the void <input id="filter">.
    let panel = |html: &str, attribute: &str, id: &str| -> String {
        elements_with(html, attribute)
            .into_iter()
            .find(|(tag, _)| tag.contains(&format!(r#"id="{id}""#)))
            .unwrap_or_else(|| panic!("{id} missing"))
            .1
            .to_string()
    };
    // The labels after the opening tag; the two axes differ by data-field.
    let xaxis = |inner: &str| -> String {
        let start = inner.find(r#"<div class="xaxis""#).expect("x axis");
        let labels = start + inner[start..].find('>').expect("x axis tag end") + 1;
        inner[labels..labels + inner[labels..].find("</div>").expect("x axis end")].to_string()
    };
    let net_axis = xaxis(&panel(&page, "data-panel", "p-net"));
    for (name, html, attribute) in [
        ("page", &page, "data-panel"),
        ("gpu fragment", &fragment, "hx-swap-oob"),
    ] {
        let gpu = panel(html, attribute, "p-gpu");
        assert_eq!(
            gpu.matches("data-chart=").count(),
            3,
            "{name}: one plot per device"
        );
        let lanes: Vec<(&str, &str)> = elements_with(&gpu, "data-gpu")
            .into_iter()
            .filter(|(tag, _)| tag.starts_with(r#"<div class="gpu-lane""#))
            .collect();
        let entries: Vec<(&str, &str)> = elements_with(&gpu, "data-gpu")
            .into_iter()
            .filter(|(tag, _)| tag.starts_with(r#"<div class="legend-item""#))
            .collect();
        assert_eq!(lanes.len(), 3, "{name}: one lane per device");
        assert_eq!(
            gpu.matches(r#"class="gpu-boundary""#).count(),
            lanes.len() - 1,
            "{name}: one lane boundary per gap between {} lanes",
            lanes.len()
        );
        assert_eq!(entries.len(), 3, "{name}: one legend entry per device");
        let view_box = format!(r#"viewBox="0 0 {} {}""#, lane.width, lane.height);
        for (index, (tag, inner)) in lanes.iter().enumerate() {
            assert!(
                tag.contains(&format!(r#"data-gpu="{index}""#)),
                "{name}: lane {index}"
            );
            let plots = elements_with(inner, "data-chart");
            assert_eq!(plots.len(), 1, "{name}: lane {index} has one plot box");
            assert!(
                plots[0].1.contains(&view_box),
                "{name}: lane {index} {view_box}"
            );
            assert_eq!(
                plots[0].1.matches("<g ").count(),
                1,
                "{name}: lane {index} series"
            );
            assert!(
                plots[0]
                    .1
                    .contains(&format!(r#"class="gpu-series g{index}""#)),
                "{name}: lane {index} series colour"
            );
            assert!(
                inner.contains(&format!(r#"class="legend-item" data-gpu="{index}""#))
                    && inner.contains(&format!("sw-gpu-{index}")),
                "{name}: lane {index} holds its legend entry"
            );
            // ACC-VIEW-82-GPU-TAB: the edge tab is the entry's first markup
            // and carries the series colour class the CSS strip paints.
            let entry = entries[index].1;
            let tab = format!(r#"<i class="sw sw-gpu-{}"></i>"#, index % 3);
            let tab_at = entry
                .find(&tab)
                .unwrap_or_else(|| panic!("{name}: GPU {index} has no {tab}: {entry}"));
            assert!(
                entry[..tab_at].trim().is_empty()
                    && entry[tab_at + tab.len()..]
                        .trim_start()
                        .starts_with(r#"<span class="legend-name">"#),
                "{name}: GPU {index} edge tab is not the entry's first markup: {entry}"
            );
        }
        assert!(lanes[0].1.contains("<path"), "{name}: GPU 0 draws a line");
        assert!(!lanes[2].1.contains("<path"), "{name}: GPU 2 drew a line");
        assert!(
            entries[2].1.contains("unavailable") && !entries[2].1.contains("0%"),
            "{name}: GPU 2 entry {}",
            entries[2].1
        );
        for marker in [
            "GPU 0".to_string(),
            format!(r#"title="{}""#, first.name),
            // DEF-VIEW-46-LEGEND-SHIFT: the load and the temperature reserve
            // their widest forms, so the VRAM and the power keep their place.
            r#"<span data-w="100%">37%</span><span class="lg-vram">"#.to_string(),
            first.vram_label(),
            format!(
                r#"<span data-w="100°C">{}</span> &middot; "#,
                first.temperature_label()
            ),
            first.power_label(),
        ] {
            assert!(
                entries[0].1.contains(&marker),
                "{name}: GPU 0 entry lacks {marker}: {}",
                entries[0].1
            );
        }
        // ACC-VIEW-79-GPU-BARS: LOAD, MEM and PWR bars in the core meter
        // style; a bar with no reading renders its unavailable state.
        assert!(
            entries[0].1.contains(
                r#"<i class="gbl">load</i><span class="meter" style="--value:37"></span><i class="gbl">mem</i><span class="meter" style="--value:2"></span><i class="gbl">pwr</i><span class="meter" data-state="unavailable"></span>"#
            ),
            "{name}: GPU 0 bars {}",
            entries[0].1
        );
        assert!(
            entries[1].1.contains(
                r#"<i class="gbl">load</i><span class="meter" style="--value:12"></span><i class="gbl">mem</i><span class="meter" data-state="unavailable"></span><i class="gbl">pwr</i><span class="meter" style="--value:75"></span>"#
            ),
            "{name}: GPU 1 bars {}",
            entries[1].1
        );
        assert_eq!(
            entries[2]
                .1
                .matches(r#"<span class="meter" data-state="unavailable"></span>"#)
                .count(),
            3,
            "{name}: GPU 2 bars render unavailable, never zero: {}",
            entries[2].1
        );
        assert!(
            !entries[2].1.contains("--value"),
            "{name}: GPU 2 bars carry no value: {}",
            entries[2].1
        );
        assert!(
            !gpu.contains("gpu-spark") && !gpu.contains(r#"class="gpu""#),
            "{name}: GPU cards markup remains"
        );
        assert_eq!(
            gpu.matches(r#"class="xaxis""#).count(),
            1,
            "{name}: one shared axis"
        );
        assert_eq!(
            xaxis(&gpu),
            net_axis,
            "{name}: the GPU plot has its own axis"
        );
    }
}

/// The page and the event fragment `name`, the two places a panel renders.
fn page_and_event<'a>(page: &'a str, events: &'a [(&str, String)], name: &str) -> [&'a str; 2] {
    let event = events
        .iter()
        .find(|(event, _)| *event == name)
        .map(|(_, html)| html.as_str())
        .expect("event");
    [page, event]
}

/// ACC-GRAPH-47-GAUGE-SMOOTH: each arc and its colour band draw the 1 s EMA;
/// the number, the aria-label and the stat tiles keep btop's raw value.
#[tokio::test]
async fn gauge_arcs_draw_the_ema_and_readouts_stay_raw() {
    let mut snapshot = fixture_snapshot();
    snapshot.cpu.total_pct = Some(80.0);
    snapshot.cpu.gauge_pct = Some(50.0);
    snapshot.mem.ram_percent = Some(70.0);
    snapshot.mem.gauge_segments = vec![("used", 25.0), ("cache", 15.0)];
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let events = fragments(&snapshot);
    let cpu_docs = page_and_event(&page, &events, "cpu");
    for html in cpu_docs {
        for marker in [
            format!(
                r#"<path class="g-fill warn" d="{}">"#,
                gauge_arc(Some(50.0))
            ),
            format!(
                r#"<path class="g-fill warn" d="{}">"#,
                gauge_arc_circle(Some(50.0))
            ),
            r#"<svg class="g-circle" viewBox="-12 0 132 114""#.to_string(),
            r#"<svg class="g-squircle" viewBox="-12 0 132 122""#.to_string(),
            r#"<svg class="g-wide" viewBox="13 19.7 82 86.3" preserveAspectRatio="none""#
                .to_string(),
            r#"<text class="g-value" x="54" y="66">80<tspan class="g-unit" dx="2.5">%</tspan>"#
                .to_string(),
            r#"aria-label="CPU total 80 percent""#.to_string(),
            r#"<span class="stat-value crit" data-w="100" data-range="100" id="cpu-total">80</span>"#.to_string(),
            // DEF-LAYOUT-94-SHORT-VIEW: the stretched short-view shape needs
            // both SVG boxes and the undistorted HTML readout it shows.
            r#"preserveAspectRatio="none""#.to_string(),
            r#"<div class="g-read" aria-hidden="true"><b>80<i>%</i></b><span>total</span></div>"#
                .to_string(),
        ] {
            assert!(html.contains(&marker), "lacks {marker}");
        }
        // DEF-LAYOUT-90-LOAD-SHIFT, DEF-LAYOUT-94-SHORT-VIEW: the three
        // shapes each carry the fill and the stylesheet picks one. Only the
        // CPU gauge takes a band colour (ACC-VIEW-175-MEM-SHARE).
        // DEF-VIEW-112-WIDE-ARC: the wide shape's fill is a dash of its band.
        assert_eq!(html.matches(r#"<path class="g-fill warn""#).count(), 2);
        assert_eq!(
            html.matches(r##"<use class="g-fill warn" href="#g-wide-arc" style="--from:0;--to:50.00"></use>"##)
                .count(),
            1
        );
    }
    // DEF-VIEW-112-WIDE-ARC: the page holds the wide band once; a frame carries
    // no copy of its path.
    let band = format!(
        r#"<path id="g-wide-arc" vector-effect="non-scaling-stroke" d="{}">"#,
        gauge_arc(Some(100.0))
    );
    assert_eq!(page.matches(&band).count(), 1);
    assert!(
        events
            .iter()
            .all(|(_, html)| !html.contains(r#"id="g-wide-arc""#))
    );
    let mem_docs = page_and_event(&page, &events, "mem");
    for html in mem_docs {
        // ACC-VIEW-190-MEM-RECLAIM: one fill per segment in each of the three
        // shapes, each running from where the last one ended. DEF-VIEW-112:
        // the wide shape's is a dash of its band, so it carries the percents.
        for (seg, from, to) in [("used", 0.0, 25.0), ("cache", 25.0, 40.0)] {
            let squircle = format!(
                r#"<path class="g-fill" data-seg="{seg}" d="{}">"#,
                gauge_span(from, to)
            );
            assert_eq!(html.matches(&squircle).count(), 1, "{seg}");
            let circle = format!(
                r#"<path class="g-fill" data-seg="{seg}" d="{}">"#,
                gauge_span_circle(from, to)
            );
            assert_eq!(html.matches(&circle).count(), 1, "{seg}");
            let wide = format!(
                r##"<use class="g-fill" data-seg="{seg}" href="#g-wide-arc" style="--from:{from:.2};--to:{to:.2}"></use>"##
            );
            assert_eq!(html.matches(&wide).count(), 1, "{seg}");
        }
        assert_eq!(html.matches(r#"<path class="g-fill" data-seg"#).count(), 4);
        let (value, unit) = snapshot.mem.ram_used_parts().expect("memory used");
        let readout = format!(
            r#"<text class="g-value" x="54" y="66">{}<tspan class="g-unit" dx="2.5">{}</tspan>"#,
            value, unit
        );
        assert!(
            html.contains(&readout),
            "the memory readout is not centred on the gauge axis"
        );
        let overlay = format!(
            r#"<div class="g-read" aria-hidden="true"><b>{}<i>{}</i></b><span>used</span></div>"#,
            value, unit
        );
        assert!(
            html.contains(&overlay),
            "the short-view memory readout is missing or differs"
        );
        assert!(
            html.contains(r#"<b data-w="100%">70%</b> used"#),
            "the memory foot is not raw"
        );
    }
}

/// ACC-LAYOUT-74-RESERVED-WIDTH: the memory numbers batch 3 re-aligned keep
/// their widest form, so a change of digits moves no unit or following word:
/// the tile's value, the used and available percents, the gauge's percent
/// cells and the available value. The CPU tile, gauge readout, load, GPU
/// legend, network scale and process counts are pinned by their own tests.
#[tokio::test]
async fn reserved_widths_sit_on_the_memory_values() {
    let snapshot = fixture_snapshot();
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let (value, unit) = snapshot.mem.ram_used_parts().expect("memory used");
    let used = snapshot.mem.ram_percent.expect("used percent");
    let available = snapshot
        .mem
        .ram_available_percent
        .expect("available percent");
    for html in page_and_event(&page, &fragments(&snapshot), "mem") {
        for marker in [
            format!(
                r#"<span class="stat-value" data-w="0000" data-range="{}" id="mem-used">{value}</span><span class="stat-unit">{unit}</span>"#,
                snapshot.mem.ram_used_range()
            ),
            format!(r#"<b data-w="100%">{used:.0}%</b> used"#),
            format!(r#"<b data-w="100%">{available:.0}%</b> available"#),
            format!(
                r#"<span>free + reclaimable</span><b data-w="0000 MiB">{}</b><span class="pct" data-w="100%">{:.0}%</span>"#,
                snapshot.mem.ram_free_reclaimable_label(),
                snapshot
                    .mem
                    .ram_free_reclaimable_percent()
                    .expect("sum percent")
            ),
        ] {
            assert!(html.contains(&marker), "lacks {marker}");
        }
        let cells = html.matches(r#"class="pct""#).count();
        assert!(cells > 0, "no memory percent cells");
        assert_eq!(
            html.matches(r#"<span class="pct" data-w="100%">"#).count(),
            cells,
            "a memory percent cell does not reserve its width"
        );
    }
}

/// ACC-VIEW-48-CORE-BARS, DEF-VIEW-37-CORE-LABEL and ACC-VIEW-56-CORE-COLOUR:
/// one row per core - a two-digit label, a meter and that core's 1 s average
/// as an integer with its percent sign; the row carries the average as
/// --value, which sets the meter length and the gradient colour - with no
/// tinted cell, no band class and none of the raw value.
#[tokio::test]
async fn cores_render_rows_with_a_meter_and_the_smoothed_number() {
    let mut snapshot = fixture_snapshot();
    snapshot.cpu.cores = vec![
        CoreUsage {
            name: "cpu0".to_string(),
            usage_pct: Some(91.0),
            smoothed_pct: Some(42.4),
        },
        CoreUsage {
            name: "cpu1".to_string(),
            usage_pct: Some(3.0),
            smoothed_pct: Some(57.6),
        },
    ];
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let events = fragments(&snapshot);
    for html in page_and_event(&page, &events, "cpu") {
        // The cores panel runs to the end of its section, or of the event.
        let cores = &html[html.find(r#"class="cores-body""#).expect("cores body")..];
        let cores = &cores[..cores.find("</section>").unwrap_or(cores.len())];
        for row in [
            r#"<div class="core-row" data-tip="Core cpu0, unit percent&#10;source: /proc/stat cpu0 line, delta&#10;1 s average shown by the meter and number" style="--value:42.4"><span class="cl">C00</span><span class="meter"></span><span class="cv"><span data-w="100">42</span><i>%</i></span></div>"#,
            r#"<div class="core-row" data-tip="Core cpu1, unit percent&#10;source: /proc/stat cpu1 line, delta&#10;1 s average shown by the meter and number" style="--value:57.6"><span class="cl">C01</span><span class="meter"></span><span class="cv"><span data-w="100">58</span><i>%</i></span></div>"#,
        ] {
            assert!(cores.contains(row), "lacks {row} in {cores}");
        }
        for tint in ["core-cell", "--v:", "warn", "crit", ">91<", ">3<"] {
            assert!(!cores.contains(tint), "the cores carry {tint}");
        }
    }
}

/// DEF-VIEW-37-CORE-LABEL and ACC-VIEW-54-CORE-LABEL: the 64-core fixture
/// names its rows C00 to C63 in order, each value with its percent sign.
#[tokio::test]
async fn cores_name_every_row_with_two_digits() {
    let snapshot = fixture_snapshot();
    assert_eq!(snapshot.cpu.cores.len(), 64);
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let events = fragments(&snapshot);
    for html in page_and_event(&page, &events, "cpu") {
        let labels: Vec<&str> = html
            .split(r#"<span class="cl">"#)
            .skip(1)
            .map(|rest| &rest[..rest.find('<').expect("label end")])
            .collect();
        let expected: Vec<String> = (0..64).map(|n| format!("C{n:02}")).collect();
        assert_eq!(labels, expected);
        assert_eq!(html.matches("<i>%</i></span></div>").count(), 64);
    }
}

/// ACC-VIEW-49-TOTAL-LABEL, ACC-VIEW-129-LOAD-LABELS, ACC-VIEW-130-LOAD-SCALE
/// and ACC-VIEW-131-LOAD-WIDTH: the CPU gauge reads TOTAL, and beside it each
/// load average as a whole percent of all cores with its window, both load
/// figures reserving normalised 100%.
#[tokio::test]
async fn cpu_gauge_says_total_and_shows_the_load_triple() {
    let mut snapshot = fixture_snapshot();
    assert_eq!(snapshot.cpu.cores.len(), 64);
    snapshot.cpu.total_pct = Some(25.0);
    snapshot.cpu.load_avg = Some([32.0, 16.0, 9.6]);
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let events = fragments(&snapshot);
    for html in page_and_event(&page, &events, "cpu") {
        for marker in [
            r#"<text class="g-label" x="54" y="78">total</text>"#,
            r#"<span>load</span><b><span class="load-cell"><span data-w="100%" data-num>50%</span>/1m</span> <span class="load-cell"><span data-w="100%" data-num>25%</span>/5m</span> <span class="load-cell"><span data-w="100%" data-num>15%</span>/15m</span></b>"#,
            r#"load <b data-w="100%" data-num>50%</b>/1m <b data-w="100%" data-num>25%</b>/5m <b data-w="100%" data-num>15%</b>/15m"#,
            "Load average, unit percent of all cores",
            "1, 5 and 15 minute averages divided by 64 cores",
        ] {
            assert!(html.contains(marker), "lacks {marker}");
        }
    }
    for (name, html) in std::iter::once(("page", &page)).chain(events.iter().map(|(n, h)| (*n, h)))
    {
        assert!(!html.contains("BUSY"), "{name} says BUSY");
        assert!(!html.contains(">busy<"), "{name} labels a readout busy");
        assert!(!html.contains("cores busy"), "{name} shows cores busy");
        assert!(!html.contains("percent busy"), "{name} reads busy aloud");
    }
}

/// ACC-VIEW-178-CPU-FACTS, DEF-VIEW-117-CPU-CHIPS: beside the gauge a plain
/// label names each of four facts in order, no chip, and each value comes from
/// the recorded tree: load from
/// loadavg over 64 cores, user and system from the /proc/stat delta, pressure
/// from pressure/cpu. Cores busy, frequency and temperature left the column.
#[tokio::test]
async fn cpu_facts_name_four_values_from_the_recorded_tree() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proc");
    let root = std::env::temp_dir().join(format!("btop-web-facts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let tree = root.join("proc");
    std::fs::create_dir_all(tree.join("pressure")).expect("tree");
    for file in ["loadavg", "pressure/cpu"] {
        std::fs::copy(fixtures.join(file), tree.join(file)).expect("copy");
    }
    let config = Config::load(None).expect("config");
    std::fs::write(tree.join("stat"), "cpu  100 0 100 800 0 0 0 0 0 0\n").expect("stat");
    cpu::collect(&tree, &config);
    std::fs::write(tree.join("stat"), "cpu  300 0 200 1500 0 0 0 0 0 0\n").expect("stat");
    let mut snapshot = fixture_snapshot();
    snapshot.cpu = cpu::collect(&tree, &config);
    std::fs::remove_dir_all(&root).expect("cleanup");
    // This stat has no cpuN lines; the load divides by the recorded 64 cores.
    snapshot.cpu.effective_cpus = Some(64.0);

    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let events = fragments(&snapshot);
    for html in page_and_event(&page, &events, "cpu") {
        let side = &html[html.find(r#"data-field="cpu-gauge-facts""#).expect("facts")..];
        let side = &side[..side.find("</div>\n</div>").expect("facts end")];
        let labels: Vec<&str> = side
            .split(r#"<span>"#)
            .skip(1)
            .map(|rest| &rest[..rest.find('<').expect("label end")])
            .collect();
        assert_eq!(labels, ["load", "user", "system", "pressure"]);
        assert!(!side.contains("chip"), "a fact is still a chip: {side}");
        for marker in [
            r#">139%</span>/1m"#,
            r#">144%</span>/5m"#,
            r#">142%</span>/15m"#,
            r#"<span>user</span><b data-w="100%">20%</b>"#,
            r#"<span>system</span><b data-w="100%">10%</b>"#,
            r#"<span>pressure</span><b data-w="100.00%">1.25%</b>"#,
        ] {
            assert!(side.contains(marker), "lacks {marker} in {side}");
        }
        for gone in ["cores busy", "frequency", "temperature", "GHz", "°C"] {
            assert!(!side.contains(gone), "the column still shows {gone}");
        }
    }
}

/// The CPU stat header's facts, from the page or a cpu event.
fn cpu_stat_facts(html: &str) -> &str {
    let facts = &html[html.find(r#"data-field="cpu-stat-facts""#).expect("facts")..];
    &facts[..facts.find("</header>").expect("header end")]
}

/// DEF-VIEW-114-CPU-TEMP: on the recorded tree with sensors the CPU stat header
/// shows the temperature beside the frequency, in the frequency's form plus a
/// reserved width. The gauge column's test above keeps it out of the four facts.
#[tokio::test]
async fn cpu_stat_header_shows_the_temperature_where_a_sensor_reads() {
    let snapshot = recorded_snapshot("tests/fixtures/proc", 1);
    assert_eq!(snapshot.cpu.temperature_c, Some(45.0));
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let marker = format!(
        r#"<b>{}</b></span><span class="cpu-temp" data-tip="CPU temperature, unit degrees Celsius&#10;source: hwmon and thermal sysfs, m°C / 1000&#10;whole degrees, btop's sensor choice"><b data-w="100°C">45°C</b></span>"#,
        snapshot.cpu.frequency_label()
    );
    for html in page_and_event(&page, &fragments(&snapshot), "cpu") {
        let facts = cpu_stat_facts(html);
        assert!(facts.contains(&marker), "lacks {marker} in {facts}");
    }
}

/// DEF-VIEW-114-CPU-TEMP: the recorded tree with no hwmon or thermal class has
/// a frequency and no sensor, so the header shows no temperature at all.
#[tokio::test]
async fn cpu_stat_header_has_no_temperature_without_a_sensor() {
    let snapshot = recorded_snapshot("tests/fixtures/container/proc", 1);
    assert_eq!(snapshot.cpu.temperature_c, None);
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    for html in page_and_event(&page, &fragments(&snapshot), "cpu") {
        let facts = cpu_stat_facts(html);
        assert!(facts.contains("CPU frequency"), "{facts}");
        for absent in ["temperature", "°C", "unavailable"] {
            assert!(!facts.contains(absent), "{absent} in {facts}");
        }
    }
}

/// ACC-VIEW-178-CPU-FACTS and INTENT's no invented data: on the empty tree each
/// of the four facts keeps its chip and reads unavailable, never zero.
#[tokio::test]
async fn cpu_facts_read_unavailable_on_the_empty_tree() {
    let snapshot = unavailable_snapshot();
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let events = fragments(&snapshot);
    for html in page_and_event(&page, &events, "cpu") {
        for label in ["load", "user", "system", "pressure"] {
            let marker = format!(r#"<span>{label}</span><span>unavailable</span>"#);
            assert!(html.contains(&marker), "lacks {marker}");
        }
    }
}

/// ACC-VIEW-179-PROC-HEADERS: the sort menu and the column toggles read the
/// table headers' uppercase words, in the headers' order, and MEM has replaced
/// MemB. Each toggle keeps its key; [`process_headers_are_sort_buttons`] pins
/// the headers themselves.
#[tokio::test]
async fn sort_menu_and_column_toggles_use_the_header_words() {
    let page = page_of(&app(Some(fixture_snapshot()), true)).await;
    let select = &page[page.find(r#"id="proc-sort""#).expect("sort menu")..];
    let select = &select[..select.find("</select>").expect("sort menu end")];
    let menu = &page[page.find(r#"id="proc-col-menu""#).expect("column menu")..];
    let menu = &menu[..menu.find("</div>").expect("column menu end")];
    for (key, word) in [
        ("pid", "PID"),
        ("name", "PROGRAM"),
        ("command", "COMMAND"),
        ("threads", "THREADS"),
        ("user", "USER"),
        ("mem", "MEM"),
        ("cpu", "CPU%"),
    ] {
        assert!(
            select.contains(&format!(r#"<option value="{key}""#))
                && select.contains(&format!(">{word}</option>")),
            "the sort menu lacks {key} as {word}"
        );
        let at = menu
            .find(&format!(r#"data-col="{key}""#))
            .unwrap_or_else(|| panic!("no {key} toggle"));
        assert!(
            menu[at..]
                .split("</button>")
                .next()
                .expect("toggle")
                .ends_with(&format!(">{word}")),
            "the {key} toggle does not read {word}"
        );
    }
    assert!(
        select.contains(r#"<option value="mem" selected>MEM</option>"#),
        "the default sort moved"
    );
    for old in [
        ">MemB<",
        ">Cpu%<",
        ">Pid<",
        ">Program<",
        ">Command<",
        ">Threads<",
        ">User<",
    ] {
        assert!(!page.contains(old), "the page still reads {old}");
    }
}

/// ACC-VIEW-130-LOAD-SCALE and ACC-VIEW-131-LOAD-WIDTH: the aggregate scale
/// reaches the gauge and the sparkline header, and every figure reserves the
/// core count times 100.
#[tokio::test]
async fn aggregate_load_renders_in_the_gauge_and_the_header() {
    let mut snapshot = fixture_snapshot();
    assert_eq!(snapshot.cpu.cores.len(), 64);
    snapshot.cpu.load_avg = Some([32.0, 16.0, 9.6]);
    snapshot.cpu.load_scale = LoadScale::Aggregate;
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let events = fragments(&snapshot);
    for html in page_and_event(&page, &events, "cpu") {
        for marker in [
            r#"<span>load</span><b><span class="load-cell"><span data-w="6400%" data-num>3200%</span>/1m</span> <span class="load-cell"><span data-w="6400%" data-num>1600%</span>/5m</span> <span class="load-cell"><span data-w="6400%" data-num>960%</span>/15m</span></b>"#,
            r#"load <b data-w="6400%" data-num>3200%</b>/1m <b data-w="6400%" data-num>1600%</b>/5m <b data-w="6400%" data-num>960%</b>/15m"#,
            "Load average, unit percent of one core",
            "summed over all cores; 64 cores cap it at 6400%",
        ] {
            assert!(html.contains(marker), "lacks {marker}");
        }
    }
}

/// ACC-SCOPE-64-CONTAINER: the recorded container tree renders only the
/// container's values; what has no container source reads unavailable, and
/// the host values the tree also carries (loadavg 14.22, the 60 cpuN lines
/// outside the cpuset, the cpuinfo frequency) appear nowhere. The memory bar
/// (DEF-VIEW-50-MEM-TERMS, DEF-VIEW-95-SCOPE-MIX) states each row's scope:
/// used and cache are this container's docker working set and file cache;
/// this recorded memory.max (8 GiB) binds, so the total is that limit, free
/// is the room under it, and other is dropped.
/// DEF-CPU-44: the load is the container's own, two R or D tasks over E = 2.
#[tokio::test]
async fn container_page_shows_only_container_values() {
    let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/container/proc");
    let mut config = Config::load(None).expect("config");
    config.container = btop_web::collector::container::detect(&tree);
    let container = config
        .container
        .clone()
        .expect("the container tree is a container");
    cpu::count_container_load(&tree, container.cgroup.as_deref().expect("cgroup"));
    let snapshot = recorded_snapshot_with("tests/fixtures/container/proc", 2, config);
    assert!(snapshot.container);
    let (_sender, receiver) = watch::channel(Some(Arc::new(Frame::new(snapshot.clone()))));
    let mut state = AppState::new(receiver, true);
    state.container = Some(container.tip().into());
    let page = String::from_utf8(get(&btop_web::app(state), "/").await.2).expect("utf-8 page");

    let start = page
        .find(r#"<span class="chip chip-container" data-p="2" data-field="container-chip""#)
        .expect("CONTAINER chip");
    let chip = &page[start..start + page[start..].find(">").expect("chip tag end")];
    // DEF-VIEW-74: the chip's detector and cgroup path live in its one
    // tooltip; a title would make the browser draw a second box.
    let cgroup = container.cgroup.expect("cgroup").display().to_string();
    assert!(
        chip.contains(&format!(
            "detected by 1 /.dockerenv&#10;docker container, cgroup {cgroup}"
        )),
        "chip tip lacks the detector and cgroup path: {chip}"
    );
    assert!(
        !chip.contains(" title="),
        "chip also carries a title: {chip}"
    );

    let events = fragments(&snapshot);
    for html in page_and_event(&page, &events, "cpu") {
        for marker in [
            r#"<span>load</span><b><span class="load-cell"><span data-w="100%" data-num>100%</span>/1m</span> <span class="load-cell"><span data-w="100%" data-num>100%</span>/5m</span> <span class="load-cell"><span data-w="100%" data-num>100%</span>/15m</span></b>"#,
            "divided by 2 cores",
            r#">load <b data-w="100%" data-num>100%</b>/1m <b data-w="100%" data-num>100%</b>/5m <b data-w="100%" data-num>100%</b>/15m</span>"#,
            r#">2 cores</span>"#,
            // DEF-CPU-41: the cpuset 0-3 gives four core rows, named C00 to C03.
            r#"4 &middot; 0&ndash;100%</span>"#,
            r#"<span class="cl">C03</span>"#,
            // ACC-VIEW-178-CPU-FACTS: the cgroup's cpu.pressure, not the
            // tree's host /proc/pressure/cpu (7.77).
            r#"<span>pressure</span><b data-w="100.00%">0.37%</b>"#,
        ] {
            assert!(html.contains(marker), "lacks {marker}");
        }
        assert!(!html.contains(">7.77%<"), "the host pressure leaked");
        assert_eq!(html.matches(r#"class="core-row""#).count(), 4);
        assert!(
            !html.contains("load unavailable"),
            "the container load is withheld"
        );
        assert!(!html.contains("14 13 15"), "the host load leaked");
        // The gauge scale coordinates can contain "7.11" inside a number; the
        // host's per-core load renders as the text node itself.
        assert!(!html.contains(">7.11<"), "the host load per core leaked");
        assert!(!html.contains("64 cores"), "the host core count leaked");
        assert!(!html.contains("of 64"), "the host core count leaked");
        assert!(!html.contains("GHz"), "the host frequency leaked");
        assert!(!html.contains("°C"), "a host temperature leaked");
    }
    // DEF-GPU-40: the GPU panel takes the host path; this tree records no GPU.
    for html in page_and_event(&page, &events, "gpu") {
        assert!(
            !html.contains(
                r#"<h2 id="gpu-title">GPU</h2><span class="head-right meta" data-withheld"#
            ),
            "GPU header is withheld"
        );
        assert!(
            html.contains("GPU metrics unavailable"),
            "GPU panel left the host path"
        );
    }
    for html in page_and_event(&page, &events, "mem") {
        // DEF-VIEW-95-SCOPE-MIX: the recorded memory.max (8 GiB) binds, so
        // the header total is the container's limit, not MemTotal.
        // DEF-VIEW-97-MEM-UNITS: the tag also names whose limit it is.
        for header in [
            ">of 8.00 GiB (container limit)</span>",
            ">of total 8.00 GiB (container limit)</span>",
        ] {
            assert_eq!(
                html.matches(header).count(),
                1,
                "memory total does not state it is the container's limit"
            );
        }
        // ACC-VIEW-194-MEM-WORDS: the first two rows' words drop the scope and
        // their tooltips keep it; DEF-VIEW-91-MEM-CONFUSE: the second says it
        // is file cache, the answer to whether it is swap.
        assert_mem_bar(
            html,
            &[
                ("used", "used"),
                ("cache", "reclaimable"),
                ("free", "container free"),
            ],
        );
        for tip in [
            "this container's working set",
            "this container's reclaimable file cache",
        ] {
            assert!(html.contains(tip), "a row's tooltip lost {tip}");
        }
        // ACC-VIEW-195-MEM-SUM: 4 GiB free + 1 GiB reclaimable, of 8 GiB.
        assert!(
            html.contains(
                r#"<span>free + reclaimable</span><b data-w="0000 MiB">5.00 GiB</b><span class="pct" data-w="100%">63%</span>"#
            ),
            "the sum row is not free plus reclaimable with its percent"
        );
        assert!(
            html.contains("the container free and reclaimable rows"),
            "the sum row's tooltip does not name its source rows"
        );
        assert!(
            !html.contains(r#"data-seg="other""#),
            "other is drawn under a binding limit"
        );
        // DEF-VIEW-96-MEM-WRONG: 4 GiB memory.current - 1 GiB inactive_file.
        assert!(html.contains("3.00 GiB"), "used is not the working set");
        assert!(html.contains("1.00 GiB"), "cache is not inactive_file");
        assert!(
            html.contains("IO unavailable"),
            "storage IO is not withheld"
        );
    }
    let proc_rows = page_and_event(&page, &events, "proc");
    for html in proc_rows {
        assert!(!html.contains("13_start_mlflow"), "pid 900 is not a member");
        assert!(html.contains("ttyd"), "member pid 7 is missing");
    }
}

/// DEF-VIEW-91-MEM-CONFUSE, DEF-VIEW-97-MEM-UNITS: an unlimited memory.max
/// leaves the host's MemTotal as the total, so the header says GiB and whose
/// total it is, and the rows name the container's used and file cache, the
/// host's memory outside the container, and the host's free memory.
#[tokio::test]
async fn unlimited_container_memory_names_the_host_total_and_row_scopes() {
    let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/container/proc");
    let mut config = Config::load(None).expect("config");
    config.container = btop_web::collector::container::detect(&tree);
    let mut snapshot = recorded_snapshot_with("tests/fixtures/container/proc", 1, config);
    // The collector's unlimited-container shape: no binding limit, MemTotal
    // as the total, and other for the host's memory outside this container.
    snapshot.mem.ram_limit = None;
    snapshot.mem.ram_total = Some(527_649_328 * 1024);
    snapshot.mem.ram_other = Some(50 * 1024 * 1024 * 1024);
    snapshot.mem.ram_other_percent = Some(10.0);
    snapshot.mem.ram_other_cache = Some(250 * 1024 * 1024 * 1024);
    snapshot.mem.ram_other_cache_percent = Some(50.0);
    let spans = [("used", 0.0, 10.0), ("cache", 10.0, 30.0)];
    snapshot.mem.gauge_segments = spans
        .iter()
        .map(|(name, from, to)| (*name, to - from))
        .collect();
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    for html in page_and_event(&page, &fragments(&snapshot), "mem") {
        // ACC-VIEW-192-GAUGE-CONTAINER: every shape draws the container's two
        // segments from zero and the rest is the bare track, while the bar
        // below keeps all five rows; the tooltip names what the arc draws.
        for (name, from, to) in spans {
            for path in [gauge_span_circle(from, to), gauge_span(from, to)] {
                let marker = format!(r#"<path class="g-fill" data-seg="{name}" d="{path}">"#);
                assert!(html.contains(&marker), "lacks {name} {from} to {to}");
            }
        }
        assert_eq!(html.matches(r#"<path class="g-fill" data-seg"#).count(), 4);
        assert_eq!(html.matches(r#"<use class="g-fill" data-seg"#).count(), 2);
        for gone in ["other", "other-cache", "free"] {
            assert!(!html.contains(&format!(r#"class="g-fill" data-seg="{gone}""#)));
        }
        assert!(
            html.contains("arc: container used and reclaimable 30%, 1 s EMA"),
            "the gauge tooltip does not name what the arc draws"
        );
        // DEF-VIEW-106-SWAP-UNITS: the recorded 512 MiB of 128 GiB, in GiB.
        assert!(
            html.contains(
                r#"<span>swap</span><b>0.50 / 128 GiB</b><span class="pct" data-w="100%">0%</span>"#
            ),
            "the swap row is not in GiB with its percent"
        );
        // ACC-VIEW-193-MEM-TOTAL: both headers drop the words, the tooltip keeps
        // the scope. ACC-VIEW-196-MEM-HEADER: the table panel's states the total.
        for header in [">of 503 GiB</span>", ">of total 503 GiB</span>"] {
            assert_eq!(
                html.matches(header).count(),
                1,
                "a Memory header does not read {header}"
            );
        }
        assert!(
            !html.contains("host total"),
            "the header names the host total"
        );
        assert_eq!(
            html.matches("the host's total; memory.max is unlimited")
                .count(),
            2,
            "a header tooltip lost whose total it is"
        );
        assert_mem_bar(
            html,
            &[
                ("used", "used"),
                ("cache", "reclaimable"),
                ("other", "other used"),
                ("other-cache", "other reclaimable"),
                ("free", "free"),
            ],
        );
        // ACC-VIEW-194-MEM-WORDS: the tooltips keep the scope the words drop.
        for tip in [
            "this container's working set",
            "this container's reclaimable file cache",
        ] {
            assert!(html.contains(tip), "a row's tooltip lost {tip}");
        }
        // ACC-VIEW-195-MEM-SUM: the sum row adds other reclaimable here.
        let sum_row = format!(
            r#"<span>free + reclaimable</span><b data-w="0000 MiB">{}</b><span class="pct" data-w="100%">{:.0}%</span>"#,
            snapshot.mem.ram_free_reclaimable_label(),
            snapshot
                .mem
                .ram_free_reclaimable_percent()
                .expect("sum percent")
        );
        assert!(html.contains(&sum_row), "lacks {sum_row}");
        assert!(
            html.contains("the free, reclaimable and other reclaimable rows"),
            "the sum row's tooltip does not name its source rows"
        );
        assert!(
            html.contains("not swap; swap has its own row"),
            "the cache row does not answer whether it is swap"
        );
    }
}

/// ACC-LAYOUT-46-ADAPTIVE-PRIORITY: every field that appears once in its
/// panel carries its priority (data-p) and name (data-field), on the page and
/// in the event that re-renders it; repeated elements (core rows, process
/// cells, legend meta lines, GPU models) drop by class and carry no marker.
#[tokio::test]
async fn tier_markers_sit_on_single_elements() {
    let snapshot = fixture_snapshot();
    assert!(snapshot.cpu.name.is_some() && snapshot.net.selected_iface.is_some());
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let events = fragments(&snapshot);
    let marker = |field: &str, priority: u8| format!(r#"data-p="{priority}" data-field="{field}""#);
    // The opening tag of every marked element; <col> is void, so no close tag is sought.
    let marked_tags = |html: &'_ str| -> Vec<String> {
        html.match_indices(" data-p=")
            .map(|(at, _)| {
                let open = html[..at].rfind('<').expect("tag start");
                html[open..at + html[at..].find('>').expect("tag end") + 1].to_string()
            })
            .collect()
    };
    // (event, field, priority, count in that event)
    let fields: [(&str, &str, u8, usize); 21] = [
        ("cpu", "cpu-stat-facts", 3, 1),
        ("cpu", "cpu-stat-axis", 3, 1),
        ("cpu", "cpu-name", 3, 1),
        ("cpu", "cpu-gauge-cores", 3, 1),
        ("cpu", "cpu-gauge-facts", 3, 1),
        ("cpu", "gauge-scale", 3, 2),
        ("mem", "mem-stat-facts", 3, 1),
        ("mem", "mem-stat-axis", 3, 1),
        ("mem", "mem-gauge-total", 3, 1),
        ("mem", "mem-gauge-facts", 3, 1),
        ("mem", "gauge-scale", 3, 2),
        ("net", "net-interface", 3, 1),
        ("net", "net-axis-note", 3, 1),
        ("net", "net-x-axis", 3, 1),
        ("net", "net-link", 3, 1),
        ("gpu", "gpu-count", 3, 1),
        ("gpu", "gpu-axis-note", 3, 1),
        ("gpu", "gpu-x-axis", 3, 1),
        ("proc", "proc-command", 3, 2),
        ("proc", "proc-threads", 3, 2),
        ("proc", "proc-user", 3, 2),
    ];
    for (event, field, priority, count) in fields {
        let [_, fragment] = page_and_event(&page, &events, event);
        assert_eq!(
            fragment.matches(&marker(field, priority)).count(),
            count,
            "{event}: {field}"
        );
        // DEF-LAYOUT-90-LOAD-SHIFT: each gauge carries both shapes, so the
        // scale group is marked once per shape: twice in the page per panel.
        let in_page = if field == "gauge-scale" {
            2 * count
        } else {
            count
        };
        assert_eq!(
            page.matches(&marker(field, priority)).count(),
            in_page,
            "page: {field}"
        );
        assert_eq!(
            page.matches(&format!(r#"data-field="{field}""#)).count(),
            in_page,
            "page: {field} priority"
        );
    }
    for (field, priority, id) in [
        ("cpu-gauge", 2, "p-cpu-gauge"),
        ("mem-gauge", 2, "p-mem-gauge"),
        ("cores", 3, "p-cores"),
        ("proc-stat", 3, "p-proc-stat"),
        ("storage", 4, "p-storage"),
    ] {
        let tags: Vec<String> = marked_tags(&page)
            .into_iter()
            .filter(|tag| tag.contains(&marker(field, priority)))
            .collect();
        assert_eq!(tags.len(), 1, "{field}");
        assert!(
            tags[0].starts_with("<section") && tags[0].contains(&format!(r#"id="{id}""#)),
            "{field} is not on section {id}: {}",
            tags[0]
        );
    }
    assert_eq!(
        page.matches(&marker("clock", 4)).count(),
        1,
        "top bar clock"
    );
    // ACC-LAYOUT-62-VERSION: the version shows at every tier.
    assert!(
        page.contains(&format!(
            r#"<span class="u-meta u-ver">{}</span>"#,
            env!("CARGO_PKG_VERSION")
        )),
        "the version span carries a tier marker"
    );
    assert!(!page.contains(r#"data-field="version""#));
    assert_eq!(
        page.matches(&marker("host", 4)).count(),
        page.matches("u-host").count()
    );
    for (name, html) in
        std::iter::once(("page", page.as_str())).chain(events.iter().map(|(n, h)| (*n, h.as_str())))
    {
        for tag in marked_tags(html) {
            for repeated in ["<td", "legend-meta", "lg-model", "core-row", "legend-item"] {
                assert!(
                    !tag.contains(repeated),
                    "{name}: a repeated element carries a marker: {tag}"
                );
            }
        }
        if let Some(start) = html.find(r#"class="cores-body""#) {
            let cores = &html[start..];
            let cores = &cores[..cores.find("</section>").unwrap_or(cores.len())];
            assert!(
                !cores.contains("data-p="),
                "{name}: a core row carries a marker"
            );
        }
    }
}

/// ACC-LAYOUT-46-ADAPTIVE-PRIORITY: the top bar names the tiers on screen
/// (one span per tier, its title lists the drops), bands 1-3 sit in one
/// wrapper for the two-column form, and app.css and app.js carry the tier
/// queries in place of the old fixed breakpoints.
#[tokio::test]
async fn top_bar_names_the_tiers_and_bands_1_3_share_a_wrapper() {
    let page = String::from_utf8(get(&app(Some(fixture_snapshot()), true), "/").await.2)
        .expect("utf-8 page");
    let bar_start = page.find(r#"<header class="utility">"#).expect("top bar");
    let bar = &page[bar_start..bar_start + page[bar_start..].find("</header>").expect("bar end")];
    let chip = bar
        .find(r#"<span class="chip chip-tier" id="tier-chip""#)
        .expect("tier chip");
    assert!(
        chip < bar.find(r#"id="live-chip""#).expect("live chip"),
        "the tier chip follows LIVE"
    );
    for (tier, text) in [
        ("tier-4", "p1-p3"),
        ("tier-3", "p1-p2"),
        ("tier-2", "p1"),
        ("tier-4-gpu", "p1-p3"),
        ("tier-full-gpu", "p1-p4"),
    ] {
        let start = bar
            .find(&format!(r#"<span class="up {tier}" title=""#))
            .expect(tier);
        assert!(
            bar[start..].contains(&format!(">{text}</span>")),
            "{tier} reads {text}"
        );
    }
    let titles = &bar[chip..];
    for drop in [
        "Processes tile (count graph,",
        "CPU facts (load, user, system, pressure)",
        "memory gauge",
        "GPU temperature and power",
    ] {
        assert!(titles.contains(drop), "the tier titles do not name {drop}");
    }
    // DEF-VIEW-37-CORE-LABEL: core labels show at every tier.
    assert!(
        !titles.contains("core labels"),
        "a tier drops the core labels"
    );
    // ACC-LAYOUT-62-VERSION: the version shows at every tier.
    assert!(!titles.contains("version"), "a tier drops the version");

    let tele_start = page.find(r#"<div class="tele">"#).expect("tele wrapper");
    let band_4 = page
        .find(r#"<div class="band" data-band="4">"#)
        .expect("band 4");
    let tele = &page[tele_start..band_4];
    assert_eq!(
        tele.matches("<div").count(),
        tele.matches("</div>").count(),
        "the wrapper closes before band 4"
    );
    for band in ["1", "2", "3"] {
        assert!(
            tele.contains(&format!(r#"<div class="band" data-band="{band}">"#)),
            "band {band} is outside the wrapper"
        );
    }
    assert!(!tele.contains(r#"data-band="4""#));

    let asset = |name: &str| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join(name),
        )
        .expect(name)
    };
    let css = asset("app.css");
    for rule in [
        "@property --row-h",
        "@media (max-width: 1100px), (max-height: 640px)",
        "@media (max-width: 899px), (max-height: 640px)",
        "@media (max-width: 599px), (max-height: 540px)",
        "@media (max-width: 599px) and (min-height: 541px)",
        "@media (max-height: 540px)",
    ] {
        assert!(css.contains(rule), "app.css lacks {rule}");
    }
    assert!(
        !css.contains("max-height: 860px"),
        "app.css keeps the old short-viewport block"
    );
    let js = asset("app.js");
    for query in [
        r#""(max-width: 1100px), (max-height: 640px)""#,
        r#""(max-width: 899px), (max-height: 640px)""#,
        r#""(max-width: 599px), (max-height: 540px)""#,
    ] {
        assert!(js.contains(query), "app.js lacks {query}");
    }
    assert!(
        !js.contains(r#""(max-width: 899px)""#),
        "app.js keeps the phone query"
    );
}

/// The Processes tile in the page and in the proc event.
fn proc_tiles(snapshot: &Snapshot, page: &str) -> Vec<String> {
    let (_, event) = fragments(snapshot)
        .into_iter()
        .find(|(name, _)| *name == "proc")
        .expect("proc event");
    [page.to_string(), event]
        .iter()
        .map(|html| {
            let start = html.find(r#"<h2 id="proc-stat-title">"#).expect("tile");
            let tile = &html[start..];
            tile[..tile.find("</div>\n</div>").expect("tile end")].to_string()
        })
        .collect()
}

/// ACC-VIEW-53-PROC-TILE: beside the total, the state counts and threads; the
/// count graph in its own plot box; the top process with the table's Cpu%
/// and the 80 percent counts in the foot.
#[tokio::test]
async fn proc_tile_carries_the_count_graph_and_the_facts() {
    let snapshot = fixture_snapshot();
    assert!(!snapshot.proc.chart.line.is_empty());
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let top = snapshot.proc.top.as_ref().expect("a top process");
    for tile in proc_tiles(&snapshot, &page) {
        for marker in [
            format!(
                r#"<span class="stat-value" data-w="000" data-range="{}" id="proc-total">{}</span><span class="proc-facts">"#,
                snapshot.proc.chart_max,
                snapshot.proc.total.expect("total")
            ),
            ">running <b>1</b></span>".to_string(),
            ">sleeping <b>7</b></span>".to_string(),
            ">waiting <b>0</b></span>".to_string(),
            ">zombie <b>0</b></span>".to_string(),
            ">threads <b>85</b></span>".to_string(),
            format!(
                r#"<div class="stat-chart" data-chart="proc-count" role="img" aria-label="Process count history, 0 to {}">"#,
                snapshot.proc.chart_max
            ),
            // ACC-GRAPH-67-SMOOTH-SCALE: the series group a scale change transforms.
            format!(
                r#"<g class="series"><path class="area" d="{}"></path>"#,
                snapshot.proc.chart.area
            ),
            format!(
                r#"<path class="line" d="{}"></path></g>"#,
                snapshot.proc.chart.line
            ),
            format!(
                "top <b>{} {}%</b>",
                top.name.as_deref().expect("name"),
                top.cpu_label()
            ),
            format!(
                r#"80% cpu <b class="foot-count" data-w="000">{}</b>"#,
                snapshot.proc.cpu_80.expect("cpu_80")
            ),
            r#"80% mem <b class="foot-count" data-w="000">1</b>"#.to_string(),
        ] {
            assert!(tile.contains(&marker), "lacks {marker} in {tile}");
        }
    }
}

/// ACC-LAYOUT-74-RESERVED-WIDTH: the Processes tile reserves the digits of the
/// count graph's ceiling, so a count crossing 999 within one scale moves nothing.
#[tokio::test]
async fn proc_counts_reserve_the_ceiling_digits() {
    let mut snapshot = fixture_snapshot();
    snapshot.proc.chart_max = 2000;
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    for tile in proc_tiles(&snapshot, &page) {
        for marker in [
            r#"<span class="stat-value" data-w="0000" data-range="2000" id="proc-total">"#,
            r#"80% mem <b class="foot-count" data-w="0000">1</b>"#,
        ] {
            assert!(tile.contains(marker), "lacks {marker} in {tile}");
        }
    }
}

/// ACC-VIEW-53-PROC-TILE and ACC-VIEW-23-RELEVANT: a host without a GPU shows
/// no GPU process fact; with devices it shows the count, or unavailable, and
/// a lower bound with unreadable processes (DEF-VIEW-48-GPU-FACT).
#[tokio::test]
async fn proc_tile_gpu_fact_follows_the_devices() {
    for (devices, count, unreadable, expected) in [
        (false, None, 0, None),
        (true, None, 0, Some(">gpu unavailable</span>")),
        (true, Some(3), 0, Some(">gpu <b>3</b></span>")),
        (true, Some(3), 2, Some(">gpu <b>3+</b></span>")),
    ] {
        let mut snapshot = fixture_snapshot();
        if !devices {
            snapshot.gpu.devices.clear();
        }
        snapshot.gpu.process_count = count;
        snapshot.gpu.process_unreadable = unreadable;
        let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
            .expect("utf-8 page");
        for tile in proc_tiles(&snapshot, &page) {
            match expected {
                Some(fact) => assert!(tile.contains(fact), "lacks {fact} in {tile}"),
                None => assert!(!tile.contains(">gpu "), "a GPU fact without a GPU"),
            }
            if unreadable > 0 {
                assert!(
                    tile.contains("processes it could not read"),
                    "the lower bound is unexplained: {tile}"
                );
            }
        }
    }
}

/// ACC-VIEW-53-PROC-TILE: without processes the tile reads unavailable alone;
/// no fact, graph or zero renders.
#[tokio::test]
async fn unavailable_processes_render_no_zero() {
    let mut snapshot = fixture_snapshot();
    snapshot.proc = btop_web::collector::ProcSnapshot::default();
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    for tile in proc_tiles(&snapshot, &page) {
        assert!(
            tile.contains(r#"<span class="stat-na">unavailable</span>"#),
            "{tile}"
        );
        for absent in ["<b>0</b>", "proc-facts", "data-chart", "stat-facts"] {
            assert!(!tile.contains(absent), "{absent} in {tile}");
        }
    }
}

/// DEF-THEME-38-SELECT-POPUP: the native theme list takes its scheme from the
/// page mode, not the OS, and its options paint the theme's text on its surface.
#[test]
fn theme_list_follows_the_page_mode() {
    let css = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.css"))
        .expect("app.css");
    let compact: String = css.split_whitespace().collect();
    for rule in [
        r#":root[data-mode="dark"]{color-scheme:dark;}"#,
        r#":root[data-mode="light"]{color-scheme:light;}"#,
        "select.optoption{background-color:var(--surface);color:var(--text);}",
    ] {
        assert!(compact.contains(rule), "app.css lacks {rule}");
    }
}

/// ACC-VIEW-187-FILTER-SEGMENT, ACC-VIEW-188-FILTER-HINT: the filter is a plain
/// section of the Processes bar with a lowercase, dimmed hint.
/// DEF-VIEW-110-FILTER-RED: no focus or text state recolours its left rule.
#[tokio::test]
async fn process_filter_is_a_plain_section_of_the_bar() {
    let page = String::from_utf8(get(&app(Some(Snapshot::default()), true), "/").await.2)
        .expect("utf-8 page");
    assert!(
        page.contains(r#"id="filter" type="search" placeholder="filter""#),
        "the filter's hint is not the lowercase word"
    );
    let css = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.css"))
        .expect("app.css");
    let compact: String = css.split_whitespace().collect();
    for rule in [
        "#p-proc.head-right{align-self:stretch;margin-right:-8px;}",
        ".cfg-in.filter{align-self:stretch;width:var(--filter-w);height:auto;appearance:none;-webkit-appearance:none;border:0;border-left:1pxsolidvar(--border-subtle);border-radius:0;outline:none;background:var(--surface-raised);}",
        ".cfg-in.filter::-webkit-search-decoration,.cfg-in.filter::-webkit-search-cancel-button{appearance:none;-webkit-appearance:none;}",
        ".cfg-in.filter::placeholder{color:color-mix(insrgb,var(--text)20%,var(--muted));opacity:1;}",
    ] {
        assert!(compact.contains(rule), "app.css lacks {rule}");
    }
    for state in [".cfg-in.filter:focus", ".cfg-in.filter:not("] {
        assert!(
            !compact.contains(state),
            "app.css gives the filter the state {state}"
        );
    }
}

/// Each event's out-of-band wrappers name a panel that exists once in the
/// page, and together the five events reach every swapped region.
#[tokio::test]
async fn every_oob_target_exists_in_the_page() {
    let snapshot = fixture_snapshot();
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let mut targets = Vec::new();
    let events = fragments(&snapshot);
    assert_eq!(
        events.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        ["cpu", "mem", "net", "proc", "gpu"]
    );
    for (name, html) in &events {
        let ids = oob_targets(html);
        assert!(!ids.is_empty(), "{name} swaps nothing");
        for id in ids {
            assert_eq!(
                page.matches(&format!(r#"id="{id}""#)).count(),
                1,
                "{name}: target {id} is not one element of the page"
            );
            assert!(
                page.contains(&format!(r#"sse-swap="{name}""#)),
                "{name} sink"
            );
            targets.push(id.to_string());
        }
    }
    targets.sort();
    assert_eq!(
        targets,
        [
            "p-cores",
            "p-cpu-gauge",
            "p-cpu-stat",
            "p-gpu",
            "p-mem-gauge",
            "p-mem-stat",
            "p-net",
            "p-proc-stat",
            "p-storage",
            "proc-table-wrap",
            "update-interval",
        ]
    );
}

/// ACC-SAFE-13-READONLY-UI: the page states the read-only state, and the
/// stated reason is the aria-describedby target of the chip.
#[tokio::test]
async fn read_only_chip_and_reason_appear_only_when_enabled() {
    const MARKERS: [&str; 3] = [
        "READ-ONLY",
        "aria-describedby=\"readonly-reason\"",
        "process signals, priority changes and every mutating request",
    ];
    for (readonly, expected) in [(true, true), (false, false)] {
        let page = String::from_utf8(get(&app(Some(Snapshot::default()), readonly), "/").await.2)
            .expect("utf-8 page");
        for marker in MARKERS {
            assert_eq!(
                page.contains(marker),
                expected,
                "readonly={readonly}: page marker {marker}"
            );
        }
    }
}

#[tokio::test]
async fn healthz_and_readyz_report_the_listener() {
    // ACC-PERF-136-SLEEP: an idle sampler is parked, so readiness must not
    // wait for a snapshot no viewer asked for.
    for app in [app(None, true), app(Some(Snapshot::default()), true)] {
        let (status, _, body) = get(&app, "/healthz").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"ok");

        let (status, _, body) = get(&app, "/readyz").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"ready");
    }
}

/// ACC-NET-181-JSON-STREAM: `/events.json` streams the sampler's frames as
/// one newline-terminated JSON document per published frame, carrying the
/// five sections and the plot paths the page draws. The values come from the
/// fixture tree, and a second published frame adds a second line at the same
/// cadence `/events` uses.
#[tokio::test]
async fn the_json_stream_writes_one_document_per_frame() {
    use http_body_util::BodyExt;

    async fn next_line(body: &mut Body) -> String {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
            .await
            .expect("a line within one second")
            .expect("the stream is still open")
            .expect("a data frame")
            .into_data()
            .expect("frame data");
        String::from_utf8(chunk.to_vec()).expect("utf-8 line")
    }

    let snapshot = fixture_snapshot();
    let (sender, receiver) = watch::channel(Some(Arc::new(Frame::new(snapshot.clone()))));
    let app = btop_web::app(AppState::new(receiver, true));

    let request = Request::builder()
        .uri("/events.json")
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let (parts, mut body) = response.into_parts();
    assert_eq!(
        content_type(&parts.headers),
        btop_web::routes::telemetry::CONTENT_TYPE
    );

    let first_line = next_line(&mut body).await;
    assert!(
        first_line.ends_with('\n'),
        "the document ends with a newline"
    );
    let document: serde_json::Value =
        serde_json::from_str(&first_line).expect("one JSON document per line");
    for key in [
        "events",
        "update_ms",
        "window_ms",
        "slow_ms",
        "slow_seq",
        "container",
        "cpu",
        "mem",
        "net",
        "proc",
        "gpu",
        "plots",
    ] {
        assert!(document.get(key).is_some(), "the document lacks {key}");
    }
    assert_eq!(
        document["events"],
        serde_json::json!(["cpu", "mem", "net", "proc", "gpu"])
    );
    assert_eq!(document["update_ms"], serde_json::json!(snapshot.update_ms));
    assert_eq!(
        document["cpu"]["total_pct"],
        serde_json::json!(snapshot.cpu.total_pct)
    );
    assert_eq!(
        document["mem"]["ram_total"],
        serde_json::json!(snapshot.mem.ram_total)
    );
    assert_eq!(
        document["net"]["selected_iface"],
        serde_json::json!(snapshot.net.selected_iface)
    );
    assert_eq!(
        document["proc"]["total"],
        serde_json::json!(snapshot.proc.total)
    );
    assert_eq!(
        document["gpu"]["devices"][0]["name"],
        serde_json::json!("RTX PRO 4000 Blackwell")
    );
    // The plot series the page draws: one CPU, one memory, two network, one
    // process and one GPU lane for the fixture's one device.
    for plot in ["cpu", "mem", "proc"] {
        assert_eq!(
            document["plots"][plot].as_array().map(Vec::len),
            Some(1),
            "plots.{plot}"
        );
    }
    assert_eq!(document["plots"]["net"].as_array().map(Vec::len), Some(2));
    assert_eq!(document["plots"]["gpu"].as_array().map(Vec::len), Some(1));
    for (plot, series) in [
        ("cpu", &document["plots"]["cpu"][0]),
        ("mem", &document["plots"]["mem"][0]),
        ("net", &document["plots"]["net"][0]),
        ("proc", &document["plots"]["proc"][0]),
        ("gpu", &document["plots"]["gpu"][0]),
    ] {
        for key in ["line", "area", "width", "height", "max"] {
            assert!(series.get(key).is_some(), "plots.{plot} lacks {key}");
        }
    }
    assert!(
        !first_line.trim_start().starts_with('<'),
        "the stream carries no HTML"
    );

    let mut second = snapshot;
    second.update_ms = 500;
    sender
        .send(Some(Arc::new(Frame::new(second))))
        .expect("the stream holds a subscriber");
    let second_line = next_line(&mut body).await;
    let document: serde_json::Value =
        serde_json::from_str(&second_line).expect("one JSON document per line");
    assert_eq!(document["update_ms"], serde_json::json!(500));
}

/// ACC-VIEW-182-COPY-JSON: the top bar's button reads
/// `/events.json?once=1`, and that document is the stream's own serialisation
/// of the newest frame: the two are byte-identical, so the clipboard carries
/// the stream's fields and plot paths, not a second shape. The page ships no
/// serialiser of its own.
#[tokio::test]
async fn the_copied_document_equals_the_stream_document() {
    let app = app(Some(fixture_snapshot()), true);
    let (status, headers, body) = get(&app, "/events.json?once=1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        content_type(&headers),
        btop_web::routes::telemetry::CONTENT_TYPE
    );
    let copied = String::from_utf8(body).expect("utf-8 document");
    assert_eq!(copied.matches('\n').count(), 1, "one document, one line");

    let (status, _, streamed) = get(&app, "/events.json").await;
    assert_eq!(status, StatusCode::OK);
    let streamed = String::from_utf8(streamed).expect("utf-8 stream");
    assert_eq!(copied, streamed, "the copy is not the stream's document");

    let script = String::from_utf8(get(&app, "/assets/app.js").await.2).expect("utf-8 script");
    assert!(
        script.contains("events.json?once=1"),
        "the button reads no once route"
    );
    assert!(
        script.contains("navigator.clipboard.writeText"),
        "the button writes no clipboard"
    );
    assert!(
        script.contains("execCommand(\"copy\")"),
        "the button has no selection fallback"
    );
    let page = String::from_utf8(get(&app, "/").await.2).expect("utf-8 page");
    assert!(
        page.contains(r#"id="copy-json""#),
        "the top bar lacks the copy button"
    );
}

/// ACC-NET-144-ACCESS-KEY: an agent listener with one key. `None` leaves the
/// state without a key, which must fail closed.
fn agent_app(key: Option<&str>) -> Router {
    let (_sender, receiver) = watch::channel(None);
    let mut state = AppState::new(receiver, true);
    state.access_key = key.map(Into::into);
    btop_web::routes::agent_router(state)
}

/// ACC-NET-144-ACCESS-KEY: every request to an agent listener needs the key.
/// A missing key, a wrong key of the right length and a key of the wrong
/// length each answer 401 with an empty body, before any handler runs; the
/// right key opens the stream, and a route the agent does not serve still
/// refuses first. The response has no fragment body and no event-stream type.
#[tokio::test]
async fn an_agent_refuses_every_request_without_its_key() {
    let app = agent_app(Some("the-right-key"));
    for path in [
        "/events",
        "/events.json",
        "/proc/1",
        "/healthz",
        "/readyz",
        "/agents",
        "/",
    ] {
        for key in [None, Some("the-wrong-key"), Some("short")] {
            let (status, headers, body) = request_with_key(&app, path, key).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} with {key:?}");
            assert!(
                body.is_empty(),
                "{path} with {key:?} carries {} body bytes",
                body.len()
            );
            assert!(
                !content_type(&headers).contains("text/event-stream"),
                "{path} with {key:?}: {}",
                content_type(&headers)
            );
        }
    }

    // The right key reaches the handlers; an unknown path is then an ordinary
    // 404, and the stream opens.
    let (status, _, body) = request_with_key(&app, "/healthz", Some("the-right-key")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"ok");
    assert_eq!(
        status_with_key(&app, "/readyz", Some("the-right-key")).await,
        StatusCode::OK
    );
    assert_eq!(
        status_with_key(&app, "/events", Some("the-right-key")).await,
        StatusCode::OK
    );
    assert_eq!(
        status_with_key(&app, "/events.json", Some("the-right-key")).await,
        StatusCode::OK
    );
    assert_eq!(
        request_with_key(&app, "/agents", Some("the-right-key"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request_with_key(&app, "/", Some("the-right-key")).await.0,
        StatusCode::NOT_FOUND
    );

    // A state without a key fails closed even when a request presents one.
    let keyless = agent_app(None);
    assert_eq!(
        status_with_key(&keyless, "/healthz", Some("the-right-key")).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn read_only_refuses_every_mutating_method_before_any_handler() {
    let (_sender, receiver) = watch::channel(Some(Arc::new(Frame::new(Snapshot::default()))));
    let app = btop_web::app(AppState::new(receiver.clone(), true));
    let page = get(&app, "/").await.2;

    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        for uri in [
            "/",
            "/events",
            "/healthz",
            "/readyz",
            "/assets/app.css",
            "/config",
            "/processes/1/kill",
        ] {
            let (status, headers, body) = request(&app, method.clone(), uri).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
            assert!(
                content_type(&headers).starts_with("text/plain"),
                "{method} {uri}: {}",
                content_type(&headers)
            );
            assert_eq!(body, READ_ONLY_REFUSAL.as_bytes(), "{method} {uri}");
        }
    }

    // No mutating request reached a handler: the snapshot is still the one the
    // server started with, and every read-only endpoint still answers.
    assert!(!receiver.has_changed().expect("sender alive"));
    assert_eq!(get(&app, "/").await.2, page);
    assert_eq!(status(&app, Method::GET, "/events").await, StatusCode::OK);
    assert_eq!(
        status(&app, Method::GET, "/assets/app.css").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn without_read_only_mutating_methods_reach_routing() {
    let app = app(None, false);
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        assert_eq!(
            status(&app, method, "/").await,
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    assert_eq!(
        status(&app, Method::POST, "/processes/1/kill").await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn assets_are_served_with_their_content_types() {
    let app = app(Some(Snapshot::default()), true);
    for (path, expected_type, marker) in [
        ("/assets/app.css", "text/css", "--surface"),
        ("/assets/theme.js", "text/javascript", "btop-web-theme"),
        ("/assets/htmx.min.js", "text/javascript", "htmx"),
        ("/assets/icon.svg", "image/svg+xml", "<svg"),
    ] {
        let (status, headers, body) = get(&app, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(
            content_type(&headers).starts_with(expected_type),
            "{path}: {}",
            content_type(&headers)
        );
        assert!(
            String::from_utf8_lossy(&body).contains(marker),
            "{path} lacks {marker}"
        );
    }
    let (status, _, _) = get(&app, "/assets/missing.css").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// DEF-VIEW-59-ASSET-CACHE: the ETag is the embedded bytes' hash, so a build
/// that changes a file changes its validator; `no-cache` makes the browser
/// revalidate, and a matching validator answers 304.
#[tokio::test]
async fn assets_carry_a_content_hash_etag_and_revalidate() {
    let app = app(Some(Snapshot::default()), true);
    let header_value = |headers: &HeaderMap, name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    let (status, headers, body) = get(&app, "/assets/app.css").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_value(&headers, "cache-control"), "no-cache");
    let etag = header_value(&headers, "etag");
    assert_eq!(etag.len(), 66, "quoted sha256 hex: {etag}");
    assert!(etag.starts_with('"') && etag.ends_with('"'));
    assert!(etag[1..65].bytes().all(|byte| byte.is_ascii_hexdigit()));

    // Different bytes carry a different validator.
    let (_, js_headers, _) = get(&app, "/assets/app.js").await;
    assert_ne!(header_value(&js_headers, "etag"), etag);

    // The same bytes revalidate to 304 with the validator and no body.
    let request = Request::builder()
        .uri("/assets/app.css")
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        response
            .headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok()),
        Some(etag.as_str())
    );

    // A validator from an older build does not match: the new bytes are sent.
    let stale = format!("\"{}\"", "0".repeat(64));
    let request = Request::builder()
        .uri("/assets/app.css")
        .header(header::IF_NONE_MATCH, stale)
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let served = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(served.as_ref(), body.as_slice());
}

/// ACC-THEME-154-NARROW-FONT, ACC-THEME-155-FONT-TOGGLE,
/// ACC-THEME-167-NARROW-ONLY, DEF-THEME-78-FONT-PARTIAL,
/// DEF-THEME-86-FONT-SCOPE: the narrow Galaxa face is the only one embedded
/// and served; the wide source keeps no face and its asset URL answers 404;
/// the stylesheet keeps the system stack on `--font-ui` and `--font-display`
/// at the base, the root `data-font` attribute points both tokens at the
/// narrow face, no theme sets a face of its own, stat values and headings read
/// the display token, the process list and the CONFIG popover keep the system
/// stack, and the process Program and Command cells stay on the mono stack.
#[tokio::test]
async fn narrow_face_is_the_only_galaxa_face_served_and_the_font_toggle_selects_it() {
    let app = app(Some(Snapshot::default()), true);
    // ACC-THEME-167-NARROW-ONLY: the wide face is no longer embedded, so its
    // asset URL is a miss instead of a font.
    let (status, _, _) = get(&app, "/assets/fonts/GalaxaLabsTron-VF.ttf").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let css = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.css"))
        .expect("app.css");
    // The narrow family is the only @font-face; every font-family line that
    // names a Galaxa face names the narrow one, and the wide file is not
    // loaded.
    assert_eq!(
        css.matches("@font-face").count(),
        1,
        "app.css declares more than one face"
    );
    for line in css
        .lines()
        .filter(|line| line.contains("font-family") && line.contains("GalaxaLabsTron"))
    {
        assert!(
            line.contains("\"GalaxaLabsTron Narrow\""),
            "app.css names the wide family: {line}"
        );
    }
    assert!(
        !css.contains("GalaxaLabsTron-VF.ttf"),
        "app.css still loads the wide file"
    );
    assert!(
        css.contains("font-family: \"GalaxaLabsTron Narrow\";")
            && css
                .contains("src: url(\"fonts/GalaxaLabsTronNarrow-VF.ttf\") format(\"truetype\");"),
        "app.css does not declare the narrow face"
    );
    let rule = |selector: &str| {
        let start = css
            .find(selector)
            .unwrap_or_else(|| panic!("app.css lacks {selector}"));
        let end = start + css[start..].find('}').expect("rule end");
        &css[start..end]
    };
    // Body text reads the UI token; stat values and headings read the display
    // token, so the face swap shows on both.
    assert!(
        rule("body {\n  overflow: hidden;").contains("font-family: var(--font-ui)"),
        "the body does not use the UI face"
    );
    for selector in [".ph h2 {", ".stat-value {"] {
        assert!(
            rule(selector).contains("font-family: var(--font-display)"),
            "{selector} does not use the display face"
        );
    }
    let flat: String = css.split_whitespace().collect::<Vec<_>>().join(" ");
    // Both tokens come from the system stacks at the base; the FONT control
    // moves both to the narrow face, and no theme sets either token, so
    // changing theme keeps the face the FONT control selected.
    assert!(
        flat.contains(":root { --ui: system-ui,")
            && flat.contains("--font-ui: var(--ui);")
            && flat.contains("--font-display: var(--ui);"),
        "the base token block does not define both font tokens from the system stack"
    );
    assert!(
        flat.contains(
            "[data-font=\"galaxatron\"] { --font-ui: var(--font-narrow); \
             --font-display: var(--font-narrow); \
             --track: calc(0.5px + 0.01em); letter-spacing: var(--track); }"
        ),
        "the GalaxaTron option does not point both text tokens at the narrow face"
    );
    // The process list, its popup and the CONFIG popover are the only
    // exceptions: each redeclares the UI token and its own family from the
    // system stack, and ACC-THEME-169 returns the tracking to 0 for that system
    // face.
    assert!(
        flat.contains(
            "[data-font=\"galaxatron\"] #p-proc, [data-font=\"galaxatron\"] .proc-detail, \
             [data-font=\"galaxatron\"] .config \
             { --font-ui: var(--ui); --track: 0px; font-family: var(--ui); \
             letter-spacing: var(--track); }"
        ),
        "the process list, its popup and the CONFIG popover do not keep the system stack"
    );
    assert_eq!(
        flat.matches("--font-ui:").count(),
        3,
        "a rule other than the base, the FONT control and the exceptions sets the UI face"
    );
    assert_eq!(
        flat.matches("--font-display:").count(),
        2,
        "a rule other than the base and the FONT control sets the display face"
    );
    assert!(
        !flat.contains("[data-theme=\"cyberpunk\"] { --font"),
        "the cyberpunk pair still carries a face of its own"
    );
    // ACC-LAYOUT-74-RESERVED-WIDTH: the width of a value's widest form is the
    // same text in the element's own font, so the reserve follows whichever
    // face the FONT control selected (the narrow face's 654.5 unit digits
    // included) and a value change moves nothing under either choice.
    let reserved = rule("[data-w]::before {");
    assert!(
        reserved.contains("content: attr(data-w)"),
        "the reserved width is not the widest form's own text"
    );
    assert!(
        !reserved.contains("font-family"),
        "the reserved width names a face instead of inheriting the selected one"
    );
    // The process list's Program and Command cells are the single exception:
    // they keep the mono stack, and the Pid cell follows the UI face. The
    // tree prefix is drawn (ACC-VIEW-180-PROC-TREE), so no rule of its own
    // names a face.
    assert!(
        rule(".ptable .mono {").contains("font-family: var(--mono)"),
        ".ptable .mono does not keep the mono stack"
    );
    let (_, _, page) = get(&app, "/").await;
    let page = String::from_utf8(page).expect("utf-8 page");
    assert!(
        page.contains(r#"data-font="system""#),
        "the root element does not carry the default font choice"
    );
    let snapshot = fixture_snapshot();
    let first = snapshot.proc.entries.first().expect("a process row");
    let [page, fragment] = proc_documents(&snapshot).await;
    let pid = first.pid;
    for html in [&page, &fragment] {
        assert!(
            html.contains(&format!(r#"<td class="num">{pid}</td><td class="mono">"#)),
            "the Pid cell is not plain or the Program cell is not mono"
        );
        assert!(
            !html.contains(r#"<td class="num mono">"#),
            "a Pid cell keeps the mono stack"
        );
    }
}

/// ACC-THEME-154-NARROW-FONT, ACC-THEME-167-NARROW-ONLY: the narrow face is
/// the only Galaxa face served; it ships byte for byte, app.css carries its
/// @font-face and token, and the FONT control is the one rule that reads it.
#[tokio::test]
async fn narrow_font_is_served_with_its_own_font_face() {
    let app = app(Some(Snapshot::default()), true);
    let (status, headers, body) = get(&app, "/assets/fonts/GalaxaLabsTronNarrow-VF.ttf").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        content_type(&headers).starts_with("font/"),
        "{}",
        content_type(&headers)
    );
    let embedded = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/GalaxaLabsTronNarrow-VF.ttf"),
    )
    .expect("narrow font asset");
    assert_eq!(body, embedded, "the route serves the embedded bytes");

    let css = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.css"))
        .expect("app.css");
    let face = css
        .split("@font-face")
        .find(|block| block.contains("GalaxaLabsTron Narrow"))
        .expect("the narrow @font-face");
    for line in [
        "font-family: \"GalaxaLabsTron Narrow\";",
        "font-weight: 400 900;",
        "font-display: swap;",
        "src: url(\"fonts/GalaxaLabsTronNarrow-VF.ttf\") format(\"truetype\");",
    ] {
        assert!(face.contains(line), "the narrow @font-face lacks {line}");
    }
    let flat: String = css.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("--font-narrow: \"GalaxaLabsTron Narrow\", var(--ui);"),
        "app.css lacks the narrow font token"
    );
    assert_eq!(
        flat.matches("--font-narrow:").count(),
        1,
        "the narrow token must be defined once"
    );
    assert_eq!(
        flat.matches("var(--font-narrow)").count(),
        2,
        "a rule other than the FONT control reads the narrow face"
    );
    assert!(
        flat.contains(
            "[data-font=\"galaxatron\"] { --font-ui: var(--font-narrow); \
             --font-display: var(--font-narrow); \
             --track: calc(0.5px + 0.01em); letter-spacing: var(--track); }"
        ),
        "the FONT control does not select the narrow face"
    );
}

/// ACC-THEME-155-FONT-TOGGLE: the CONFIG general page carries a FONT row of
/// exactly two house chips, System first and on by default, and theme.js
/// stores the choice under its own key and writes the root attribute.
#[tokio::test]
async fn font_control_offers_system_and_galaxatron() {
    let (_, _, body) = get(&app(Some(Snapshot::default()), true), "/").await;
    let page = String::from_utf8(body).expect("utf-8 page");
    let start = page.find("data-group=\"font\"").expect("font row");
    let row = &page[start..start + page[start..].find("</div>").expect("row end")];
    assert!(row.contains(r#"role="radiogroup""#), "{row}");
    let values: Vec<&str> = row
        .match_indices("data-value=\"")
        .map(|(at, needle)| {
            let value = &row[at + needle.len()..];
            &value[..value.find('"').expect("chip end")]
        })
        .collect();
    assert_eq!(values, ["system", "galaxatron"], "{row}");
    assert!(
        row.contains(
            r#"class="opt on" type="button" role="radio" aria-checked="true" data-value="system""#
        ),
        "System is not the default option: {row}"
    );
    assert!(
        row.contains(
            r#"class="opt" type="button" role="radio" aria-checked="false" data-value="galaxatron""#
        ),
        "GalaxaTron is not offered: {row}"
    );
    let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/theme.js"))
        .expect("theme.js");
    for line in [
        "var FONT_KEY = \"btop-web-font\";",
        "root.setAttribute(\"data-font\", font);",
        "write(FONT_KEY, font);",
    ] {
        assert!(js.contains(line), "theme.js lacks {line}");
    }
}

/// ACC-THEME-155-FONT-TOGGLE: a browser that stored the removed pair's id
/// falls back to Default without an error; the picker's first option is
/// Default and theme.js rewrites the stored id before the first paint.
#[tokio::test]
async fn a_stored_galaxatron_theme_falls_back_to_default() {
    let (_, _, body) = get(&app(Some(Snapshot::default()), true), "/").await;
    let page = String::from_utf8(body).expect("utf-8 page");
    let start = page.find("id=\"theme-picker\"").expect("theme picker");
    let select = &page[start..start + page[start..].find("</select>").expect("closed picker")];
    assert!(
        select.contains(r#"<option value="default">Default</option>"#),
        "the picker's first option is not Default: {select}"
    );
    assert!(!select.contains("default-galaxatron"), "{select}");
    let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/theme.js"))
        .expect("theme.js");
    for line in [
        "if (theme === \"default-galaxatron\") {",
        "theme = \"default\";",
        "if (!known && picker.options.length > 0) {",
    ] {
        assert!(js.contains(line), "theme.js lacks the fallback: {line}");
    }
}

#[tokio::test]
async fn events_route_answers_with_an_event_stream() {
    let request = Request::builder()
        .uri("/events")
        .body(Body::empty())
        .expect("request");
    let response = app(Some(Snapshot::default()), true)
        .oneshot(request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(content_type(response.headers()).starts_with("text/event-stream"));
}

/// DEF-VIEW-119-STALE-TAB: a connection opens with this build's stamp and no
/// frame repeats it.
#[tokio::test]
async fn events_state_the_stamp_once_per_connection() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app(Some(fixture_snapshot()), true))
            .await
            .expect("serve");
    });
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    stream
        .write_all(format!("GET /events HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
        .await
        .expect("request");
    let text = read_until(&mut stream, "event: cpu", std::time::Duration::from_secs(5)).await;
    let stamp = format!(
        "event: stamp\ndata: {}\n\n",
        btop_web::routes::asset_stamp()
    );
    let at = text.find(&stamp).expect("no stamp in the stream");
    assert!(at < text.find("event: cpu").expect("cpu"));
    assert_eq!(text.matches("event: stamp").count(), 1);
    server.abort();
}

#[tokio::test]
async fn real_socket_serves_healthz() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app(None, true)).await.expect("serve");
    });

    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    stream
        .write_all(
            format!("GET /healthz HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("request");
    let mut response = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.read_to_string(&mut response),
    )
    .await
    .expect("response timed out")
    .expect("response");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("ok"), "{response}");

    server.abort();
}

/// ACC-CONF-61-CONFIG-PAGES: the page served by an app whose live
/// configuration and config file are `config` and `file`, as main builds it.
fn config_app(
    readonly: bool,
    config: Config,
    file: Option<PathBuf>,
) -> (Router, watch::Sender<Config>) {
    let (_sender, receiver) = watch::channel(Some(Arc::new(Frame::new(Snapshot::default()))));
    let mut state = AppState::new(receiver, readonly);
    state.config.send_replace(config);
    state.config_file = Arc::new(tokio::sync::Mutex::new(file));
    let live = state.config.clone();
    (btop_web::app(state), live)
}

async fn page_of(app: &Router) -> String {
    String::from_utf8(get(app, "/").await.2).expect("utf-8 page")
}

/// A btop-web.conf under the test target's temp dir; never a real
/// ~/.config and never a btop.conf.
fn temp_conf(name: &str, text: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("config-pages")
        .join(name);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("btop-web.conf");
    std::fs::write(&path, text).expect("temp config");
    path
}

const HOST: &str = "127.0.0.1:8888";
const ORIGIN: &str = "http://127.0.0.1:8888";

async fn post_config(app: &Router, body: &str, origin: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/config")
        .header(header::HOST, HOST)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        String::from_utf8(body.to_vec()).expect("utf-8 body"),
    )
}

/// ACC-NET-143-AGENT-MODES: a client applies no server option, so its config
/// menu carries no server group and the POST boundary refuses the write the
/// local agent's shared file would otherwise receive. The strip it does show
/// is not a tier field, and its poll and chip links are relative so they
/// resolve under a proxy's trailing-slash path.
#[tokio::test]
async fn client_mode_has_no_server_rows_and_refuses_config_posts() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("client-mode-config")
        .join("client");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("client dir");
    let key_file = dir.join("alpha.key");
    std::fs::write(&key_file, "client-mode-access-key\n").expect("key file");
    let connects = [Connect {
        name: "alpha".to_string(),
        addr: "127.0.0.1:1".to_string(),
    }];
    let keys = [ConnectKey {
        name: "alpha".to_string(),
        source: btop_web::secret::Source::Path(key_file),
    }];
    let client = btop_web::client::Client::start(&connects, &keys, &dir.join("known_agents"))
        .expect("client");
    let conf = temp_conf("client-mode", "update_ms = 250\n");
    let (_sender, receiver) = watch::channel(None);
    let mut state = AppState::new(receiver, false);
    state.client = Some(Arc::new(client));
    state.config_file = Arc::new(tokio::sync::Mutex::new(Some(conf.clone())));
    let app = btop_web::app(state);

    let page = page_of(&app).await;
    assert!(!page.contains("id=\"agents\" data-p="), "{page}");
    assert!(
        page.contains("hx-get=\"agents?agent=alpha\""),
        "the strip poll is not relative: {page}"
    );
    assert!(
        page.contains("href=\"?agent=alpha\" aria-current=\"page\""),
        "{page}"
    );
    assert!(
        !page.contains("server · all viewers"),
        "client menu offers server rows: {page}"
    );
    // ACC-VIEW-182-COPY-JSON / ACC-NET-181-JSON-STREAM: the client samples
    // nothing, so it has no frame to copy and no JSON stream to serve.
    assert!(
        !page.contains(r#"id="copy-json""#),
        "client mode offers the copy button"
    );
    assert_eq!(
        status(&app, Method::GET, "/events.json").await,
        StatusCode::NOT_FOUND,
        "client mode serves a local JSON stream"
    );

    // The boundary refuses the write; the shared file is untouched.
    let (status, body) = post_config(&app, "key=update_ms&value=500", Some(ORIGIN)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("client"), "{body}");
    assert_eq!(
        std::fs::read_to_string(&conf).expect("config"),
        "update_ms = 250\n"
    );
}

/// The opening tag and inner HTML of the element whose `role` is `role`.
fn roles<'a>(html: &'a str, role: &str) -> Vec<(&'a str, &'a str)> {
    elements_with(html, "role")
        .into_iter()
        .filter(|(tag, _)| attribute(tag, "role") == Some(role))
        .collect()
}

#[tokio::test]
async fn config_menu_is_a_tablist_of_seven_pages() {
    let page = page_of(&app(Some(Snapshot::default()), true)).await;
    assert_eq!(page.matches(r#"role="tablist""#).count(), 1);
    let tabs = roles(&page, "tab");
    let panels = roles(&page, "tabpanel");
    assert_eq!(
        tabs.iter().map(|(_, label)| *label).collect::<Vec<_>>(),
        CONFIG_PAGES
    );
    assert_eq!(panels.len(), CONFIG_PAGES.len());
    for (((tab, _), (panel, _)), id) in tabs.iter().zip(&panels).zip(CONFIG_PAGES) {
        let general = id == "general";
        assert_eq!(attribute(tab, "id"), Some(format!("cfg-tab-{id}").as_str()));
        assert_eq!(
            attribute(tab, "aria-controls"),
            Some(format!("cfg-page-{id}").as_str())
        );
        assert_eq!(
            attribute(tab, "aria-selected"),
            Some(if general { "true" } else { "false" })
        );
        assert_eq!(
            attribute(tab, "tabindex"),
            Some(if general { "0" } else { "-1" })
        );
        assert_eq!(
            attribute(panel, "id"),
            Some(format!("cfg-page-{id}").as_str())
        );
        assert_eq!(
            attribute(panel, "aria-labelledby"),
            Some(format!("cfg-tab-{id}").as_str())
        );
        assert_eq!(panel.contains(" hidden"), !general, "{panel}");
    }
}

/// ACC-VIEW-104-CONFIG-DIM: the page carries exactly one scrim element,
/// app.css blackens the page behind it in both modes, and app.js shows it
/// with the popover.
#[tokio::test]
async fn config_menu_dims_the_page_behind_it() {
    let page = page_of(&app(Some(Snapshot::default()), true)).await;
    assert_eq!(
        page.matches(r#"<div class="scrim" id="config-scrim""#)
            .count(),
        1,
        "the page lacks the single config scrim"
    );
    let asset = |name: &str| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join(name),
        )
        .expect(name)
    };
    let css = asset("app.css");
    let rule = |selector: &str| {
        let start = css
            .find(selector)
            .unwrap_or_else(|| panic!("app.css lacks {selector}"));
        let end = start + css[start..].find('}').expect("rule end");
        &css[start..end]
    };
    let scrim = rule(".scrim {");
    for declaration in [
        "position: fixed;",
        "inset: 0;",
        "z-index: 35;",
        "background: color-mix(in srgb, #000 35%, transparent);",
    ] {
        assert!(
            scrim.contains(declaration),
            "the scrim rule lacks {declaration}"
        );
    }
    assert!(
        rule(":root[data-mode=\"light\"] .scrim {")
            .contains("background: color-mix(in srgb, #000 25%, transparent);"),
        "the light-mode scrim does not blacken the page"
    );
    let js = asset("app.js");
    for hook in [
        r#"var scrim = document.getElementById("config-scrim");"#,
        "scrim.hidden = (!config || config.hidden) && detailPid === null;",
    ] {
        assert!(js.contains(hook), "app.js lacks {hook}");
    }
}

#[tokio::test]
async fn config_pages_hold_their_options() {
    let page = page_of(&app(Some(fixture_snapshot()), true)).await;
    let panels = roles(&page, "tabpanel");
    let inner = |id: &str| {
        panels
            .iter()
            .find(|(tag, _)| attribute(tag, "id") == Some(format!("cfg-page-{id}").as_str()))
            .map(|(_, inner)| *inner)
            .expect("page")
    };
    for option in SERVER_OPTIONS {
        let marker = format!(r#"data-key="{}""#, option.key);
        assert_eq!(page.matches(&marker).count(), 1, "{}", option.key);
        assert!(
            inner(option.page).contains(&marker),
            "{} on {}",
            option.key,
            option.page
        );
    }
    for (id, markers) in [
        (
            "general",
            &[
                r#"id="theme-picker""#,
                r#"data-group="mode""#,
                r#"id="pause-btn""#,
            ][..],
        ),
        ("gpu", &[r#"data-group="gpu""#]),
        (
            "proc",
            &[
                r#"data-group="proc-view""#,
                r#"data-group="proc-level""#,
                r#"id="proc-sort""#,
                r#"data-group="proc-dir""#,
            ],
        ),
        (
            "layout",
            &[r#"data-group="panels""#, r#"id="layout-reset""#],
        ),
        ("net", &[r#"network scale"#]),
    ] {
        for marker in markers {
            assert_eq!(page.matches(marker).count(), 1, "{marker}");
            assert!(inner(id).contains(marker), "{marker} on {id}");
        }
    }
    for id in ["cpu", "mem", "net"] {
        assert!(
            !inner(id).contains("view · this browser"),
            "{id} has only the server group"
        );
    }
    // ACC-GRAPH-96: the net page is the network scale section.
    for key in [
        "net_auto",
        "net_floor",
        "net_fall",
        "net_sync",
        "net_download",
        "net_upload",
    ] {
        assert!(
            inner("net").contains(&format!(r#"data-key="{key}""#)),
            "{key} is not on the net page"
        );
    }
    assert!(
        !inner("layout").contains("data-key"),
        "layout has only the view group"
    );
}

/// ACC-VIEW-108-NUM-MOTION: the general page's number row offers the five
/// effects as house-chip radios, each named plainly and titled with its
/// mechanism; none is on for a browser with no stored choice.
#[tokio::test]
async fn config_offers_the_number_animation_effects() {
    let page = String::from_utf8(get(&app(Some(Snapshot::default()), true), "/").await.2)
        .expect("utf-8 page");
    let config = &page[page.find(r#"id="config""#).expect("config popover")..];
    let (tag, inner) = elements_with(config, "data-group")
        .into_iter()
        .find(|(tag, _)| attribute(tag, "data-group") == Some("num-motion"))
        .expect("num-motion group");
    assert_eq!(attribute(tag, "role"), Some("radiogroup"));
    assert_eq!(
        attribute(tag, "aria-labelledby"),
        Some("cfg-num-motion-label")
    );
    assert!(
        config.contains(r#"<span class="cfg-label" id="cfg-num-motion-label""#),
        "the row has no label"
    );
    assert!(
        config.contains(">number</span>"),
        "the row is not named number"
    );
    let options = [
        ("none", "none", "none - instant update", true),
        ("spring", "spring up", "count up gradually", false),
        (
            "blur",
            "motion blur",
            "old values fade behind the current one",
            false,
        ),
        ("odometer", "odometer", "each changed digit rolls", false),
        (
            "spring-blur",
            "spring blur",
            "count up with fading echoes",
            false,
        ),
    ];
    assert_eq!(inner.matches("<button").count(), options.len(), "{inner}");
    for (value, label, title, on) in options {
        let class = if on { "opt on" } else { "opt" };
        let checked = if on { "true" } else { "false" };
        let marker = format!(
            r#"<button class="{class}" type="button" role="radio" aria-checked="{checked}" data-value="{value}" title="{title}">{label}</button>"#
        );
        assert!(
            inner.contains(&marker),
            "the number row lacks {value}: {inner}"
        );
    }
    assert_eq!(inner.matches("class=\"opt on\"").count(), 1, "{inner}");
}

/// ACC-VIEW-200-LIVE-STATUS: the general config page offers the live status as
/// house-chip radios, on by default; theme.js stores the choice and sets the
/// root attribute app.css keys the dimmed and off forms on.
#[tokio::test]
async fn config_offers_the_live_status_choices() {
    let page = String::from_utf8(get(&app(Some(Snapshot::default()), true), "/").await.2)
        .expect("utf-8 page");
    let start = page.find(r#"id="cfg-page-general""#).expect("general page");
    let config = page[start..]
        .split(r#"role="tabpanel""#)
        .next()
        .expect("general page");
    let (tag, inner) = elements_with(config, "data-group")
        .into_iter()
        .find(|(tag, _)| attribute(tag, "data-group") == Some("live-status"))
        .expect("live-status group");
    assert_eq!(attribute(tag, "role"), Some("radiogroup"));
    assert_eq!(
        attribute(tag, "aria-labelledby"),
        Some("cfg-live-status-label")
    );
    assert!(
        config.contains(r#"<span class="cfg-label" id="cfg-live-status-label">live status</span>"#),
        "the row is not named live status"
    );
    let options = [("on", true), ("dimmed", false), ("off", false)];
    assert_eq!(inner.matches("<button").count(), options.len(), "{inner}");
    for (value, on) in options {
        let class = if on { "opt on" } else { "opt" };
        let marker = format!(
            r#"<button class="{class}" type="button" role="radio" aria-checked="{on}" data-value="{value}">{value}</button>"#
        );
        assert!(inner.contains(&marker), "the row lacks {value}: {inner}");
    }
    let asset = |name: &str| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join(name),
        )
        .expect(name)
    };
    let theme = asset("theme.js");
    assert!(theme.contains(r#"root.setAttribute("data-live-status", liveStatus);"#));
    assert!(theme.contains(r#"write(LIVE_STATUS_KEY, liveStatus);"#));
    let css = asset("app.css");
    assert!(css.contains(r#"[data-live-status="dimmed"] .chip-live {"#));
    assert!(css.contains(r#"[data-live-status="off"] .chip-live {"#));
}

/// ACC-VIEW-108-NUM-MOTION: a change of effect never interrupts a transition
/// already in flight. That transition finishes under its own mechanism and the
/// new choice starts with the next value; only the motion toggle, pause and a
/// hidden tab stop one.
#[test]
fn a_number_mode_change_leaves_a_running_transition_to_finish() {
    let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.js"))
        .expect("app.js");
    assert!(
        !js.contains(r#"if (numMode === "none") cancelNumberRuns();"#),
        "a change of effect cancels a running transition"
    );
    assert!(
        js.contains(r#"if (!motionPref) cancelNumberRuns();"#),
        "the motion toggle does not stop a running transition"
    );
    assert!(
        js.contains(r#"if (stale || paused) cancelNumberRuns();"#),
        "pause does not stop a running transition"
    );
}

/// ACC-VIEW-120-ANIM-COMPLETE, ACC-VIEW-121-FLUID and
/// ACC-VIEW-126-RATE-THRESHOLD: the animated stat values carry their full
/// display range; a change under one percent of it starts no effect; and only
/// the spring effects retarget a running effect, while the odometer and the
/// blur trail hold the newest reading until their own end.
#[tokio::test]
async fn number_effects_retarget_continuous_motion_by_display_range() {
    let snapshot = fixture_snapshot();
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    assert!(
        page.contains(r#"data-range="100" id="cpu-total""#),
        "the CPU total does not name its 100 point display range"
    );
    let (value, unit) = snapshot.mem.ram_used_parts().expect("memory used");
    let memory = format!(
        r#"<span class="stat-value" data-w="0000" data-range="{}" id="mem-used">{value}</span><span class="stat-unit">{unit}</span>"#,
        snapshot.mem.ram_used_range()
    );
    assert!(
        page.contains(&memory),
        "the memory value does not name its display range"
    );
    let processes = format!(
        r#"data-range="{}" id="proc-total">{}</span>"#,
        snapshot.proc.chart_max,
        snapshot.proc.total.expect("process total")
    );
    assert!(
        page.contains(&processes),
        "the process total does not name its display range"
    );

    let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.js"))
        .expect("app.js");
    for marker in [
        "var NUM_RATE = 0.01;",
        "function numSignificant(el, before, after)",
        "if (!numSignificant(el, before, after)) return;",
        "if (run && run.retarget && previous.text !== text)",
        "numBegin(index, host, finish, retarget);",
        "numBegin(index, host, stop, null);",
    ] {
        assert!(js.contains(marker), "app.js lacks {marker}");
    }
    assert_eq!(
        js.matches("numBegin(index, host, stop, null);").count(),
        2,
        "the blur and the odometer are not both discrete"
    );
}

/// ACC-VIEW-203-SPRING-GRADUAL: the spring effects are critically damped, k = 100
/// and c = 20, and settle on their own 800 ms; their echoes and the discrete
/// effects keep numDuration(), and the option's title no longer names an overshoot.
#[tokio::test]
async fn spring_number_effects_move_gradually() {
    let page = page_of(&app(Some(fixture_snapshot()), true)).await;
    assert!(
        page.contains(r#"data-value="spring" title="count up gradually">spring up</button>"#),
        "the spring up option does not read count up gradually"
    );
    assert!(
        !page.contains("overshoot"),
        "the page still names an overshoot"
    );

    let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.js"))
        .expect("app.js");
    for marker in [
        "k = 100 and c = 20",
        "var NUM_SPRING_MS = 800;",
        "var ms = NUM_SPRING_MS;",
        "var omega = Math.sqrt(100);",
        "var t = Math.max(0, now - started) / 1000;",
        "var echoMs = numDuration();",
        "now - echoAt >= echoMs / 8",
        "Math.min(echoMs, ms - (now - started))",
    ] {
        assert!(js.contains(marker), "app.js lacks {marker}");
    }
    assert!(
        !js.contains("Math.sqrt(1600)"),
        "the old spring is still in app.js"
    );
    assert_eq!(
        js.matches("var ms = numDuration();").count(),
        2,
        "the blur and the odometer do not both keep numDuration()"
    );
}

/// DEF-VIEW-80-SEGMENT-FLIP: the segment effect is removed, so a browser that
/// stored "segment" keeps numMode at its "none" default: a stored value
/// applies only when NUM_MODES names it, and no effect layer is built.
#[test]
fn stored_segment_mode_falls_back_to_none() {
    let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.js"))
        .expect("app.js");
    assert!(
        js.contains(r#"var NUM_MODES = ["none", "spring", "blur", "odometer", "spring-blur"];"#),
        "NUM_MODES is not the five effects"
    );
    assert!(
        js.contains(r#"if (NUM_MODES.indexOf(numStored) !== -1) numMode = numStored;"#),
        "a stored mode is not checked against NUM_MODES"
    );
    assert!(
        !js.contains("segment"),
        "the segment effect is still in app.js"
    );
}

/// DEF-VIEW-81-ODOMETER-SMALL and ACC-VIEW-131-LOAD-WIDTH: an effect starts
/// only on the large stat values and on the load figures, which opt in with
/// data-num; every other smaller host switches instantly, so no copy at 12 px
/// builds a layer, and the two share one path at one duration.
#[test]
fn number_effects_are_gated_to_the_large_stat_values() {
    let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.js"))
        .expect("app.js");
    assert!(
        js.contains(
            r#"if (!el.classList.contains("stat-value") && !el.hasAttribute("data-num")) return;"#
        ),
        "playNumber has no stat-value or data-num gate"
    );
}

/// DEF-VIEW-132-DIGIT-SHIFT: the number effect layers keep their host's
/// letter-spacing, so a digit keeps its x while an effect runs.
#[test]
fn number_effect_layers_keep_the_host_letter_spacing() {
    let css = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.css"))
        .expect("app.css");
    let compact: String = css.split_whitespace().collect();
    assert!(
        compact.contains(
            ".num-tween,.num-base,.num-echo,.num-odo{font-variant-numeric:tabular-nums;white-space:pre;letter-spacing:inherit;}"
        ),
        "the number effect layers do not inherit the host's letter-spacing"
    );
}

/// ACC-VIEW-75-PANEL-MENU: one right-click menu of Off, S, M and L, after
/// `</main>`, so no swapped fragment holds it.
#[tokio::test]
async fn panel_menu_offers_off_and_the_three_sizes_outside_the_fragments() {
    let page = page_of(&app(Some(fixture_snapshot()), true)).await;
    let menus = roles(&page, "menu");
    let panel = menus
        .iter()
        .find(|(tag, _)| attribute(tag, "id") == Some("panel-menu"))
        .expect("panel menu");
    let sizes: Vec<_> = roles(panel.1, "menuitemradio")
        .iter()
        .map(|(tag, _)| attribute(tag, "data-size"))
        .collect();
    assert_eq!(sizes, [Some("off"), Some("s"), Some("m"), Some("l")]);
    assert!(page.find(r#"id="panel-menu""#) > page.find("</main>"));
}

/// ACC-VIEW-140-COLUMN-MENU: the process header's right-click menu lists
/// every optional column with its state and renders Program, which carries
/// the tree, disabled; the stylesheet hides a switched-off column with
/// .col-off.
#[tokio::test]
async fn proc_column_menu_lists_every_optional_column_and_pins_program() {
    let page = page_of(&app(Some(fixture_snapshot()), true)).await;
    let at = page
        .find(r#"<div class="panel-menu" id="proc-col-menu""#)
        .expect("column menu");
    let menu = &page[at..at + page[at..].find("</div>").expect("menu end") + "</div>".len()];
    let items = roles(menu, "menuitemcheckbox");
    let keys: Vec<_> = items
        .iter()
        .map(|(tag, _)| attribute(tag, "data-col"))
        .collect();
    assert_eq!(
        keys,
        [
            Some("pid"),
            Some("name"),
            Some("command"),
            Some("threads"),
            Some("user"),
            Some("mem"),
            Some("cpu")
        ]
    );
    let (program, _) = items[1];
    assert!(
        program.contains(" disabled") && program.contains(r#"aria-disabled="true""#),
        "the Program entry is not disabled: {program}"
    );
    assert!(
        page.find(r#"id="proc-col-menu""#) > page.find("</main>"),
        "the column menu sits inside a swapped fragment"
    );
    let css: String =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app.css"))
            .expect("app.css")
            .split_whitespace()
            .collect();
    assert!(
        css.contains(".col-off{display:none;}"),
        "app.css lacks the .col-off rule"
    );
}

#[tokio::test]
async fn read_only_config_shows_values_without_forms() {
    let mut config = Config::defaults();
    config.set("update_ms", "500").expect("valid");
    config.set("custom_cpu_name", "Lab CPU").expect("valid");
    let (app, _) = config_app(true, config.clone(), None);
    let page = page_of(&app).await;
    assert!(!page.contains("<form"));
    assert!(!page.contains("hx-post"));
    assert!(page.contains("read-only: restart without --readonly to change"));
    for option in SERVER_OPTIONS {
        let (_, row) = elements_with(&page, "data-key")
            .into_iter()
            .find(|(tag, _)| attribute(tag, "data-key") == Some(option.key))
            .expect("row");
        let value = config.get(option.key).expect("value");
        let shown = if value.is_empty() {
            "\"\"".to_string()
        } else {
            value
        };
        assert!(
            row.contains(&format!(r#"<span class="cfg-val">{shown}</span>"#)),
            "{row}"
        );
        assert!(
            !row.contains("<input") && !row.contains("<select") && !row.contains("<button"),
            "{row}"
        );
    }
}

#[tokio::test]
async fn read_write_config_rows_are_forms() {
    let path = temp_conf("forms", &default_config_text());
    let mut config = Config::defaults();
    config.set("update_ms", "500").expect("valid");
    let (app, _) = config_app(false, config, Some(path.clone()));
    let page = page_of(&app).await;
    assert!(page.contains(r#"<meta name="htmx-config" content='{"responseHandling":"#));
    assert!(page.contains(&format!(
        "changes apply to every viewer and are saved to {}",
        path.display()
    )));
    assert!(
        page.contains("$XDG_CONFIG_HOME/btop-web/ when set, else ~/.config/btop-web/"),
        "the note does not state where the path comes from"
    );
    for option in SERVER_OPTIONS {
        let (tag, row) = elements_with(&page, "data-key")
            .into_iter()
            .find(|(tag, _)| attribute(tag, "data-key") == Some(option.key))
            .expect("row");
        assert!(tag.starts_with(r#"<form class="cfg-key""#), "{tag}");
        // The server's parser answers an invalid value in the row; the
        // browser's constraint check would hold the request back silently.
        assert!(tag.contains(" novalidate "), "{tag}");
        assert_eq!(attribute(tag, "hx-post"), Some("config"));
        assert!(row.contains(&format!(
            r#"<input type="hidden" name="key" value="{}">"#,
            option.key
        )));
    }
    assert!(page.contains(r#"id="cfg-update_ms" value="500""#));

    let (app, _) = config_app(false, Config::defaults(), None);
    assert!(page_of(&app).await.contains(
        "changes apply to every viewer until restart; no btop-web.conf path: set HOME or XDG_CONFIG_HOME"
    ));
}

#[tokio::test]
async fn config_post_needs_the_host_origin() {
    let path = temp_conf("origin", &default_config_text());
    let (app, live) = config_app(false, Config::defaults(), Some(path.clone()));
    let before = std::fs::read(&path).expect("file");
    for origin in [None, Some("http://evil.example"), Some("null")] {
        let (status, body) = post_config(&app, "key=update_ms&value=1000", origin).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{origin:?}");
        assert_eq!(body, "cross-origin request refused");
        assert_eq!(std::fs::read(&path).expect("file"), before);
        assert_eq!(*live.borrow(), Config::defaults());
    }
    let (status, _) = post_config(&app, "key=update_ms&value=1000", Some(ORIGIN)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn config_post_refuses_invalid_values_and_other_keys() {
    let path = temp_conf("invalid", &default_config_text());
    let (app, live) = config_app(false, Config::defaults(), Some(path.clone()));
    let before = std::fs::read(&path).expect("file");
    for body in [
        "key=update_ms&value=50",
        "key=net_download&value=-1",
        "key=show_swap&value=yes",
        "key=custom_cpu_name&value=a%0Ab",
        "key=update_ms",
    ] {
        let (status, fragment) = post_config(&app, body, Some(ORIGIN)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(
            fragment.contains(r#"<output class="cfg-msg">refused: "#),
            "{fragment}"
        );
        assert!(
            fragment.starts_with("<form class=\"cfg-key\""),
            "{fragment}"
        );
        assert_eq!(std::fs::read(&path).expect("file"), before, "{body}");
        assert_eq!(*live.borrow(), Config::defaults(), "{body}");
    }
    let (_, fragment) = post_config(&app, "key=update_ms&value=50", Some(ORIGIN)).await;
    assert!(fragment.contains(r#"value="250""#), "{fragment}");
    for body in [
        "key=listen&value=0.0.0.0:1",
        "key=proc_root&value=/",
        "key=bogus&value=1",
        "key=show_io_stat&value=False",
        "value=1",
    ] {
        let (status, _) = post_config(&app, body, Some(ORIGIN)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(std::fs::read(&path).expect("file"), before, "{body}");
        assert_eq!(*live.borrow(), Config::defaults(), "{body}");
    }
}

/// ACC-CONF-147-OPTIONS-EDIT: read-only keeps every server row as text and
/// refuses a write with a stated reason; the file and the live configuration
/// stay as they were.
#[tokio::test]
async fn read_only_config_post_is_refused_with_a_reason() {
    let path = temp_conf("readonly-refusal", "update_ms = 250\n");
    let (app, live) = config_app(true, Config::defaults(), Some(path.clone()));
    for option in SERVER_OPTIONS {
        let body = format!("key={}&value=x", option.key);
        let (status, answer) = post_config(&app, &body, Some(ORIGIN)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{}", option.key);
        assert_eq!(answer, READ_ONLY_REFUSAL, "{}", option.key);
    }
    assert_eq!(
        std::fs::read_to_string(&path).expect("file"),
        "update_ms = 250\n"
    );
    assert_eq!(*live.borrow(), Config::defaults());
}

/// ACC-CONF-147/148: a config POST rewrites only the changed key's line of
/// btop-web.conf, keeps a key the page did not touch, appends a key the file
/// lacks and creates a missing file with 0600.
#[tokio::test]
async fn config_post_writes_one_line_of_the_web_file() {
    let path = temp_conf("write", "update_ms = 250\ncustom_unknown = keep\n");
    let (app, live) = config_app(false, Config::defaults(), Some(path.clone()));
    let (status, fragment) = post_config(&app, "key=update_ms&value=1000", Some(ORIGIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(fragment.contains(r#"value="1000""#), "{fragment}");
    assert!(fragment.contains(&format!(
        "saved to {}",
        std::fs::canonicalize(&path).expect("path").display()
    )));
    assert_eq!(
        std::fs::read_to_string(&path).expect("file"),
        "update_ms = 1000\ncustom_unknown = keep\n",
        "the untouched key and every other line stay"
    );
    assert_eq!(live.borrow().update_ms, 1000);

    // A key the file lacks is appended.
    let path = temp_conf("append", "update_ms = 250\n");
    let (app, live) = config_app(false, Config::defaults(), Some(path.clone()));
    let (status, _) = post_config(&app, "key=custom_cpu_name&value=Lab+CPU", Some(ORIGIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(&path).expect("file"),
        "update_ms = 250\ncustom_cpu_name = \"Lab CPU\"\n"
    );
    assert_eq!(live.borrow().custom_cpu_name, "Lab CPU");

    // A web file that does not exist yet is created with 0600.
    use std::os::unix::fs::PermissionsExt;
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("config-pages/new-file");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("btop-web.conf");
    let (app, live) = config_app(false, Config::defaults(), Some(path.clone()));
    let (status, _) = post_config(&app, "key=show_swap&value=False", Some(ORIGIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(&path).expect("file"),
        "show_swap = False\n"
    );
    assert_eq!(
        std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "a created btop-web.conf is private"
    );
    assert!(!live.borrow().show_swap);

    // A symlinked path keeps its link; the file it names changes.
    let target = temp_conf("symlink", "show_swap = True\n");
    let link = target.with_file_name("link.conf");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let (app, _) = config_app(false, Config::defaults(), Some(link.clone()));
    let (status, _) = post_config(&app, "key=show_swap&value=False", Some(ORIGIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("file"),
        "show_swap = False\n"
    );
}

/// `value` as an `application/x-www-form-urlencoded` form value.
fn form_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// The valid value this test writes through `option`'s own control: the
/// opposite bool, the next int, a different chip or select choice, or a
/// string that passes that key's parser.
fn control_value(option: &btop_web::config::ServerOption, current: &str) -> String {
    match option.control {
        Control::Bool => (if current == "True" { "False" } else { "True" }).to_string(),
        Control::Int { min, max } => {
            let current: i64 = current.parse().expect("an int value");
            let next = if current < max {
                current + 1
            } else {
                current - 1
            };
            assert!((min..=max).contains(&next), "{}: {next}", option.key);
            next.to_string()
        }
        Control::Chips(choices) | Control::Choice(choices) => {
            let choices: Vec<&str> = choices.iter().map(|(value, _)| *value).collect();
            choices
                .into_iter()
                .find(|value| *value != current)
                .expect("a second choice")
                .to_string()
        }
        Control::Text if option.key == "cpu_core_map" => "4:0 5:1".to_string(),
        Control::Text => "web edit".to_string(),
    }
}

/// ACC-CONF-147-OPTIONS-EDIT and ACC-CONF-148-OWN-FILE: every option the
/// server page lists accepts a value through its own control, gets exactly
/// its line in btop-web.conf, reaches the running config, and a restart with
/// btop.conf as the baseline reads every value back; btop.conf is untouched.
#[tokio::test]
async fn every_server_option_writes_its_line_and_overrides_the_baseline_at_restart() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("config-pages/all-keys");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    // btop.conf holds only the baseline interval; every other key the web
    // file writes is absent from it.
    let base = dir.join("btop.conf");
    std::fs::write(&base, "update_ms = 250\n").expect("baseline");
    let base_before = std::fs::read(&base).expect("baseline bytes");
    let web = dir.join("btop-web.conf");
    let mut config = Config::defaults();
    let (app, live) = config_app(false, config.clone(), Some(web.clone()));
    for option in SERVER_OPTIONS {
        let current = config.get(option.key).expect("a value");
        let value = control_value(&option, &current);
        let line = config.set(option.key, &value).expect(option.key);
        let body = format!("key={}&value={}", option.key, form_encode(&value));
        let (status, fragment) = post_config(&app, &body, Some(ORIGIN)).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{} = {value}: {fragment}",
            option.key
        );
        assert!(fragment.contains("saved to"), "{}: {fragment}", option.key);
        assert_eq!(
            *live.borrow(),
            config,
            "{} did not reach the running config",
            option.key
        );
        let written = std::fs::read_to_string(&web).expect("web file");
        assert!(
            written.contains(&format!("{line}\n")),
            "{} lacks `{line}`: {written}",
            option.key
        );
    }
    assert_eq!(
        std::fs::read(&base).expect("baseline"),
        base_before,
        "btop.conf changed"
    );
    // A restart applies the web file over the btop.conf baseline.
    let restarted = Config::load_with_web(Some(&base), Some(&web)).expect("restart");
    for option in SERVER_OPTIONS {
        assert_eq!(
            config.get(option.key),
            restarted.get(option.key),
            "{} did not survive a restart",
            option.key
        );
    }
    assert_ne!(
        restarted.update_ms, 250,
        "the web file did not override the baseline"
    );
}

/// ACC-CONF-147-OPTIONS-EDIT: a write that cannot reach the file leaves the
/// running config at its old value and says so in the row.
#[tokio::test]
async fn a_failed_config_write_keeps_the_running_config_and_says_so() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("config-pages/failed-write");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let blocker = dir.join("blocked");
    std::fs::write(&blocker, "a file, not a directory").expect("blocker");
    let path = blocker.join("btop-web.conf");
    let mut config = Config::defaults();
    config.set("update_ms", "500").expect("valid");
    let (app, live) = config_app(false, config.clone(), Some(path));
    let (status, fragment) = post_config(&app, "key=update_ms&value=1000", Some(ORIGIN)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{fragment}");
    assert!(fragment.contains("refused: could not write"), "{fragment}");
    assert!(fragment.contains("nothing changed"), "{fragment}");
    assert!(
        fragment.contains(r#"value="500""#),
        "the row does not show the running value: {fragment}"
    );
    assert_eq!(*live.borrow(), config);
    let page = page_of(&app).await;
    assert!(page.contains(r#"id="cfg-update_ms" value="500""#), "{page}");
}

#[tokio::test]
async fn config_post_without_a_file_applies_until_restart() {
    let (app, live) = config_app(false, Config::defaults(), None);
    let (status, fragment) = post_config(&app, "key=update_ms&value=1000", Some(ORIGIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        fragment.contains("applied until restart; no btop-web.conf path"),
        "{fragment}"
    );
    assert_eq!(live.borrow().update_ms, 1000);
    assert!(
        page_of(&app).await.contains(">1000 ms<"),
        "a new page load shows the live interval"
    );
}

/// ACC-GRAPH-96: every network scale setting reaches the server through the
/// same config POST, changes the live config, is written to the file and a
/// reloaded page shows it: the auto flag, the floor, the fall window, the
/// sync flag and the two manual ceilings.
#[tokio::test]
async fn network_scale_rows_persist_through_the_config_path() {
    let path = temp_conf("network-scale", &default_config_text());
    let (app, live) = config_app(false, Config::defaults(), Some(path.clone()));
    for (body, key, value) in [
        ("key=net_auto&value=false", "net_auto", "False"),
        ("key=net_floor&value=2", "net_floor", "2"),
        ("key=net_fall&value=60", "net_fall", "60"),
        ("key=net_sync&value=false", "net_sync", "False"),
        ("key=net_download&value=50", "net_download", "50"),
        ("key=net_upload&value=20", "net_upload", "20"),
    ] {
        let (status, fragment) = post_config(&app, body, Some(ORIGIN)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(live.borrow().get(key).as_deref(), Some(value), "{body}");
        assert!(fragment.contains("saved to"), "{fragment}");
    }
    let written = std::fs::read_to_string(&path).expect("file");
    for line in [
        "net_auto = False",
        "net_floor = 2",
        "net_fall = 60",
        "net_sync = False",
        "net_download = 50",
        "net_upload = 20",
    ] {
        assert!(written.contains(&format!("{line}\n")), "{written}");
    }
    let page = page_of(&app).await;
    for marker in [
        r#"id="cfg-net_auto-off" aria-pressed="true""#,
        r#"id="cfg-net_floor" value="2""#,
        r#"id="cfg-net_fall" value="60""#,
        r#"id="cfg-net_sync-off" aria-pressed="true""#,
        r#"id="cfg-net_download" value="50""#,
        r#"id="cfg-net_upload" value="20""#,
    ] {
        assert!(page.contains(marker), "the reloaded page lacks {marker}");
    }
    assert!(page.contains("network scale · all viewers"));
}

/// ACC-PERF-149-SPLIT-CADENCE: the CONFIG proc page carries the process
/// refresh row as a house-chip group of 250, 500, 1000, 2000 and 5000 ms,
/// default 1000. A chosen value reaches the live config, is written to the
/// file, and a read-only server shows the value and refuses the write.
#[tokio::test]
async fn proc_refresh_row_is_a_house_chip_group_that_persists() {
    let path = temp_conf("proc-refresh", &default_config_text());
    let (app, live) = config_app(false, Config::defaults(), Some(path.clone()));
    let page = page_of(&app).await;
    let (tag, row) = elements_with(&page, "data-key")
        .into_iter()
        .find(|(tag, _)| attribute(tag, "data-key") == Some("proc_refresh_ms"))
        .expect("the process refresh row");
    assert!(tag.starts_with(r#"<form class="cfg-key""#), "{tag}");
    for value in ["250", "500", "1000", "2000", "5000"] {
        assert!(
            row.contains(&format!(r#"type="submit" name="value" value="{value}""#)),
            "the {value} chip is missing: {row}"
        );
    }
    assert!(
        row.contains(r#"value="1000" aria-pressed="true""#),
        "1000 is not the default chip: {row}"
    );
    assert!(
        !row.contains("<select") && !row.contains(r#"type="number""#),
        "the refresh row is not a chip group: {row}"
    );

    let (status, fragment) =
        post_config(&app, "key=proc_refresh_ms&value=2000", Some(ORIGIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(live.borrow().proc_refresh_ms, 2000);
    assert!(
        fragment.contains(r#"value="2000" aria-pressed="true""#),
        "{fragment}"
    );
    assert!(
        std::fs::read_to_string(&path)
            .expect("file")
            .contains("proc_refresh_ms = 2000\n"),
        "the chosen period was not written"
    );
    assert!(
        page_of(&app)
            .await
            .contains(r#"value="2000" aria-pressed="true""#),
        "the reloaded page lacks the saved period"
    );

    // A read-only server shows the value and offers no control to change it.
    let (app, _) = config_app(true, live.borrow().clone(), None);
    let page = page_of(&app).await;
    let (tag, row) = elements_with(&page, "data-key")
        .into_iter()
        .find(|(tag, _)| attribute(tag, "data-key") == Some("proc_refresh_ms"))
        .expect("the process refresh row");
    assert!(!tag.contains("<form") && !row.contains("<button"), "{row}");
    assert!(
        row.contains(r#"<span class="cfg-val">2000</span>"#),
        "{row}"
    );
    let (status, _) = post_config(&app, "key=proc_refresh_ms&value=250", Some(ORIGIN)).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a read-only server wrote the refresh period"
    );
}

/// ACC-PERF-142-WAKE-FAST: a sleeping server answers the page with a first
/// frame under 50 ms that runs no slow collector: the fast values are current
/// and the held walk and ring ride along, while the GPU values follow in a
/// later event. The tree is recorded under `tests/fixtures`, never the host:
/// `/proc` and the recorded AMD card's `/sys` copied side by side, which is
/// how [`btop_web::collector::sys_root`] pairs them.
#[tokio::test]
async fn waking_from_sleep_sends_the_first_frame_fast() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("wake-fast");
    let _ = std::fs::remove_dir_all(&base);
    copy_tree(&fixture("proc"), &base.join("proc"));
    copy_tree(&fixture("gpu/sys"), &base.join("sys"));
    let mut config = Config::defaults();
    config.proc_root = base.join("proc");
    config.update_ms = 250;
    let (updates, receiver) = watch::channel(config);
    let sampler = btop_web::collector::spawn(receiver);
    let mut state = AppState::new(sampler.snapshots.clone(), true);
    state.wake = Arc::clone(&sampler.wake);
    state.config = updates;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        axum::serve(listener, btop_web::app(state))
            .await
            .expect("serve");
    });

    // Warm up: the page's first tick runs the slow pass, then the sampler
    // parks, holding no timer.
    let (_, warm) = timed_page(address).await;
    assert!(warm.contains("GPU metrics unavailable"), "{warm}");
    let warm_held = sampler.snapshots.borrow().clone().expect("the warm frame");
    let warm_name = warm_held
        .snapshot
        .proc
        .entries
        .iter()
        .find(|entry| entry.pid == 222)
        .and_then(|entry| entry.name.clone())
        .expect("pid 222's name");
    let mut frames = sampler.snapshots.clone();
    frames.borrow_and_update();
    tokio::time::timeout(std::time::Duration::from_millis(600), frames.changed())
        .await
        .expect_err("the sampler stayed awake while nothing was connected");

    // A process and 100 user plus 100 idle CPU ticks appear while the server
    // sleeps. The first post-wake frame reports the new CPU value but reuses
    // the process walk it already held, so it does not list the new pid: no
    // slow collector ran. The next slow pass picks both up.
    bump_cpu_stat(&base.join("proc/stat"));
    copy_tree(&base.join("proc/222"), &base.join("proc/999"));

    let (first_frame, page) = timed_page(address).await;
    println!("first frame after a sleep: {first_frame:?}");
    assert!(
        first_frame < std::time::Duration::from_millis(50),
        "the first frame after a sleep took {first_frame:?}"
    );
    assert!(
        page.contains(r#"data-slow-ms="1000" data-slow-seq="1""#),
        "the first snapshot ran the slow pass"
    );
    assert!(
        page.contains(r#"id="cpu-total">50</span>"#),
        "the first frame lacks the CPU value: {page}"
    );
    let mem_used = r#"id="mem-used">"#;
    let at = page.find(mem_used).expect("the first frame lacks mem-used") + mem_used.len();
    assert!(
        page[at..].starts_with(|value: char| value.is_ascii_digit()),
        "the first frame's memory value is unavailable"
    );
    // The held walk rides in the first frame, so the pid that appeared while
    // the server slept is not invented by a slow pass on the wake tick.
    assert!(
        page.contains(r#"<td class="num">222</td>"#),
        "the first frame lost the held process walk"
    );
    assert!(
        !page.contains(r#"<td class="num">999</td>"#),
        "the first frame rebuilt the process walk"
    );
    // The sweep has not landed: the GPU panel shows its unavailable state,
    // never a zero.
    assert!(
        page.contains(r#"<p class="unavailable">GPU metrics unavailable</p>"#),
        "the first frame's GPU panel is not unavailable"
    );
    assert!(
        !page.contains("AMD GPU") && !page.contains(r#"data-gpu="0""#),
        "the first frame invented a GPU value"
    );

    // The GPU values follow in a later event of the same stream: the held
    // frame's gpu event carries the unavailable state, a later one the
    // recorded card, and the slow pass that lands with it lists the new pid.
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    stream
        .write_all(format!("GET /events HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
        .await
        .expect("request");
    // read_until returns at the first "AMD GPU", which can fall inside the
    // values event; the next frame's gpu fragment opens only after this one
    // closes, so a second read completes the first. The fragment order is
    // cpu, mem, net, proc, gpu, so the slow pass's 999 row has arrived by the
    // first match.
    let mut streamed = read_until(&mut stream, "AMD GPU", std::time::Duration::from_secs(5)).await;
    streamed.push_str(&read_until(&mut stream, "AMD GPU", std::time::Duration::from_secs(5)).await);
    let unavailable = streamed
        .find("GPU metrics unavailable")
        .unwrap_or_else(|| panic!("no first gpu event in {}", tail(&streamed)));
    let values = streamed
        .find("AMD GPU")
        .unwrap_or_else(|| panic!("no gpu event with values in {}", tail(&streamed)));
    assert!(
        unavailable < values,
        "the GPU values arrived before the unavailable state"
    );
    assert!(
        streamed.contains("37%"),
        "the gpu event lacks the recorded utilisation: {}",
        tail(&streamed)
    );
    assert!(
        streamed.contains(r#"<td class="num">999</td>"#),
        "the slow pass after the wake did not list the new pid: {}",
        tail(&streamed)
    );
    // The slow pass after the sleep read pid 222's identity from the same
    // cache entry, not a new string: the LRU survives the sleep.
    let after = sampler.snapshots.borrow().clone().expect("the slow frame");
    let after_name = after
        .snapshot
        .proc
        .entries
        .iter()
        .find(|entry| entry.pid == 222)
        .and_then(|entry| entry.name.clone())
        .expect("pid 222's name");
    assert!(
        Arc::ptr_eq(&warm_name, &after_name),
        "the identity cache did not survive the sleep"
    );

    server.abort();
    sampler.task.abort();
    let _ = std::fs::remove_dir_all(&base);
}

/// A recorded fixture tree by its path under `tests/fixtures`.
fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

/// ACC-VIEW-176-PROC-POPUP: the popup body on a recorded tree. Pid 500 holds a
/// render node and has a 1516-byte command line and an io file; pid 501 has
/// none of those. A pid the frame does not list is a 404 and a pid that is not
/// a number a 400. The rings fill on the collector's pass, not on the request.
#[tokio::test]
async fn proc_detail_serves_one_process_from_the_recorded_tree() {
    const TREE: &str = "tests/fixtures/proc-gpu/proc";
    let mut config = Config::load(None).expect("config");
    config.proc_root = Path::new(env!("CARGO_MANIFEST_DIR")).join(TREE);
    let snapshot = recorded_snapshot_with(TREE, 2, config.clone());
    let (_sender, receiver) = watch::channel(Some(Arc::new(Frame::new(snapshot))));
    let mut state = AppState::new(receiver, true);
    state.config = watch::Sender::new(config.clone());
    let app = btop_web::app(state);

    let (code, headers, body) = get(&app, "/proc/500").await;
    assert_eq!(code, StatusCode::OK);
    assert!(content_type(&headers).starts_with("text/html"));
    let first = String::from_utf8(body).expect("utf-8");
    assert!(first.contains("process 500"), "{first}");
    assert!(first.contains("5.00 MiB"), "io read: {first}");
    assert!(first.contains("1.00 MiB"), "io write: {first}");
    assert!(first.contains("/dev/dri/renderD128"), "{first}");
    assert!(!first.contains("/tmp/not-a-gpu"), "{first}");
    for chart in ["proc-detail-cpu", "proc-detail-mem", "proc-detail-gpu"] {
        assert!(
            first.contains(&format!(r#"data-chart="{chart}""#)),
            "{chart}: {first}"
        );
    }
    // DEF-GRAPH-129-PROC-PLOT: the first body draws the passes the sampler has
    // already run, as the GPU plot draws the panel's series.
    for chart in ["proc-detail-cpu", "proc-detail-mem"] {
        let plot = &first[first.find(chart).expect("plot")..];
        assert!(
            plot[..plot.find("</svg>").expect("a first curve")].contains(r#"class="line" d="M"#),
            "{chart}: {first}"
        );
    }
    assert!(
        first.contains(r#"class="gpu-series g0"><path class="area""#),
        "{first}"
    );
    // ACC-VIEW-202-PLOT-LABELS: each plot has the Network panel's scale gutter
    // in its grid, and the head has no range text left.
    assert_eq!(
        first.matches(r#"class="net-plot-row""#).count(),
        3,
        "{first}"
    );
    assert_eq!(first.matches(r#"class="plot-scale""#).count(), 3, "{first}");
    assert!(
        first.contains(r#"<span class="scale-total">1%</span><span class="scale-mid" style="--at:50">0.5%</span><span class="scale-zero">0%</span>"#),
        "{first}"
    );
    assert!(
        first.contains(r#"<span class="scale-zero">0 B</span>"#),
        "{first}"
    );
    assert!(
        first.contains(r#"<span class="scale-total">100%</span><span class="scale-mid" style="--at:50">50%</span>"#),
        "{first}"
    );
    assert!(!first.contains("&ndash;"), "a range text: {first}");
    assert!(!first.contains("since the popup opened"), "{first}");
    assert!(
        first.contains(r#"<span class="pd-scale">device load</span>"#),
        "{first}"
    );
    // DEF-GRAPH-130-PLOT-STEPS: the body and the GPU plot, which comes from the
    // frame, each say which slow pass they hold.
    assert!(
        first.contains(r#"class="pd-live" data-slow-seq=""#),
        "{first}"
    );
    assert!(
        first.contains(r#"data-chart="proc-detail-gpu" data-slow-seq="1""#),
        "{first}"
    );

    // DEF-VIEW-115-MEM-TIP, DEF-VIEW-123-POPUP-TIP: the three facts state the
    // process's own figure, and memory names stat before statm.
    assert_eq!(
        first
            .matches("a collapsed row in the table sums its subtree")
            .count(),
        3,
        "{first}"
    );
    assert!(first.contains("stat field 24 × page size"), "{first}");
    assert!(!first.contains("the table's Cpu%"), "{first}");
    // DEF-VIEW-124-POPUP-CHIPS: the section labels are plain text; COPY stays a
    // chip.
    assert!(!first.contains("chip-fact"), "{first}");
    for label in ["CPU", "Memory", "GPU 0", "command"] {
        assert!(
            first.contains(&format!(r#"<span class="pd-label">{label}</span>"#)),
            "{label}: {first}"
        );
    }
    assert!(first.contains(r#"class="chip chip-copy pd-copy up">copy<"#));

    // The whole command line, past btop's 1000 bytes, as text.
    assert!(first.contains("tail-marker.usd"), "{first}");
    assert!(!first.contains("<b>night"), "an unescaped command: {first}");

    proc::collect(&config.proc_root, &config);
    let next = String::from_utf8(get(&app, "/proc/500").await.2).expect("utf-8");
    let line = |chart: &str| {
        let plot = &next[next.find(chart).expect("plot")..];
        plot[..plot.find("</svg>").expect("svg")].contains(r#"class="line" d="M"#)
    };
    assert!(line("proc-detail-cpu"), "{next}");
    assert!(line("proc-detail-mem"), "{next}");
    // Axis and the unchanged page words.
    assert!(next.contains(">now</span>"), "{next}");

    let plain = String::from_utf8(get(&app, "/proc/501").await.2).expect("utf-8");
    assert!(plain.contains("plainproc --idle"), "{plain}");
    assert!(!plain.contains("proc-detail-gpu"), "{plain}");
    assert!(plain.contains("<span>gpu</span><b>none</b>"), "{plain}");
    assert!(
        plain.contains("<span>io read</span><b>unavailable</b>"),
        "{plain}"
    );

    assert_eq!(
        status(&app, Method::GET, "/proc/99999").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(&app, Method::GET, "/proc/abc").await,
        StatusCode::BAD_REQUEST
    );
}

/// ACC-VIEW-176-PROC-POPUP: the page carries the popup's frame, hidden, outside
/// every swapped panel, and app.js and app.css carry its hooks.
#[tokio::test]
async fn the_page_carries_the_process_popup() {
    let app = app(Some(fixture_snapshot()), true);
    let page = String::from_utf8(get(&app, "/").await.2).expect("utf-8");
    let main_end = page.find("</main>").expect("main");
    let popup = page.find(r#"id="proc-detail""#).expect("the popup frame");
    assert!(popup > main_end, "a swapped fragment could hold the popup");
    assert!(page[popup..].contains(r#"role="dialog""#));
    assert!(page[popup..].contains(r#"id="proc-detail-close""#));
    let js = String::from_utf8(get(&app, "/assets/app.js").await.2).expect("utf-8");
    assert!(
        js.contains(r#""proc/" + pid + window.location.search"#),
        "a relative route"
    );
    let css = String::from_utf8(get(&app, "/assets/app.css").await.2).expect("utf-8");
    assert!(
        css.contains(".proc-detail[hidden]"),
        "a hidden popup would still show"
    );
}

/// Copy a recorded tree into a private directory, so a test that changes its
/// files never touches `tests/fixtures`.
fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).expect("tree");
    for entry in std::fs::read_dir(source).expect("fixture tree") {
        let entry = entry.expect("entry");
        let destination = target.join(entry.file_name());
        let path = entry.path();
        if entry.file_type().expect("entry type").is_symlink() {
            // The recorded sys tree keeps driver links whose targets are not
            // recorded; the collector reads the link itself.
            std::os::unix::fs::symlink(std::fs::read_link(&path).expect("link"), &destination)
                .expect("link");
        } else if path.is_dir() {
            copy_tree(&path, &destination);
        } else {
            std::fs::copy(&path, &destination).expect("fixture file");
        }
    }
}

/// The page over a real socket, timed from the request to the last byte: the
/// frame a viewer receives.
async fn timed_page(address: std::net::SocketAddr) -> (std::time::Duration, String) {
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    let at = std::time::Instant::now();
    stream
        .write_all(
            format!("GET / HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await
        .expect("request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("page");
    (
        at.elapsed(),
        String::from_utf8(response).expect("utf-8 page"),
    )
}

/// The response text read until `needle` appears or `timeout` expires.
async fn read_until(
    stream: &mut tokio::net::TcpStream,
    needle: &str,
    timeout: std::time::Duration,
) -> String {
    let mut text = String::new();
    let mut buffer = [0u8; 8192];
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, stream.read(&mut buffer)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return text,
            Ok(Ok(read)) => {
                text.push_str(&String::from_utf8_lossy(&buffer[..read]));
                if text.contains(needle) {
                    return text;
                }
            }
        }
    }
}

/// The last of `text`, for an assertion message that stays readable.
fn tail(text: &str) -> &str {
    let mut start = text.len().saturating_sub(2000);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Add 100 user and 100 idle ticks to the recorded `cpu` line, so the next
/// sample reads 50 percent without the host's counters.
fn bump_cpu_stat(path: &Path) {
    // The recorded files are read-only; the private copy is the test's own.
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).expect("stat").permissions().mode();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o200))
        .expect("stat permissions");
    let text = std::fs::read_to_string(path).expect("stat");
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() == Some("cpu") {
            let mut times: Vec<u64> = fields.map(|value| value.parse().expect("tick")).collect();
            times[0] += 100;
            times[3] += 100;
            out.push_str(&format!(
                "cpu {}",
                times
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    std::fs::write(path, out).expect("stat");
}

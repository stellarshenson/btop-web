//! Router integration: in-process `oneshot` requests for every route and one
//! real socket, so the axum/hyper path is exercised at least once.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use btop_web::AppState;
use btop_web::chart::{ChartPath, GPU_PLOT, PROC_GRAPH, gauge_arc};
use btop_web::collector::{
    CoreUsage, CpuSnapshot, GpuDevice, MemSnapshot, Snapshot, cpu, gpu, mem, net, proc,
};
use btop_web::config::{CONFIG_PAGES, Config, SERVER_OPTIONS, default_config_text};
use btop_web::routes::READ_ONLY_REFUSAL;
use btop_web::routes::dashboard::PANELS;
use btop_web::routes::events::fragments;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;
use tower::ServiceExt;

fn app(snapshot: Option<Snapshot>, readonly: bool) -> Router {
    let (_sender, receiver) = watch::channel(snapshot);
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
        gpu::attach_charts(&root, &mut gpu.devices, config.update_ms);
        snapshot = Snapshot {
            cpu: cpu::collect(&root, &config),
            mem: mem.collect(&root, &config),
            net: net::collect(&root, &config),
            proc: proc::collect(&root, &config),
            gpu,
            update_ms: config.update_ms,
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
            GPU_PLOT.width,
            GPU_PLOT.height,
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
    ] {
        assert!(
            page.contains(&format!("value=\"{theme}\"")),
            "theme picker lacks {theme}"
        );
    }
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
        "href=\"assets/app.css\"",
        "src=\"assets/htmx.min.js\"",
        "src=\"assets/htmx-sse.min.js\"",
        "src=\"assets/theme.js\"",
        "src=\"assets/app.js\"",
        "sse-connect=\"events\"",
    ] {
        assert!(page.contains(marker), "page lacks {marker}");
    }
    for absolute in ["\"/assets", "\"/events"] {
        assert!(
            !page.contains(absolute),
            "page carries the absolute reference {absolute}"
        );
    }
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
    }
    assert_eq!(
        page.matches(r#"data-state="unavailable""#).count(),
        2,
        "the CPU and memory gauges are the meters the empty tree leaves"
    );
    assert!(
        page.contains(r#"<div class="mem-row swap"><span>swap</span><b>unavailable</b>"#),
        "the unavailable swap row does not say so"
    );
    // data-w="100%" is the reserved width of a percent cell, not a value
    // (ACC-LAYOUT-74-RESERVED-WIDTH).
    assert!(
        !page.replace(r#" data-w="100%""#, "").contains("0%"),
        "an unavailable value rendered as zero"
    );
    assert!(
        !page.contains("--value:0"),
        "an unavailable meter rendered a zero fill"
    );
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
        page.contains(r#"<div class="mem-row swap"><span>swap</span><b>unavailable</b>"#),
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
        r#"class="live-word up" aria-live="polite" data-w="paused">live<"#,
        "READ-ONLY",
        "config-toggle",
    ] {
        assert!(bar.contains(marker), "top bar lacks {marker}");
    }
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

/// ACC-GRAPH-67-SMOOTH-SCALE: each network plot draws its area and line in
/// one series group, which a scale change transforms.
#[tokio::test]
async fn network_plots_draw_in_a_series_group() {
    let snapshot = fixture_snapshot();
    assert!(!snapshot.net.down_chart.line.is_empty());
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
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
    let entries = &snapshot.proc.entries;
    for html in proc_documents(&snapshot).await {
        assert_eq!(html.matches(">Cpu%</button></th>").count(), 1);
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
                cpu_cell.contains(r#"<svg data-chart="proc" viewBox="0 0 10 12""#),
                has_own
            );
            assert_eq!(
                cpu_cell.contains(r#"<svg class="sum" data-chart="proc" viewBox="0 0 10 12""#),
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
            "Pid", "Program", "Command", "Threads", "User", "MemB", "Cpu%",
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

/// ACC-CONF-63-TITLE-FLAG: the page title is btop-web unless set, and a set
/// title is HTML-escaped.
#[tokio::test]
async fn title_flag_text_is_escaped() {
    let titled = {
        let (_sender, receiver) = watch::channel(Some(Snapshot::default()));
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
        let (_sender, receiver) = watch::channel(Some(Snapshot::default()));
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

/// ACC-VIEW-58-PROC-VIEW: the config menu offers the three views and the
/// tree-node option as house-chip radios.
#[tokio::test]
async fn config_offers_process_view_and_tree_nodes() {
    let page = String::from_utf8(get(&app(Some(Snapshot::default()), true), "/").await.2)
        .expect("utf-8 page");
    let config = &page[page.find(r#"id="config""#).expect("config popover")..];
    for (group, values) in [
        ("proc-view", &["top", "all", "tree"][..]),
        ("proc-nodes", &["collapsed", "expanded"][..]),
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

/// ACC-GRAPH-43-GPU-LOAD: the GPU panel is one plot with one series per
/// device and a legend with one entry per device, on the network plot's time
/// axis. A device without a utilization reading draws no line and reads
/// unavailable, never zero.
#[tokio::test]
async fn gpu_panel_draws_one_series_and_one_legend_entry_per_device() {
    let mut snapshot = recorded_snapshot("tests/fixtures/proc", 2);
    let chart = || {
        ChartPath::new(
            &[12.0, 37.0],
            0,
            960,
            GPU_PLOT.width,
            GPU_PLOT.height,
            0.0,
            100.0,
        )
    };
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
        assert_eq!(gpu.matches("data-chart=").count(), 1, "{name}: one plot");
        let series: Vec<(&str, &str)> = elements_with(&gpu, "data-gpu")
            .into_iter()
            .filter(|(tag, _)| tag.starts_with("<g"))
            .collect();
        let entries: Vec<(&str, &str)> = elements_with(&gpu, "data-gpu")
            .into_iter()
            .filter(|(tag, _)| tag.starts_with("<div"))
            .collect();
        assert_eq!(series.len(), 3, "{name}: one series per device");
        assert_eq!(entries.len(), 3, "{name}: one legend entry per device");
        for (index, entry) in entries.iter().enumerate() {
            assert_eq!(
                gpu.matches(&format!(
                    r#"class="gpu-series g{index}" data-gpu="{index}""#
                ))
                .count(),
                1,
                "{name}: series {index}"
            );
            assert_eq!(
                gpu.matches(&format!(r#"class="legend-item" data-gpu="{index}""#))
                    .count(),
                1,
                "{name}: legend entry {index}"
            );
            assert!(
                entry.1.contains(&format!("sw-gpu-{index}")),
                "{name}: entry {index} swatch"
            );
        }
        assert!(series[0].1.contains("<path"), "{name}: GPU 0 draws a line");
        assert!(!series[2].1.contains("<path"), "{name}: GPU 2 drew a line");
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
        assert!(
            !gpu.contains("gpu-spark") && !gpu.contains(r#"class="gpu""#),
            "{name}: GPU cards markup remains"
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
    snapshot.mem.gauge_pct = Some(40.0);
    let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
        .expect("utf-8 page");
    let events = fragments(&snapshot);
    for html in page_and_event(&page, &events, "cpu") {
        for marker in [
            format!(
                r#"<path class="g-fill warn" d="{}">"#,
                gauge_arc(Some(50.0))
            ),
            r#"<text class="g-value" x="74" y="66">80.0<tspan class="g-unit" x="75">%</tspan>"#
                .to_string(),
            r#"aria-label="CPU total 80.0 percent""#.to_string(),
            r#"<span class="stat-value crit" data-w="100.0" id="cpu-total">80.0</span>"#
                .to_string(),
        ] {
            assert!(html.contains(&marker), "lacks {marker}");
        }
    }
    for html in page_and_event(&page, &events, "mem") {
        let arc = format!(
            r#"<path class="g-fill warn" d="{}">"#,
            gauge_arc(Some(40.0))
        );
        assert!(html.contains(&arc), "the memory arc is not the EMA");
        assert!(
            html.contains(r#"<b data-w="100%">70%</b> used"#),
            "the memory foot is not raw"
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
            r#"<div class="core-row" title="cpu0" style="--value:42.4"><span class="cl">C00</span><span class="meter"></span><span class="cv">42<i>%</i></span></div>"#,
            r#"<div class="core-row" title="cpu1" style="--value:57.6"><span class="cl">C01</span><span class="meter"></span><span class="cv">58<i>%</i></span></div>"#,
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

/// ACC-VIEW-49-TOTAL-LABEL and ACC-VIEW-50-CPU-LOAD: the CPU gauge reads
/// TOTAL, and beside it the load averages per core and the busy cores.
#[tokio::test]
async fn cpu_gauge_says_total_and_shows_load_per_core_and_cores_busy() {
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
            "<span>load per core</span><b>0.50 0.25 0.15</b><small>1 &middot; 5 &middot; 15 min</small>",
            "1.00 means one running or waiting task per core",
            // ACC-LAYOUT-74-RESERVED-WIDTH: the busy cores reserve the core
            // count in their format, so "of 64" keeps its place.
            r#"<span>cores busy</span><b><span data-w="64.0">16.0</span> <small>of 64</small></b>"#,
        ] {
            assert!(html.contains(marker), "lacks {marker}");
        }
    }
    for (name, html) in std::iter::once(("page", &page)).chain(events.iter().map(|(n, h)| (*n, h)))
    {
        assert!(!html.contains("BUSY"), "{name} says BUSY");
        assert!(!html.contains(">busy<"), "{name} labels a readout busy");
        assert!(!html.contains("percent busy"), "{name} reads busy aloud");
    }
}

/// ACC-SCOPE-64-CONTAINER: the recorded container tree renders only the
/// container's values; what has no container source reads unavailable, and
/// the host values the tree also carries (loadavg 14.22, the 60 cpuN lines
/// outside the cpuset, the cpuinfo frequency, MemTotal 503 GiB) appear nowhere.
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
    let (_sender, receiver) = watch::channel(Some(snapshot.clone()));
    let mut state = AppState::new(receiver, true);
    state.container = Some(container.title().into());
    let page = String::from_utf8(get(&btop_web::app(state), "/").await.2).expect("utf-8 page");

    let start = page
        .find(r#"<span class="chip chip-container" data-p="2" data-field="container-chip""#)
        .expect("CONTAINER chip");
    let chip = &page[start..start + page[start..].find("</span></span>").expect("chip end")];
    assert!(chip.ends_with(r#"<span class="up">container"#), "{chip}");
    let cgroup = container.cgroup.expect("cgroup").display().to_string();
    assert!(
        page.contains(&format!(
            "docker container, detected by 1 /.dockerenv, cgroup {cgroup}."
        )),
        "chip title lacks the cgroup path"
    );

    let events = fragments(&snapshot);
    for html in page_and_event(&page, &events, "cpu") {
        for marker in [
            "<span>load per core</span><b>1.00 1.00 1.00</b>",
            "divided by 2 cores",
            r#"<span>load <b data-w="000">2</b> <b data-w="000">2</b> <b data-w="000">2</b></span>"#,
            r#"data-field="cpu-gauge-cores">2 cores</span>"#,
            // DEF-CPU-41: the cpuset 0-3 gives four core rows, named C00 to C03.
            r#"<h2 id="cores-title">Cores</h2><span class="head-right meta">4 &middot; 0&ndash;100%</span>"#,
            r#"<span class="cl">C03</span>"#,
        ] {
            assert!(html.contains(marker), "lacks {marker}");
        }
        assert_eq!(html.matches(r#"class="core-row""#).count(), 4);
        assert!(
            !html.contains("load unavailable"),
            "the container load is withheld"
        );
        assert!(!html.contains("14 13 15"), "the host load leaked");
        assert!(!html.contains("7.11"), "the host load per core leaked");
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
        assert!(
            html.contains("of 8.00 GiB"),
            "memory total is not memory.max"
        );
        assert!(
            html.contains("IO unavailable"),
            "storage IO is not withheld"
        );
        assert!(!html.contains("of 503"), "MemTotal leaked");
    }
    let proc_rows = page_and_event(&page, &events, "proc");
    for html in proc_rows {
        assert!(!html.contains("13_start_mlflow"), "pid 900 is not a member");
        assert!(html.contains("ttyd"), "member pid 7 is missing");
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
    let fields: [(&str, &str, u8, usize); 18] = [
        ("cpu", "cpu-stat-foot", 3, 1),
        ("cpu", "cpu-name", 3, 1),
        ("cpu", "cpu-gauge-cores", 3, 1),
        ("cpu", "cpu-gauge-facts", 3, 1),
        ("cpu", "gauge-scale", 3, 1),
        ("mem", "mem-stat-foot", 3, 1),
        ("mem", "mem-gauge-total", 3, 1),
        ("mem", "mem-gauge-facts", 3, 1),
        ("mem", "gauge-scale", 3, 1),
        ("net", "net-interface", 3, 1),
        ("net", "net-axis-note", 3, 1),
        ("net", "net-x-axis", 3, 1),
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
        let in_page = if field == "gauge-scale" { 2 } else { count };
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
        .find(r#"<span class="chip chip-tier" id="tier-chip">"#)
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
        "load per core",
        "cores busy",
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
                r#"<span class="stat-value" data-w="000">{}</span><span class="proc-facts">"#,
                snapshot.proc.total.expect("total")
            ),
            "<span>running <b>1</b></span>".to_string(),
            "<span>sleeping <b>7</b></span>".to_string(),
            "<span>waiting <b>0</b></span>".to_string(),
            "<span>zombie <b>0</b></span>".to_string(),
            "<span>threads <b>85</b></span>".to_string(),
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
                r#"80% cpu <b data-w="000">{}</b>"#,
                snapshot.proc.cpu_80.expect("cpu_80")
            ),
            r#"80% mem <b data-w="000">1</b>"#.to_string(),
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
            r#"<span class="stat-value" data-w="0000">"#,
            r#"80% mem <b data-w="0000">1</b>"#,
        ] {
            assert!(tile.contains(marker), "lacks {marker} in {tile}");
        }
    }
}

/// ACC-VIEW-53-PROC-TILE and ACC-VIEW-23-RELEVANT: a host without a GPU shows
/// no GPU process fact; with devices it shows the count, or unavailable.
#[tokio::test]
async fn proc_tile_gpu_fact_follows_the_devices() {
    for (devices, count, expected) in [
        (false, None, None),
        (true, None, Some("<span>gpu unavailable</span>")),
        (true, Some(3), Some("<span>gpu <b>3</b></span>")),
    ] {
        let mut snapshot = fixture_snapshot();
        if !devices {
            snapshot.gpu.devices.clear();
        }
        snapshot.gpu.process_count = count;
        let page = String::from_utf8(get(&app(Some(snapshot.clone()), true), "/").await.2)
            .expect("utf-8 page");
        for tile in proc_tiles(&snapshot, &page) {
            match expected {
                Some(fact) => assert!(tile.contains(fact), "lacks {fact} in {tile}"),
                None => assert!(!tile.contains("<span>gpu "), "a GPU fact without a GPU"),
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
        for absent in ["<b>0</b>", "proc-facts", "data-chart", "stat-foot"] {
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
async fn healthz_is_process_only_and_readyz_waits_for_a_snapshot() {
    let (status, _, body) = get(&app(None, true), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"ok");

    let (status, _, body) = get(&app(None, true), "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, b"starting");

    let (status, _, body) = get(&app(Some(Snapshot::default()), true), "/readyz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"ready");
}

#[tokio::test]
async fn read_only_refuses_every_mutating_method_before_any_handler() {
    let (_sender, receiver) = watch::channel(Some(Snapshot::default()));
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
    let (_sender, receiver) = watch::channel(Some(Snapshot::default()));
    let mut state = AppState::new(receiver, readonly);
    state.config.send_replace(config);
    state.config_file = Arc::new(tokio::sync::Mutex::new(file));
    let live = state.config.clone();
    (btop_web::app(state), live)
}

async fn page_of(app: &Router) -> String {
    String::from_utf8(get(app, "/").await.2).expect("utf-8 page")
}

/// A config file under the test target's temp dir; never a real btop.conf.
fn temp_conf(name: &str, text: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("config-pages")
        .join(name);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("btop.conf");
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
                r#"data-group="proc-nodes""#,
                r#"id="proc-sort""#,
                r#"data-group="proc-dir""#,
            ],
        ),
        (
            "layout",
            &[r#"data-group="panels""#, r#"id="layout-reset""#],
        ),
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
    assert!(
        !inner("layout").contains("data-key"),
        "layout has only the view group"
    );
}

/// ACC-VIEW-75-PANEL-MENU: one right-click menu of Off, S, M and L, after
/// `</main>`, so no swapped fragment holds it.
#[tokio::test]
async fn panel_menu_offers_off_and_the_three_sizes_outside_the_fragments() {
    let page = page_of(&app(Some(fixture_snapshot()), true)).await;
    let menus = roles(&page, "menu");
    assert_eq!(menus.len(), 1);
    let sizes: Vec<_> = roles(menus[0].1, "menuitemradio")
        .iter()
        .map(|(tag, _)| attribute(tag, "data-size"))
        .collect();
    assert_eq!(sizes, [Some("off"), Some("s"), Some("m"), Some("l")]);
    assert!(page.find(r#"id="panel-menu""#) > page.find("</main>"));
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
    assert!(page.contains("read-only: start with --no-readonly to change"));
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
        "changes apply to every viewer until restart; no config file was loaded (-c PATH)"
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

#[tokio::test]
async fn config_post_writes_one_line_of_the_file() {
    let original = default_config_text();
    let path = temp_conf("write", &original);
    let (app, live) = config_app(false, Config::defaults(), Some(path.clone()));
    let (status, fragment) = post_config(&app, "key=update_ms&value=1000", Some(ORIGIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(fragment.contains(r#"value="1000""#), "{fragment}");
    assert!(fragment.contains(&format!(
        "saved to {}",
        std::fs::canonicalize(&path).expect("path").display()
    )));
    let written = std::fs::read_to_string(&path).expect("file");
    assert_eq!(written.lines().count(), original.lines().count());
    for (now, was) in written.lines().zip(original.lines()) {
        if was.starts_with("update_ms =") {
            assert_eq!(now, "update_ms = 1000");
        } else {
            assert_eq!(now, was);
        }
    }
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

#[tokio::test]
async fn config_post_without_a_file_applies_until_restart() {
    let (app, live) = config_app(false, Config::defaults(), None);
    let (status, fragment) = post_config(&app, "key=update_ms&value=1000", Some(ORIGIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        fragment.contains("applied until restart; no config file loaded"),
        "{fragment}"
    );
    assert_eq!(live.borrow().update_ms, 1000);
    assert!(
        page_of(&app).await.contains(">1000 ms<"),
        "a new page load shows the live interval"
    );
}

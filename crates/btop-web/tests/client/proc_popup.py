"""ACC-VIEW-176-PROC-POPUP in a real browser: a click on a process row opens a
popup with that pid, its plots and its whole command line; Escape, a click
outside and the close control each close it; focus moves in and comes back.
ACC-VIEW-199-POPUP-DIM: the config scrim shows with the popup and goes with it.
ACC-VIEW-201-ROW-FOCUS: the row that holds the focus has no outline and paints as a hovered row.
DEF-GRAPH-128-PLOT-JUMP: under a delayed fetch a plot's curve only moves left.
DEF-GRAPH-125-TAIL-GAP: it reaches the plot's right edge in every frame.

python3 -I proc_popup.py URL [OUT.png]

`make test-client` runs it through the ignored e2e test, against the recorded
tree tests/fixtures/proc-gpu. Needs Playwright with its bundled chromium.

DEF-GRAPH-129-PROC-PLOT: the first body's curve covers the time the server has run.
DEF-GRAPH-130-PLOT-STEPS: a delayed and a held body stop no curve.
ACC-VIEW-202-PLOT-LABELS: each plot names itself and has a value scale on its grid.
"""

import itertools
import re
import sys

from playwright.sync_api import sync_playwright

URL = sys.argv[1]
GPU_PID, PLAIN_PID = "500", "501"

# Text and pid of the focused element; a row reports its first cell.
FOCUS = """() => {
  const el = document.activeElement;
  return el.tagName === 'TR' ? 'row:' + el.cells[0].textContent : el.id || el.tagName;
}"""
ROW_STYLE = "(el) => { const style = getComputedStyle(el); return [style.outlineStyle, style.backgroundColor]; }"
# Every plot the popup's rectangle meets, outside the popup.
OVERLAPS = """() => {
  const box = document.getElementById('proc-detail').getBoundingClientRect();
  const hit = [];
  for (const chart of document.querySelectorAll('[data-chart]')) {
    if (chart.closest('#proc-detail')) continue;
    const r = chart.getBoundingClientRect();
    if (r.width && r.height && r.left < box.right && box.left < r.right && r.top < box.bottom && box.top < r.bottom) {
      hit.push(chart.getAttribute('data-chart'));
    }
  }
  return hit;
}"""
# Per popup plot over the given ms: the frames, the largest move right between
# two frames and the travel left, all from the left end of the curve, and the
# most the curve's right end falls short of its plot's right edge.
LEFT_ENDS = """(ms) => new Promise((done) => {
  const plots = {};
  const end = performance.now() + ms;
  const step = () => {
    document.querySelectorAll('[data-chart^="proc-detail"]').forEach((plot, at) => {
      const line = plot.querySelector('.line');
      if (!line) return;
      const box = line.getBoundingClientRect();
      const left = box.left;
      const seen = plots[at] || (plots[at] = { name: plot.dataset.chart, frames: 0, right: 0, first: left, last: left, short: -Infinity });
      seen.frames += 1;
      seen.short = Math.max(seen.short, plot.getBoundingClientRect().right - box.right);
      seen.right = Math.max(seen.right, left - seen.last);
      seen.last = left;
    });
    if (performance.now() < end) requestAnimationFrame(step);
    else done(Object.values(plots));
  };
  requestAnimationFrame(step);
})"""
# Per popup plot over the given ms: the longest run of frames with no movement of
# the curve's left end, the largest move right and the travel left.
STOPS = """(ms) => new Promise((done) => {
  const plots = {};
  const end = performance.now() + ms;
  const step = () => {
    document.querySelectorAll('[data-chart^="proc-detail"] .line').forEach((line, at) => {
      const left = line.getBoundingClientRect().left;
      const seen = plots[at] || (plots[at] = { run: 0, still: 0, right: 0, first: left, last: left });
      seen.run = Math.abs(left - seen.last) < 0.005 ? seen.run + 1 : 0;
      seen.still = Math.max(seen.still, seen.run);
      seen.right = Math.max(seen.right, left - seen.last);
      seen.last = left;
    });
    if (performance.now() < end) requestAnimationFrame(step);
    else done(Object.values(plots));
  };
  requestAnimationFrame(step);
})"""
# Each plot's name, the name's size and colour, a panel heading's colour, its scale
# labels, and each label's distance in px from the place on the plot it stands for.
PLOTS = """() => [...document.querySelectorAll('.pd-plot')].map((plot) => {
  const name = plot.querySelector('.pd-label');
  const box = plot.querySelector('.net-plot').getBoundingClientRect();
  const span = (cls) => plot.querySelector('.plot-scale > .' + cls);
  const middle = span('scale-mid').getBoundingClientRect();
  return {
    name: name.textContent,
    size: parseFloat(getComputedStyle(name).fontSize),
    color: getComputedStyle(name).color,
    h2: getComputedStyle(document.querySelector('.panel .ph h2')).color,
    labels: ['scale-total', 'scale-mid', 'scale-zero'].map((cls) => span(cls).textContent),
    top: span('scale-total').getBoundingClientRect().top - box.top,
    middle: (middle.top + middle.bottom) / 2 - (box.top + box.bottom) / 2,
    zero: span('scale-zero').getBoundingClientRect().bottom - box.bottom,
  };
})"""
# The CPU curve's span in slots of one per second.
CURVE_SLOTS = """() => {
  const plot = document.querySelector('[data-chart="proc-detail-cpu"]');
  const box = plot.getBoundingClientRect();
  const window_s = Number(document.getElementById('update-interval').getAttribute('data-window-ms')) / 1000;
  return (box.right - plot.querySelector('.line').getBoundingClientRect().left) / (box.width / window_s);
}"""


def row(page, pid):
    return page.locator("#proc-body tr", has=page.locator(f'td:first-child:text-is("{pid}")'))


def open_popup(page, pid):
    row(page, pid).locator("td").nth(1).click()
    page.wait_for_selector("#proc-detail:not([hidden]) .pd-live")


def assert_no_stop(plots):
    assert plots, "no curve to measure"
    for plot in plots:
        assert plot["right"] <= 0.1, f"a curve moved right: {plot}"
        assert plot["still"] <= 3, f"a curve stopped: {plot}"
        assert plot["first"] - plot["last"] >= 1, f"a curve did not scroll: {plot}"


def scrim_shown(page):
    return page.locator("#config-scrim").is_visible()


def main():
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        page = browser.new_page(viewport={"width": 1440, "height": 900})
        page.goto(URL, wait_until="load")
        page.wait_for_selector(f'#proc-body td:first-child:text-is("{GPU_PID}")', timeout=20000)

        # DEF-GRAPH-129-PROC-PLOT: the server has sampled since the page woke it,
        # and the first body draws that time at once.
        running = 12
        page.wait_for_timeout(running * 1000)
        open_popup(page, GPU_PID)
        slots = page.evaluate(CURVE_SLOTS)
        assert slots >= running - 3, f"the first body covers {slots} slots of the {running} s the server has run"
        page.keyboard.press("Escape")

        open_popup(page, GPU_PID)
        title = page.inner_text("#proc-detail-title")
        assert GPU_PID in title, f"title {title!r} lacks the pid"
        assert page.evaluate(FOCUS) == "proc-detail-close", "focus did not move into the popup"
        assert scrim_shown(page), "the popup did not dim the dashboard"
        assert page.evaluate("document.elementFromPoint(720, 12).id") == "config-scrim", "the top bar is not dimmed"
        overlaps = page.evaluate(OVERLAPS)
        assert not overlaps, f"the popup covers {overlaps}"
        # The first body draws each plot's curve.
        for chart in ("cpu", "mem", "gpu"):
            page.wait_for_selector(f'[data-chart="proc-detail-{chart}"] path.line', state="attached", timeout=10000)
        command = page.inner_text(".pd-cmd")
        assert "tail-marker.usd" in command and len(command) > 1000, "the command line is clipped"
        assert "<b>night</b>" in command, "the command line is not escaped text"
        box = page.locator(".pd-cmd").bounding_box()
        assert box["width"] > 0 and box["height"] > 0
        if len(sys.argv) > 2:
            page.screenshot(path=sys.argv[2])
        # DEF-VIEW-122-POPUP-COPY: the copy chip returns to rest as the top bar's does.
        page.click(".pd-copy")
        page.wait_for_selector(".pd-copy.copied", timeout=2000)
        page.wait_for_selector(".pd-copy:not(.copied):text-is('copy')", timeout=2000)

        page.keyboard.press("Escape")
        assert page.locator("#proc-detail").is_hidden(), "Escape did not close it"
        assert not scrim_shown(page), "Escape left the scrim"
        assert page.evaluate(FOCUS) == f"row:{GPU_PID}", "Escape did not return focus to the row"
        # ACC-VIEW-201-ROW-FOCUS: with the pointer off the rows, the focus alone styles the row.
        page.mouse.move(720, 12)
        outline, background = row(page, GPU_PID).evaluate(ROW_STYLE)
        row(page, PLAIN_PID).hover()
        _, hovered = row(page, PLAIN_PID).evaluate(ROW_STYLE)
        assert outline == "none", f"the focused row draws an outline: {outline}"
        assert background == hovered, f"the focused row paints {background}, a hovered row {hovered}"

        open_popup(page, GPU_PID)
        page.click("#proc-detail-close")
        assert page.locator("#proc-detail").is_hidden(), "the close control did not close it"
        assert not scrim_shown(page), "the close control left the scrim"
        assert page.evaluate(FOCUS) == f"row:{GPU_PID}", "close did not return focus to the row"

        open_popup(page, GPU_PID)
        page.mouse.click(720, 12)
        assert page.locator("#proc-detail").is_hidden(), "a click outside did not close it"
        assert not scrim_shown(page), "a click outside left the scrim"
        assert page.evaluate(FOCUS) == f"row:{GPU_PID}", "a click outside did not return focus"

        # The scrim covers the other rows: a click on one closes the popup.
        open_popup(page, GPU_PID)
        cell = row(page, PLAIN_PID).locator("td").last.bounding_box()
        page.mouse.click(cell["x"] + cell["width"] / 2, cell["y"] + cell["height"] / 2)
        assert page.locator("#proc-detail").is_hidden(), "a click on another row did not close the popup"

        # CONFIG reached with the popup open closes the popup; the popover keeps the scrim.
        open_popup(page, GPU_PID)
        page.focus(".config-toggle")
        page.keyboard.press("Enter")
        assert page.locator("#proc-detail").is_hidden() and page.locator("#config").is_visible()
        assert scrim_shown(page), "closing the popup removed the popover's scrim"
        page.keyboard.press("Escape")
        assert not scrim_shown(page), "closing the popover left the scrim"

        # ACC-VIEW-202-PLOT-LABELS: the names, and the value labels on the grid.
        open_popup(page, GPU_PID)
        plots = page.evaluate(PLOTS)
        names = [plot["name"] for plot in plots]
        assert names == ["CPU", "Memory", "GPU 0"], names
        for plot in plots:
            assert plot["size"] >= 11, f"a small name: {plot}"
            assert plot["color"] == plot["h2"], f"a dim name: {plot}"
            assert all(plot["labels"]), f"a missing label: {plot}"
            assert abs(plot["middle"]) <= 1, f"the middle label is off its grid line: {plot}"
            assert abs(plot["top"]) <= 1 and abs(plot["zero"]) <= 1, f"a corner label is off the box: {plot}"
        assert plots[0]["labels"][1:] == ["0.5%", "0%"], plots[0]
        assert plots[1]["labels"][2] == "0 B", plots[1]
        assert plots[2]["labels"] == ["100%", "50%", "0%"], plots[2]
        page.keyboard.press("Escape")

        # A process with no GPU node has no GPU plot; the keyboard opens it.
        row(page, PLAIN_PID).focus()
        page.keyboard.press("Enter")
        page.wait_for_selector("#proc-detail:not([hidden]) .pd-live")
        assert PLAIN_PID in page.inner_text("#proc-detail-title")
        assert page.locator('[data-chart="proc-detail-gpu"]').count() == 0, "a GPU plot without a GPU"
        assert page.evaluate("document.documentElement.scrollHeight <= innerHeight"), "the page scrolls"
        page.keyboard.press("Escape")

        delays = itertools.cycle((100, 500, 300))

        def delayed(route):
            page.wait_for_timeout(next(delays))
            route.continue_()

        page.route(re.compile(r"/proc/\d+"), delayed)
        open_popup(page, GPU_PID)
        for chart in ("cpu", "mem", "gpu"):
            page.wait_for_selector(f'[data-chart="proc-detail-{chart}"] path.line', state="attached", timeout=10000)
        plots = page.evaluate(LEFT_ENDS, 5500)
        assert {plot["name"] for plot in plots} == {"proc-detail-cpu", "proc-detail-mem", "proc-detail-gpu"}, plots
        for plot in plots:
            assert plot["frames"] >= 30, f"too few frames: {plot}"
            assert plot["right"] <= 0.1, f"a curve moved right: {plot}"
            assert plot["first"] - plot["last"] >= 1, f"a curve did not scroll: {plot}"
            assert plot["short"] <= 0, f"a curve stops short of its plot's edge: {plot}"
        page.keyboard.press("Escape")
        # The last delayed fetch ends before the page closes.
        page.wait_for_timeout(600)

        # DEF-GRAPH-130-PLOT-STEPS: no curve stops over a delayed body, nor over one held for 4 s.
        open_popup(page, GPU_PID)
        assert_no_stop(page.evaluate(STOPS, 5500))
        page.keyboard.press("Escape")
        page.wait_for_timeout(600)
        page.unroute(re.compile(r"/proc/\d+"))
        open_popup(page, GPU_PID)
        held = []
        page.route(re.compile(r"/proc/\d+"), lambda route: held.append(route))
        assert_no_stop(page.evaluate(STOPS, 4000))
        assert held, "no body was held"
        for route in held:
            route.continue_()
        page.unroute(re.compile(r"/proc/\d+"))
        browser.close()
    print("ok")


main()

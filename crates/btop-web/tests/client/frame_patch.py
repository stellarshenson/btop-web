"""ACC-PERF-133-PAGE-COST in a real browser: a frame writes into the nodes on the
page. A kept node keeps its identity, the panel ends equal to the fragment, and
what the page owns survives: a chart's animation and a gauge text's fitted size.
DEF-GRAPH-125-TAIL-GAP: every curve reaches its plot's right edge in every frame.

python3 -I frame_patch.py URL

`make test-client` runs it through the ignored e2e test, against the recorded
tree tests/fixtures/proc-gpu. Needs Playwright with its bundled chromium.
"""

import sys

from playwright.sync_api import sync_playwright

URL = sys.argv[1]

# One task, so a live frame cannot land between a swap and its check. The sink
# is a page element htmx swaps a frame into; the wrapper's id picks the panel.
CHECKS = """() => {
  const sink = document.querySelector('[sse-swap="net"]');
  const frame = (id, html) => htmx.swap(sink, `<div id="${id}" hx-swap-oob="innerHTML">${html}</div>`, { swapStyle: 'innerHTML' });
  const out = {};
  const panel = document.getElementById('p-storage');

  frame('p-storage', '<p class="a" data-x="1">one</p><i>two</i>');
  const p = panel.querySelector('p');
  const text = p.firstChild;
  frame('p-storage', '<p class="b" title="t">three</p><i>two</i>');
  out.attributesAndText = panel.innerHTML === '<p class="b" title="t">three</p><i>two</i>';
  out.nodeKept = panel.querySelector('p') === p && p.firstChild === text;
  frame('p-storage', '<p class="b" title="t">three</p><i>two</i><b>grown</b>');
  out.grown = panel.innerHTML === '<p class="b" title="t">three</p><i>two</i><b>grown</b>';
  frame('p-storage', '<span>other</span>');
  out.reshaped = panel.innerHTML === '<span>other</span>';

  // The server's form of a panel carries neither the chart's animation nor the
  // gauge text's fitted size; a frame leaves both on the kept nodes.
  const server = (el) => el.innerHTML.replace(/ style="[^"]*"/g, '');
  const body = document.querySelector('#p-cpu-gauge .gauge-body');
  const value = [...document.querySelectorAll('#p-cpu-gauge .g-value')].find((node) => node.style.fontSize);
  const size = value ? value.style.fontSize : '';
  frame('p-cpu-gauge', server(document.getElementById('p-cpu-gauge')));
  out.gaugeKept = !!value && value.isConnected && body.isConnected && value.style.fontSize === size;
  const svg = document.querySelector('[data-chart] svg.chart[style]');
  const name = svg ? svg.style.animationName : '';
  const stat = svg ? svg.closest('[id^="p-"]') : null;
  if (stat) frame(stat.id, server(stat));
  out.chartKept = !!svg && svg.isConnected && !!name && svg.style.animationName === name;
  return out;
}"""

# DEF-GRAPH-125-TAIL-GAP: per frame, how far each drawn curve's end falls short of
# the right edge of the box that clips its svg, and how far the svg scrolled.
TAIL_GAP = """(ms) => new Promise((done) => {
  const seen = {};
  const end = performance.now() + ms;
  const step = () => {
    document.querySelectorAll('[data-chart]').forEach((box) => {
      const line = box.querySelector('g.series path.line, g.gpu-series path.line');
      if (!line) return;
      const edge = box.getBoundingClientRect();
      const gap = edge.right - line.getBoundingClientRect().right;
      const shift = edge.left - line.ownerSVGElement.getBoundingClientRect().left;
      const s = seen[box.dataset.chart] || (seen[box.dataset.chart] = { gap, shift });
      s.gap = Math.max(s.gap, gap);
      s.shift = Math.max(s.shift, shift);
    });
    if (performance.now() < end) requestAnimationFrame(step);
    else done(seen);
  };
  requestAnimationFrame(step);
})"""

with sync_playwright() as play:
    browser = play.chromium.launch(headless=True)
    page = browser.new_page(viewport={"width": 1440, "height": 900})
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.goto(URL, wait_until="load")
    page.wait_for_selector("[data-chart] svg.chart[style]", state="attached", timeout=10000)
    gaps = page.evaluate(TAIL_GAP, 3000)
    assert {"cpu", "mem", "proc-count", "gpu"} <= gaps.keys(), gaps
    for name, seen in gaps.items():
        assert seen["shift"] > 0, f"{name} did not scroll: {seen}"
        assert seen["gap"] <= 0, f"{name} stops short of its plot's edge: {seen}"
    result = page.evaluate(CHECKS)
    browser.close()

failed = [name for name, passed in result.items() if not passed]
print(result)
if failed or errors:
    sys.exit(f"failed: {failed} page errors: {errors}")

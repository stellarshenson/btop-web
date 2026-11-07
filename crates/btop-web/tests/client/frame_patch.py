"""ACC-PERF-133-PAGE-COST in a real browser: a frame writes into the nodes on the
page. A kept node keeps its identity, the panel ends equal to the fragment, and
what the page owns survives: a chart's animation and a gauge text's fitted size.
DEF-GRAPH-125-TAIL-GAP: every curve reaches its plot's right edge in every frame.

python3 -I frame_patch.py URL [CLIENT_URL]

`make test-client` runs it through the ignored e2e test, against the recorded
tree tests/fixtures/proc-gpu. Needs Playwright with its bundled chromium. A
client-mode page is rendered from the default snapshot; the second URL makes
the script check that the cpu and mem records create the members that page
lacks, and that a running number effect ends on the newest reading.
"""

import sys

from playwright.sync_api import sync_playwright

URL = sys.argv[1]
CLIENT_URL = sys.argv[2] if len(sys.argv) > 2 else None


def cpu_record(total, one="10%"):
    """A complete c1 record: two cores, an aggregate load, a frequency and no
    temperature."""
    fields = [
        "c1", "250", "1000", "1000", "1", str(total), "ok", "ok",
        "M0 0", "M0 0", "61.00", "100%", "Load average, percent of one core",
        one, "20%", "30%", "20%", "10%", "1.25%",
        "3.20 GHz", "", "4", "Test CPU", "8", "", "2", "",
        "cpu4", "42.4", "42", "cpu9", "", "",
        "", "",
    ]
    return "|".join(fields)


def mem_record():
    """A complete m1 record: a total, seven rows (two created), one mount and
    the tip set of a container under a memory limit."""
    fields = [
        "m1", "1", "5.0", "GiB", "8", "63%", "37%", "5.0 GiB", "",
        "8.00 GiB", " (container limit)", "", "", "", "",
        "1",
        "used", "M0 0", "M0 0", "0.00", "63.00",
        "2", "used", "63.000", "free", "37.000",
        "7",
        "used", "5.0 GiB", "63%", "used", "",
        "cache", "1.0 GiB", "12%", "reclaimable", "",
        "other", "0.5 GiB", "6%", "other used", "",
        "other-cache", "0.2 GiB", "3%", "other reclaimable", "",
        "free", "1.3 GiB", "16%", "container free", "",
        "free-cache", "1.5 GiB", "19%", "free + reclaimable", "",
        "swap", "0.1 / 2 GiB", "5%", "swap", "",
        "1",
        "data\\|disk", "12%", "1.00 TB", "63", "63%", "630 GB", "37", "37%", "370 GB",
        "limit",
        "", "",
    ]
    return "|".join(fields)


# A client page is rendered from the default snapshot: no CPU name, core rows,
# memory total, extra memory rows or mount exists. Dispatching the same events
# the agent sends must create them, and a number under a running spring must
# finish on the newest reading rather than the one it started from.
CLIENT_CHECKS = """([one, two, mem]) => {
  const send = (type, data) => document.body.dispatchEvent(new CustomEvent('htmx:sseBeforeMessage', { bubbles: true, cancelable: true, detail: { type, data } }));
  const text = (selector) => {
    const el = document.querySelector(selector);
    return el ? el.textContent : null;
  };
  const tips = (key) => document.getElementById('tips').getAttribute('data-' + key);
  const out = {};
  send('cpu', one);
  out.cpuFactCreated = text('#p-cpu-stat [data-field="cpu-freq"] b') === '3.20 GHz' && document.querySelector('#p-cpu-stat [data-field="cpu-freq"]').getAttribute('data-tip') === tips('cpu-freq');
  // DEF-VIEW-152-LOAD-TIP: both load facts take the record's load tip.
  out.loadTipsFromRecord = document.querySelector('#p-cpu-stat [data-field="cpu-load"]').getAttribute('data-tip') === one.split('|')[12]
    && document.querySelector('#p-cpu-gauge .fact.load').getAttribute('data-tip') === one.split('|')[12];
  out.cpuNameCreated = text('#p-cpu-gauge [data-field="cpu-name"]') === 'Test CPU';
  out.coresCreated = document.querySelectorAll('#p-cores .core-row').length === 2 && text('#p-cores .core-row .cl') === 'C04';
  out.firstNumber = text('#cpu-total') === '37';
  out.cpuGaugeRead = text('#p-cpu-gauge .g-read b') === '37%';
  send('mem', mem);
  out.memGaugeRead = text('#p-mem-gauge .g-read b') === '5.0GiB';
  out.mountCreated = text('#p-storage .mount .mount-label') === 'data|disk';
  out.mountOneTip = document.querySelector('#p-storage .mount .mount-label').getAttribute('data-tip') === text('#p-storage .mount .mount-label') && document.querySelector('#p-storage .mount .mount-io').getAttribute('data-tip') === tips('mount-io');
  out.rowsCreated = !!document.querySelector('#p-mem-gauge .mem-row[data-row="other"]') && !!document.querySelector('#p-mem-gauge .mem-row[data-row="swap"]');
  out.totalCreated = text('#p-mem-gauge [data-field="mem-gauge-total"] .meta-value') === '8.00 GiB';
  out.usedFactCreated = text('#p-mem-stat [data-field="mem-used-fact"] b') === '63%';
  // DEF-PERF-147-TIP-FRAMES: the record names the limit set; the page's
  // texts reach the created nodes and replace the default page's host wording.
  const tipOf = (selector) => document.querySelector(selector).getAttribute('data-tip');
  out.memTipsFromPage = !!tips('row-free-limit')
    && tipOf('#p-mem-gauge .mem-row[data-row="free"]') === tips('row-free-limit')
    && tipOf('#p-mem-gauge .mem-row[data-row="other"]') === tips('row-other')
    && tipOf('#p-mem-gauge .mem-row[data-row="swap"]') === tips('row-swap-limit')
    && tipOf('#p-mem-gauge [data-field="mem-gauge-total"]') === tips('total-limit')
    && tipOf('#p-mem-stat [data-field="mem-used-fact"]') === tips('used-limit')
    && tipOf('#p-storage .mount') === tips('mount');
  send('cpu', two);
  out.springStarted = document.getElementById('cpu-total').hasAttribute('data-num-host');
  return out;
}"""

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
    if CLIENT_URL:
        client = browser.new_page(viewport={"width": 1440, "height": 900})
        client.on("pageerror", lambda error: errors.append(str(error)))
        client.goto(CLIENT_URL, wait_until="load")
        client.evaluate("localStorage.setItem('btop-web-num-motion', 'spring')")
        client.reload(wait_until="load")
        client.wait_for_selector("#p-cpu-stat .stat-tile", state="attached", timeout=10000)
        result.update(
            client.evaluate(
                CLIENT_CHECKS,
                [cpu_record(37), cpu_record(61, "15%"), mem_record()],
            )
        )
        # The spring settles 800 ms after the reading; the extra time covers a
        # slow first frame.
        client.wait_for_timeout(1500)
        result["springFinal"] = client.evaluate(
            """() => {
              const el = document.getElementById('cpu-total');
              return !!el && !el.hasAttribute('data-num-host') && el.textContent === '61';
            }"""
        )
        client.close()
    browser.close()

failed = [name for name, passed in result.items() if not passed]
print(result)
if failed or errors:
    sys.exit(f"failed: {failed} page errors: {errors}")

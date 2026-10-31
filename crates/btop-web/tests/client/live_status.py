"""ACC-VIEW-200-LIVE-STATUS in a real browser: the choice survives a reload and
applies on the first paint; on is the chip, dimmed is plain muted text and off
hides the status, in LIVE, PAUSED and STALE; the chips to its right stay where
they are.

python3 -I live_status.py URL

`make test-client` runs it through the ignored e2e test, against the recorded
tree tests/fixtures/proc-gpu. Needs Playwright with its bundled chromium.
"""

import sys

from playwright.sync_api import sync_playwright

URL = sys.argv[1]
KEY = "btop-web-live-status"
# Keeps the page's stream, so the test can stop it and the page turns stale.
KEEP_STREAM = """const Source = window.EventSource;
window.EventSource = class extends Source {
  constructor(...args) { super(...args); window.stream = this; }
};"""
# What the status shows and how it is drawn.
READ = """() => {
  const chip = document.getElementById('live-chip');
  const style = getComputedStyle(chip);
  const word = chip.querySelector('.live-word');
  const age = chip.querySelector('.live-age');
  const upper = getComputedStyle(word).textTransform === 'uppercase';
  return {
    shown: style.display !== 'none',
    text: ((upper ? word.textContent.toUpperCase() : word.textContent) + ' '
      + (getComputedStyle(age).display === 'none' ? '' : age.textContent)).trim(),
    border: parseFloat(style.borderTopWidth),
    radius: parseFloat(style.borderTopLeftRadius),
    tint: getComputedStyle(chip, '::before').display !== 'none',
    lamp: getComputedStyle(chip.querySelector('.lamp')).display !== 'none',
    colour: style.color,
    muted: getComputedStyle(document.querySelector('.u-meta')).color,
    tip: chip.hasAttribute('data-tip'),
  };
}"""
# The right edge of each chip right of the status.
RIGHT = """() => {
  const right = [];
  for (let el = document.getElementById('live-chip').nextElementSibling; el; el = el.nextElementSibling) {
    right.push(el.getBoundingClientRect().right);
  }
  return right;
}"""
# The status's display when the parser first creates it, before any paint.
FIRST_PAINT = """new MutationObserver((_, observer) => {
  const chip = document.getElementById('live-chip');
  if (!chip) return;
  window.firstDisplay = getComputedStyle(chip).display;
  observer.disconnect();
}).observe(document, { childList: true, subtree: true });"""


def from_config(page, selector):
    page.click(".config-toggle")
    page.click(selector)
    page.keyboard.press("Escape")


def enter(page, state):
    if state == "paused":
        from_config(page, "#pause-btn")
    elif state == "stale":
        page.evaluate("window.stream.close()")
    if state != "live":
        page.wait_for_selector(f"#live-chip.{state}", state="attached")


def check(read, value, state):
    if value == "off":
        assert not read["shown"], f"off {state}: {read}"
        return
    assert read["shown"] and read["tip"], f"{value} {state}: {read}"
    if value == "on":
        assert read["border"] == 1 and read["radius"] == 2, f"on {state}: {read}"
        assert read["tint"] and read["lamp"], f"on {state}: {read}"
        assert read["text"].startswith(state.upper()), f"on {state}: {read}"
    else:
        assert read["border"] == 0 and read["radius"] == 0, f"dimmed {state}: {read}"
        assert not read["tint"] and not read["lamp"], f"dimmed {state}: {read}"
        assert read["colour"] == read["muted"], f"dimmed {state}: {read}"
        if state == "live":
            assert read["text"].startswith("live ") and read["text"].endswith(" s"), read
        else:
            assert read["text"] == state, f"dimmed {state}: {read}"


def main():
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        context = browser.new_context(viewport={"width": 1040, "height": 760})
        context.add_init_script(KEEP_STREAM)
        context.add_init_script(FIRST_PAINT)
        page = context.new_page()
        page.goto(URL)
        assert page.evaluate("document.documentElement.getAttribute('data-live-status')") == "on"
        baseline = {}
        for value in ("on", "dimmed", "off"):
            for state in ("live", "paused", "stale"):
                page.goto(URL)
                from_config(page, f'[data-group="live-status"] [data-value="{value}"]')
                page.reload()
                page.wait_for_function("document.getElementById('sample-age').textContent !== ''")
                assert page.evaluate(f"localStorage.getItem('{KEY}')") == value
                assert (page.evaluate("window.firstDisplay") == "none") == (value == "off")
                checked = page.locator('[data-group="live-status"] [aria-checked="true"]')
                assert checked.get_attribute("data-value") == value
                enter(page, state)
                check(page.evaluate(READ), value, state)
                right = page.evaluate(RIGHT)
                assert baseline.setdefault(state, right) == right, f"{value} {state}: {right}"
        browser.close()
    print("ok")


main()

"""ACC-LAYOUT-222-DRAG-LIVE in a real browser: a held drag of the CPU header
past Memory and Processes shows the insert order before the release; the
release and a reload keep it; a second drag cancelled with Escape restores it.

python3 -I drag_live.py URL

`make test-client` runs it through the ignored e2e test, against the recorded
tree tests/fixtures/proc-gpu. Needs Playwright with its bundled chromium.
"""

import sys

from playwright.sync_api import sync_playwright

URL = sys.argv[1]
SHOWN = "[...document.querySelectorAll('.band .panel[data-panel]')].map(p => p.dataset.panel)"
STORED = "JSON.parse(localStorage.getItem('btop-web-layout')).order"


def centre(page, selector):
    box = page.locator(selector).bounding_box()
    return box["x"] + box["width"] / 2, box["y"] + box["height"] / 2


def hold_drag(page, panel, onto):
    """Press the panel's header and move over `onto`, still holding."""
    target = centre(page, f'[data-panel="{onto}"] .ph')
    page.mouse.move(*centre(page, f'[data-panel="{panel}"] .ph h2'))
    page.mouse.down()
    page.mouse.move(*target, steps=20)


def main():
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        page = browser.new_page(viewport={"width": 1440, "height": 900})
        page.goto(URL)
        start = page.evaluate(SHOWN)
        assert start[:3] == ["cpu-stat", "mem-stat", "proc-stat"], start
        moved = ["mem-stat", "proc-stat", "cpu-stat"] + start[3:]

        hold_drag(page, "cpu-stat", "proc-stat")
        assert page.evaluate(SHOWN) == moved, f"held: {page.evaluate(SHOWN)}"
        page.mouse.up()
        assert page.evaluate(SHOWN) == moved, f"released: {page.evaluate(SHOWN)}"
        assert page.evaluate(STORED) == moved, f"stored: {page.evaluate(STORED)}"
        page.reload()
        assert page.evaluate(SHOWN) == moved, f"reloaded: {page.evaluate(SHOWN)}"

        hold_drag(page, "mem-stat", "cpu-stat")
        assert page.evaluate(SHOWN) != moved, "the second drag moved nothing"
        page.keyboard.press("Escape")
        assert page.evaluate(SHOWN) == moved, f"escaped: {page.evaluate(SHOWN)}"
        page.mouse.up()
        assert page.evaluate(SHOWN) == moved, f"after escape: {page.evaluate(SHOWN)}"
        browser.close()
    print("ok")


main()

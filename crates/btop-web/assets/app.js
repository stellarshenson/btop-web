// Dashboard interactions: the live state, the CONFIG popover, panel layout,
// GPU selection, pause, the process list (view, tree, filter, sort) and the
// process table fit. One delegated listener per event; the only timers are
// the filter debounce and the stale timeout, which is re-armed once per frame
// (a stream that stops sends no event to notice).
// Theme, mode and the stored layout load in theme.js.
(function () {
  "use strict";

  var GPU_KEY = "btop-web-gpu-selection";
  var config = document.getElementById("config");
  var pauseButton = document.getElementById("pause-btn");
  var filterInput = document.getElementById("filter");
  var procCount = document.getElementById("proc-count");
  var liveChip = document.getElementById("live-chip");
  var sampleAge = document.getElementById("sample-age");
  var interval = document.getElementById("update-interval");
  // ACC-CONF-61: each cpu frame carries the interval in effect (read below).
  var updateMs = interval ? Number(interval.getAttribute("data-update-ms")) || 250 : 250;
  // ACC-GRAPH-98-ANIM-CLOCK: the chart window, set from the interval beside it.
  var windowMs = interval ? Number(interval.getAttribute("data-window-ms")) || 0 : 0;

  var lastTrigger = null;
  var paused = false;
  var stale = false;
  var staleTimer = 0;
  var lastFrame = 0;
  var lastFrameAt = 0; // performance.now() of the last frame, the motion clock
  var pending = [];
  var filterTimer = 0;
  var gpuSelection = [];

  function closest(node, selector) {
    return node && node.closest ? node.closest(selector) : null;
  }

  /* ---------- stat tooltips (ACC-VIEW-80-STAT-TIPS) ---------- */

  // The server writes each stat's text into data-tip; this code only places
  // the one box. A process cell takes its column header's text, so a table of
  // hundreds of rows repeats no attribute. The box stays clear of the hovered
  // stat, every plot box and the viewport edges, and a pointer that stays put
  // keeps it through the frame swaps.
  var tip = document.getElementById("tip");
  var tipAt = null;
  var tipX = 0;
  var tipY = 0;

  function tipSource(node) {
    if (!node || !node.closest) return null;
    // A plot box carries no tooltip, and neither does its contents.
    if (node.closest("[data-chart]")) return null;
    var own = node.closest("[data-tip]");
    if (own) return { el: own, text: own.getAttribute("data-tip") };
    var cell = node.closest("td");
    var table = cell ? cell.closest("table.ptable") : null;
    if (!table) return null;
    var head = table.querySelectorAll("thead th")[cell.cellIndex];
    if (!head || !head.hasAttribute("data-tip")) return null;
    return { el: cell, text: head.getAttribute("data-tip") };
  }

  function placeTip(anchor) {
    var pad = 8;
    var gap = 6;
    var box = anchor.getBoundingClientRect();
    var width = tip.offsetWidth;
    var height = tip.offsetHeight;
    var vw = window.innerWidth;
    var vh = window.innerHeight;
    var charts = [];
    var nodes = document.querySelectorAll("[data-chart]");
    for (var index = 0; index < nodes.length; index += 1) {
      var rect = nodes[index].getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) charts.push(rect);
    }
    // On screen, clear of the hovered stat and of every plot.
    function clear(x, y) {
      if (x < pad || y < pad || x + width > vw - pad || y + height > vh - pad) return false;
      if (x < box.right + gap && x + width > box.left - gap && y < box.bottom + gap && y + height > box.top - gap) return false;
      for (var hit = 0; hit < charts.length; hit += 1) {
        // 1 px of slack: a live frame can shift a chart by a fraction.
        if (x < charts[hit].right + 1 && x + width > charts[hit].left - 1 && y < charts[hit].bottom + 1 && y + height > charts[hit].top - 1) return false;
      }
      return true;
    }
    function put(x, y) {
      tip.style.left = x + "px";
      tip.style.top = y + "px";
    }
    var align = [
      Math.max(pad, Math.min(box.left, vw - width - pad)),
      Math.max(pad, Math.min(box.right - width, vw - width - pad)),
    ];
    for (var side = 0; side < align.length; side += 1) {
      for (var away = gap; away < vh + height; away += 6) {
        if (clear(align[side], box.top - away - height)) return put(align[side], box.top - away - height);
        if (clear(align[side], box.bottom + away)) return put(align[side], box.bottom + away);
      }
    }
    var y = Math.max(pad, Math.min(box.top, vh - height - pad));
    for (var across = gap; across < vw + width; across += 6) {
      if (clear(box.right + across, y)) return put(box.right + across, y);
      if (clear(box.left - across - width, y)) return put(box.left - across - width, y);
    }
    put(pad, pad);
  }

  function hideTip() {
    tipAt = null;
    if (tip) tip.hidden = true;
  }

  function showTip(source) {
    if (!tip) return;
    if (!source) return hideTip();
    if (tipAt && tipAt.el === source.el && tipAt.text === source.text) return;
    tipAt = source;
    tip.textContent = source.text;
    tip.hidden = false;
    tip.style.visibility = "hidden";
    placeTip(source.el);
    tip.style.visibility = "";
  }

  // A frame replaces the stat under a still pointer; re-read its text.
  function refreshTip() {
    if (!tip || !tipAt || tip.hidden) return;
    var source = tipSource(document.elementFromPoint(tipX, tipY));
    if (!source) return hideTip();
    showTip(source);
  }

  if (tip) {
    document.addEventListener("pointermove", function (event) {
      tipX = event.clientX;
      tipY = event.clientY;
      showTip(tipSource(event.target));
    });
    document.addEventListener("pointerdown", hideTip);
    document.documentElement.addEventListener("pointerleave", hideTip);
    window.addEventListener("blur", hideTip);
    window.addEventListener("resize", hideTip);
  }

  /* ---------- clock and live state ---------- */

  function paintClock() {
    var now = new Date();
    var text = String(now.getHours()).padStart(2, "0") + ":" + String(now.getMinutes()).padStart(2, "0");
    var clocks = document.querySelectorAll(".hdr-clock");
    for (var index = 0; index < clocks.length; index += 1) clocks[index].textContent = text;
  }

  // STALE outranks PAUSED: a paused page still receives frames, so it only
  // turns stale when the stream itself stops.
  function paintLive() {
    var state = stale ? "stale" : paused ? "paused" : "live";
    if (liveChip) {
      liveChip.classList.toggle("paused", state === "paused");
      liveChip.classList.toggle("stale", state === "stale");
    }
    var words = document.querySelectorAll(".live-word");
    for (var index = 0; index < words.length; index += 1) {
      if (words[index].textContent !== state) words[index].textContent = state;
    }
    if (pauseButton) {
      pauseButton.classList.toggle("on", paused);
      pauseButton.setAttribute("aria-pressed", String(paused));
      pauseButton.textContent = paused ? "resume" : "pause";
    }
    syncMotion();
  }

  function armStale() {
    window.clearTimeout(staleTimer);
    staleTimer = window.setTimeout(function () {
      stale = true;
      if (lastFrame > 0 && sampleAge) sampleAge.textContent = "since " + new Date(lastFrame).toTimeString().slice(0, 8);
      paintLive();
    }, 2 * updateMs);
  }

  // Fires for every frame, paused or not, before the frame is applied.
  document.body.addEventListener("htmx:sseBeforeMessage", function (event) {
    if (event.detail.type === "cpu") {
      var sent = /data-update-ms="(\d+)"/.exec(event.detail.data);
      if (sent) updateMs = Number(sent[1]);
      var windowSent = /data-window-ms="(\d+)"/.exec(event.detail.data);
      if (windowSent) windowMs = Number(windowSent[1]);
      var now = Date.now();
      if (lastFrame > 0 && sampleAge) sampleAge.textContent = ((now - lastFrame) / 1000).toFixed(1) + " s";
      lastFrame = now;
      lastFrameAt = window.performance.now();
      if (stale) {
        stale = false;
        paintLive();
      }
      armStale();
      paintClock();
      syncMotion();
    }
    var held = procHold && event.detail.type === "proc";
    if (!paused && !held) {
      if (event.detail.type === "proc") procFocus = procFocusKey(document.activeElement);
      return;
    }
    event.preventDefault();
    rememberSnapshot(event);
  });

  /* ---------- plot motion on its own clock (ACC-GRAPH-98-ANIM-CLOCK) ---------- */

  // Between readouts every panel chart scrolls left at the display frame rate:
  // one slot is the plot width times the interval over the chart's window, and
  // the server draws each new frame exactly one slot further left, so the swap
  // continues the motion instead of restarting it. Only the drawn series group
  // moves; the grid under the box, the axes, the labels and the process row
  // sparklines stay. The server's one-slot tail keeps the curve at the right
  // edge and holds the newest smoothed knot, never a raw sample.
  var MOTION_KEY = "btop-web-motion";
  var motionPref = true;
  try {
    motionPref = window.localStorage.getItem(MOTION_KEY) !== "off";
  } catch (error) {
    // Storage is optional; motion stays on for this page view.
  }
  var reducedMotion = window.matchMedia ? window.matchMedia("(prefers-reduced-motion: reduce)") : null;
  var motionFrame = 0;

  function motionAllowed() {
    return motionPref && windowMs > 0 && lastFrameAt > 0 && !paused && !stale && !document.hidden && !(reducedMotion && reducedMotion.matches);
  }

  function shiftSeries(progress) {
    var groups = document.querySelectorAll("[data-chart] g.series, [data-chart] g.gpu-series");
    for (var index = 0; index < groups.length; index += 1) {
      var svg = groups[index].ownerSVGElement;
      var width = svg && svg.viewBox ? svg.viewBox.baseVal.width : 0;
      groups[index].style.setProperty("--shift", (-progress * width * updateMs / windowMs).toFixed(4) + "px");
    }
  }

  // The frame's own timestamp, not the callback's wall time: the step is then
  // the display cadence itself, evenly spaced whatever the paint costs.
  function motionStep(now) {
    motionFrame = 0;
    if (!motionAllowed()) return;
    shiftSeries(Math.min(1, Math.max(0, (now - lastFrameAt) / updateMs)));
    motionFrame = window.requestAnimationFrame(motionStep);
  }

  // One loop, re-armed only while it is allowed to run.
  function syncMotion() {
    if (!motionAllowed()) {
      if (motionFrame) window.cancelAnimationFrame(motionFrame);
      motionFrame = 0;
      return;
    }
    if (!motionFrame) motionFrame = window.requestAnimationFrame(motionStep);
  }

  function syncMotionControls() {
    var chips = document.querySelectorAll('[data-group="motion"] .opt');
    for (var index = 0; index < chips.length; index += 1) {
      var on = chips[index].getAttribute("data-value") === (motionPref ? "on" : "off");
      chips[index].classList.toggle("on", on);
      chips[index].setAttribute("aria-checked", String(on));
    }
  }

  document.addEventListener("click", function (event) {
    var chip = closest(event.target, '[data-group="motion"] .opt');
    if (!chip) return;
    motionPref = chip.getAttribute("data-value") !== "off";
    try {
      window.localStorage.setItem(MOTION_KEY, motionPref ? "on" : "off");
    } catch (error) {
      // Storage is optional; the choice holds for this page view.
    }
    syncMotionControls();
    syncMotion();
  });

  if (reducedMotion && reducedMotion.addEventListener) reducedMotion.addEventListener("change", syncMotion);
  document.addEventListener("visibilitychange", syncMotion);

  /* ---------- CONFIG popover ---------- */

  function setExpanded(open) {
    var toggles = document.querySelectorAll(".config-toggle");
    for (var index = 0; index < toggles.length; index += 1) {
      toggles[index].setAttribute("aria-expanded", String(open));
    }
  }

  function visibleToggle() {
    var toggles = document.querySelectorAll(".config-toggle");
    for (var index = 0; index < toggles.length; index += 1) {
      if (toggles[index].offsetParent !== null) return toggles[index];
    }
    return null;
  }

  function closeConfig(returnTo) {
    if (!config || config.hidden) return;
    config.hidden = true;
    setExpanded(false);
    if (!returnTo) return;
    var trigger = returnTo.isConnected ? returnTo : visibleToggle();
    if (trigger) trigger.focus();
  }

  function openConfig(trigger) {
    if (!config) return;
    lastTrigger = trigger;
    config.hidden = false;
    setExpanded(true);
    var tab = config.querySelector('[role="tab"][aria-selected="true"]');
    if (tab) tab.focus();
  }

  // ACC-CONF-61-CONFIG-PAGES: WAI-ARIA tabs with automatic activation; the
  // selected page holds for this page view and is not stored.
  var tabs = config ? config.querySelectorAll('[role="tab"]') : [];

  function selectTab(tab) {
    for (var index = 0; index < tabs.length; index += 1) {
      var on = tabs[index] === tab;
      tabs[index].classList.toggle("on", on);
      tabs[index].setAttribute("aria-selected", String(on));
      tabs[index].tabIndex = on ? 0 : -1;
      var page = document.getElementById(tabs[index].getAttribute("aria-controls"));
      if (page) page.hidden = !on;
    }
  }

  if (tabs.length > 0) {
    var tabList = tabs[0].parentNode;
    tabList.addEventListener("click", function (event) {
      var tab = closest(event.target, '[role="tab"]');
      if (tab) selectTab(tab);
    });
    tabList.addEventListener("keydown", function (event) {
      var at = Array.prototype.indexOf.call(tabs, document.activeElement);
      var to = { ArrowRight: at + 1, ArrowLeft: at - 1, Home: 0, End: tabs.length - 1 }[event.key];
      if (at === -1 || to === undefined) return;
      event.preventDefault();
      var tab = tabs[(to + tabs.length) % tabs.length];
      selectTab(tab);
      tab.focus();
    });
  }

  if (config) {
    document.addEventListener("click", function (event) {
      var toggle = closest(event.target, ".config-toggle");
      if (toggle) {
        if (config.hidden) openConfig(toggle);
        else closeConfig(null);
        return;
      }
      if (!config.hidden && !config.contains(event.target)) closeConfig(null);
    });

    document.addEventListener("keydown", function (event) {
      if (event.key !== "Escape" || config.hidden) return;
      event.preventDefault();
      closeConfig(lastTrigger);
    });
  }

  /* ---------- panel layout (ACC-VIEW-40-GADGETS) ---------- */

  var layoutApi = window.btopLayout;
  var layout = layoutApi ? layoutApi.read() : null;

  // A band whose panels are all hidden - by the layout, a viewport drop or an
  // empty panel - gives its height to the others. A child's computed display
  // is its own, so a hidden band does not hide its panels from this check.
  function syncBands() {
    var bands = document.querySelectorAll(".band");
    for (var index = 0; index < bands.length; index += 1) {
      var panels = bands[index].querySelectorAll(".panel");
      var shown = false;
      for (var inner = 0; inner < panels.length; inner += 1) {
        if (window.getComputedStyle(panels[inner]).display !== "none") shown = true;
      }
      bands[index].hidden = !shown;
    }
  }

  function syncLayoutControls() {
    if (!layout) return;
    var rows = document.querySelectorAll(".cfg-panel[data-panel-id]");
    for (var index = 0; index < rows.length; index += 1) {
      var id = rows[index].getAttribute("data-panel-id");
      var on = layout.off.indexOf(id) === -1;
      var show = rows[index].querySelector('[data-act="show"]');
      if (show) {
        show.classList.toggle("on", on);
        show.setAttribute("aria-checked", String(on));
        show.textContent = on ? "shown" : "hidden";
      }
      var size = layout.size[id] || "m";
      var sizes = rows[index].querySelectorAll("[data-size]");
      for (var inner = 0; inner < sizes.length; inner += 1) {
        var picked = sizes[inner].getAttribute("data-size") === size;
        sizes[inner].classList.toggle("on", picked);
        sizes[inner].setAttribute("aria-checked", String(picked));
      }
    }
  }

  function commitLayout() {
    layoutApi.write(layout);
    layoutApi.apply(layout);
    syncLayoutControls();
    syncBands();
  }

  var panelGroup = document.querySelector('[data-group="panels"]');
  if (panelGroup && layout) {
    panelGroup.addEventListener("click", function (event) {
      var button = closest(event.target, "button");
      var row = closest(button, ".cfg-panel[data-panel-id]");
      if (!button || !row) return;
      var id = row.getAttribute("data-panel-id");
      if (button.getAttribute("data-act") === "show") {
        var position = layout.off.indexOf(id);
        if (position !== -1) layout.off.splice(position, 1);
        else if (layout.off.length < Object.keys(layoutApi.panels).length - 1) layout.off.push(id);
        else return; // The last shown panel stays shown.
      } else if (button.hasAttribute("data-size")) {
        var size = button.getAttribute("data-size");
        if (size === "m") delete layout.size[id];
        else layout.size[id] = size;
      } else {
        return;
      }
      commitLayout();
    });
  }

  var resetButton = document.getElementById("layout-reset");
  if (resetButton && layoutApi) {
    resetButton.addEventListener("click", function () {
      layout = layoutApi.reset();
      syncLayoutControls();
      syncBands();
    });
  }

  /* ---------- panel menu (ACC-VIEW-75-PANEL-MENU) ---------- */

  // Right-click, the ContextMenu key or Shift+F10 on a panel opens the Layout
  // page's choices for it on the same layout state. The menu sits outside the
  // swapped fragments, so a frame neither closes nor rebuilds it.
  var panelMenu = document.getElementById("panel-menu");
  var menuPanel = null;

  function closePanelMenu(refocus) {
    if (panelMenu.hidden) return;
    panelMenu.hidden = true;
    if (refocus && menuPanel.isConnected) menuPanel.focus();
    menuPanel = null;
  }

  function openPanelMenu(panel, x, y) {
    var id = panel.getAttribute("data-panel");
    var size = layout.size[id] || "m";
    // The last shown panel stays shown, as on the Layout page.
    var last = layout.off.length >= Object.keys(layoutApi.panels).length - 1;
    var name = document.querySelector('.cfg-panel[data-panel-id="' + id + '"] .cfg-panel-name');
    panelMenu.setAttribute("aria-label", (name ? name.textContent : id) + " size");
    var items = panelMenu.querySelectorAll('[role="menuitemradio"]');
    for (var index = 0; index < items.length; index += 1) {
      var value = items[index].getAttribute("data-size");
      items[index].classList.toggle("on", value === size);
      items[index].setAttribute("aria-checked", String(value === size));
      if (value === "off") items[index].setAttribute("aria-disabled", String(last));
    }
    menuPanel = panel;
    panelMenu.hidden = false;
    panelMenu.style.left = Math.max(0, Math.min(x, window.innerWidth - panelMenu.offsetWidth)) + "px";
    panelMenu.style.top = Math.max(0, Math.min(y, window.innerHeight - panelMenu.offsetHeight)) + "px";
    panelMenu.querySelector('[aria-checked="true"]').focus();
  }

  if (panelMenu && layout) {
    var menuPanels = document.querySelectorAll(".panel[data-panel]");
    for (var index = 0; index < menuPanels.length; index += 1) menuPanels[index].tabIndex = 0;

    // Text fields keep the browser's menu, so the filter still takes a paste.
    document.addEventListener("contextmenu", function (event) {
      // On the open menu (also the ContextMenu key's keyup event on Windows).
      if (panelMenu.contains(event.target)) return event.preventDefault();
      var panel = closest(event.target, ".panel[data-panel]");
      if (!panel || closest(event.target, "input, select, textarea")) return;
      event.preventDefault();
      openPanelMenu(panel, event.clientX, event.clientY);
    });

    document.addEventListener("keydown", function (event) {
      if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
      var panel = closest(document.activeElement, ".panel[data-panel]");
      if (!panel || closest(document.activeElement, "input, select, textarea")) return;
      event.preventDefault();
      var box = panel.getBoundingClientRect();
      openPanelMenu(panel, box.left + 8, box.top + 8);
    });

    panelMenu.addEventListener("keydown", function (event) {
      var items = Array.prototype.slice.call(panelMenu.querySelectorAll('[role="menuitemradio"]'));
      var at = items.indexOf(document.activeElement);
      var to = { ArrowDown: at + 1, ArrowUp: at - 1, Home: 0, End: items.length - 1 }[event.key];
      if (to !== undefined) {
        items[(to + items.length) % items.length].focus();
      } else if (event.key === "Escape" || event.key === "Tab") {
        closePanelMenu(true);
      } else {
        return; // Enter and Space click the focused item.
      }
      event.preventDefault();
      event.stopPropagation(); // Escape here leaves an open CONFIG popover open.
    });

    panelMenu.addEventListener("click", function (event) {
      var item = closest(event.target, '[role="menuitemradio"]');
      if (!item || item.getAttribute("aria-disabled") === "true") return;
      var id = menuPanel.getAttribute("data-panel");
      var size = item.getAttribute("data-size");
      if (size === "off") layout.off.push(id);
      else if (size === "m") delete layout.size[id];
      else layout.size[id] = size;
      commitLayout();
      closePanelMenu(size !== "off");
    });

    document.addEventListener("pointerdown", function (event) {
      if (!panelMenu.contains(event.target)) closePanelMenu(false);
    });
    window.addEventListener("resize", function () {
      closePanelMenu(false);
    });
  }

  // The tier queries of app.css (drop-p4, drop-p3, drop-p2): a tier change can
  // empty or refill a band, also while paused, when no frame re-runs syncBands.
  if (window.matchMedia) {
    [
      "(max-width: 1100px), (max-height: 640px)",
      "(max-width: 899px), (max-height: 640px)",
      "(max-width: 599px), (max-height: 540px)",
    ].forEach(function (query) {
      var media = window.matchMedia(query);
      if (media.addEventListener) media.addEventListener("change", syncBands);
    });
  }

  /* ---------- GPU selection ---------- */

  var band3 = document.querySelector('.band[data-band="3"]');

  function gpuBoxes() {
    return document.querySelectorAll("#p-gpu .legend-item[data-gpu]");
  }

  function readGpuSelection(count) {
    var all = [];
    for (var index = 0; index < count; index += 1) all.push(index);
    try {
      var stored = JSON.parse(window.localStorage.getItem(GPU_KEY));
      if (!Array.isArray(stored)) return all;
      var kept = all.filter(function (index) {
        return stored.indexOf(index) !== -1;
      });
      return kept.length > 0 ? kept : all;
    } catch (error) {
      return all;
    }
  }

  function writeGpuSelection(selection) {
    try {
      window.localStorage.setItem(GPU_KEY, JSON.stringify(selection));
    } catch (error) {
      // Storage is optional; the selection holds for this page view.
    }
  }

  function gpuChip(group, index) {
    var chip = group.querySelector('.opt[data-value="' + index + '"]');
    if (chip) return chip;
    chip = document.createElement("button");
    chip.className = "opt";
    chip.type = "button";
    chip.setAttribute("role", "checkbox");
    chip.setAttribute("data-value", String(index));
    chip.textContent = String(index);
    var model = document.querySelector('#p-gpu .legend-item[data-gpu="' + index + '"] .lg-model');
    if (model) chip.title = model.textContent;
    group.appendChild(chip);
    return chip;
  }

  function syncGpuChips(selection) {
    var group = document.querySelector('[data-group="gpu"]');
    if (!group) return;
    var boxes = gpuBoxes();
    var chips = group.querySelectorAll(".opt");
    for (var index = chips.length - 1; index >= 0; index -= 1) {
      if (Number(chips[index].getAttribute("data-value")) >= boxes.length) group.removeChild(chips[index]);
    }
    for (var index = 0; index < boxes.length; index += 1) {
      var chip = gpuChip(group, index);
      var on = selection.indexOf(index) !== -1;
      chip.classList.toggle("on", on);
      chip.setAttribute("aria-checked", String(on));
    }
  }

  function applyGpuSelection() {
    var boxes = gpuBoxes();
    if (boxes.length === 0) {
      gpuSelection = [];
      if (band3) band3.style.removeProperty("--gpu-lanes");
      syncGpuChips(gpuSelection);
      return;
    }
    var selection = gpuSelection.filter(function (index) {
      return index < boxes.length;
    });
    if (selection.length === 0) {
      for (var index = 0; index < boxes.length; index += 1) selection.push(index);
    }
    gpuSelection = selection;
    // The lane of a GPU and its legend entry both carry data-gpu.
    var items = document.querySelectorAll("#p-gpu [data-gpu]");
    for (var index = 0; index < items.length; index += 1) {
      var on = selection.indexOf(Number(items[index].getAttribute("data-gpu"))) !== -1;
      items[index].setAttribute("data-selected", String(on));
    }
    // Band 3 reserves 16 px per visible lane (app.css, .band[data-band="3"]).
    if (band3) band3.style.setProperty("--gpu-lanes", String(selection.length));
    syncGpuChips(selection);
  }

  var gpuGroup = document.querySelector('[data-group="gpu"]');
  if (gpuGroup) {
    gpuGroup.addEventListener("click", function (event) {
      var chip = closest(event.target, ".opt");
      if (!chip || !gpuGroup.contains(chip)) return;
      var index = Number(chip.getAttribute("data-value"));
      var position = gpuSelection.indexOf(index);
      if (position === -1) gpuSelection.push(index);
      else if (gpuSelection.length > 1) gpuSelection.splice(position, 1);
      else return; // The last visible GPU stays visible.
      gpuSelection.sort(function (a, b) {
        return a - b;
      });
      writeGpuSelection(gpuSelection);
      applyGpuSelection();
    });
  }

  /* ---------- pause ---------- */

  function rememberSnapshot(event) {
    for (var index = 0; index < pending.length; index += 1) {
      if (pending[index].target === event.target) {
        pending[index].data = event.detail.data;
        return;
      }
    }
    pending.push({ target: event.target, data: event.detail.data });
  }

  function flushSnapshots() {
    var latest = pending;
    pending = [];
    for (var index = 0; index < latest.length; index += 1) {
      if (!document.body.contains(latest[index].target)) continue;
      htmx.swap(latest[index].target, latest[index].data, { swapStyle: "innerHTML" });
    }
    afterFrames();
  }

  if (pauseButton) {
    pauseButton.addEventListener("click", function () {
      paused = !paused;
      paintLive();
      if (!paused) flushSnapshots();
    });
  }

  /* ---------- process list: view, tree, filter and sort (ACC-VIEW-58-PROC-VIEW) ---------- */

  // The server sends every row in btop's tree pre-order with its depth, raw
  // values (data-raw) and, on parents, the subtree's labels (data-sum); one
  // fragment serves every viewer. This viewer's view, collapse set and sort
  // live in one object, stored as JSON under PROC_KEY. ACC-CONF-61 moves the
  // controls into a config page and reuses it; they are found by data-group.
  var PROC_KEY = "btop-web-proc";
  var PROC_VIEWS = ["top", "all", "tree"];
  // The sort key of each column, in proc.html's cell order.
  var PROC_KEYS = ["pid", "name", "command", "threads", "user", "mem", "cpu"];
  var procWrap = document.getElementById("proc-table-wrap");
  var procView = readProcView();
  var procHold = false;
  var procFocus = null;
  var collator = new Intl.Collator();

  // view: the stored choice, else the server's starting view (#p-proc
  // data-proc-view); `chosen` keeps a viewer who never picked a view on it.
  function readProcView() {
    var panel = document.getElementById("p-proc");
    var start = panel ? panel.getAttribute("data-proc-view") : null;
    var state = { view: PROC_VIEWS.indexOf(start) !== -1 ? start : "top", chosen: null, nodes: "expanded", toggled: {}, sort: "mem", descending: true };
    try {
      var stored = JSON.parse(window.localStorage.getItem(PROC_KEY));
      if (!stored || stored.v !== 1) return state;
      if (PROC_VIEWS.indexOf(stored.view) !== -1) state.view = state.chosen = stored.view;
      if (stored.nodes === "collapsed" || stored.nodes === "expanded") state.nodes = stored.nodes;
      if (stored.toggled && typeof stored.toggled === "object") {
        Object.keys(stored.toggled).forEach(function (pid) {
          if (typeof stored.toggled[pid] === "boolean") state.toggled[pid] = stored.toggled[pid];
        });
      }
      if (PROC_KEYS.indexOf(stored.sort) !== -1) state.sort = stored.sort;
      if (typeof stored.descending === "boolean") state.descending = stored.descending;
    } catch (error) {
      // Storage is optional; the defaults hold for this page view.
    }
    return state;
  }

  // Written only on a user action.
  function commitProc() {
    try {
      window.localStorage.setItem(PROC_KEY, JSON.stringify({ v: 1, view: procView.chosen || undefined, nodes: procView.nodes, toggled: procView.toggled, sort: procView.sort, descending: procView.descending }));
    } catch (error) {
      // Storage is optional; the choice holds for this page view.
    }
    syncProcControls();
    applyProc();
  }

  // The config menu's sort select and direction chips mirror the column headers.
  var procSort = document.getElementById("proc-sort");

  function syncProcControls() {
    var chosen = { "proc-view": procView.view, "proc-nodes": procView.nodes, "proc-dir": procView.descending ? "desc" : "asc" };
    var chips = document.querySelectorAll('[data-group="proc-view"] .opt, [data-group="proc-nodes"] .opt, [data-group="proc-dir"] .opt');
    for (var index = 0; index < chips.length; index += 1) {
      var group = closest(chips[index], "[data-group]").getAttribute("data-group");
      var on = chips[index].getAttribute("data-value") === chosen[group];
      chips[index].classList.toggle("on", on);
      chips[index].setAttribute("aria-checked", String(on));
    }
    if (procSort) procSort.value = procView.sort;
  }

  // ACC-VIEW-59-PROC-SORT: text columns start A to Z, numbers largest first.
  function sortBy(key) {
    procView.sort = key;
    procView.descending = ["name", "command", "user"].indexOf(key) === -1;
  }

  function rawNumber(token) {
    return token === undefined || token === "-" ? null : Number(token);
  }

  // A null total is a subtree with no value; an unavailable value adds nothing.
  function plus(one, other) {
    return one === null ? other : other === null ? one : one + other;
  }

  // One row's data, parsed once per frame.
  function procRow(row) {
    var cells = row.cells;
    var raw = (row.getAttribute("data-raw") || "").split(" ");
    var cpuText = cells[6].lastChild;
    if (!cpuText || cpuText.nodeType !== 3) cpuText = cells[6].appendChild(document.createTextNode(""));
    var p = {
      row: row,
      pid: Number(cells[0].textContent),
      depth: Number(raw[0]) || 0,
      own: { cpu: rawNumber(raw[1]), mem: rawNumber(raw[2]), threads: rawNumber(raw[3]) },
      // Cpu%, MemB and Threads labels and the Cpu% class, own and summed.
      ownLabels: [cpuText.nodeValue, cells[5].textContent, cells[3].textContent, cells[6].className],
      sumLabels: null,
      // btop filters on pid, name, command and user: cells 0, 1, 2 and 4.
      texts: [cells[0].textContent, cells[1].textContent, cells[2].textContent, cells[4].textContent],
      cpuText: cpuText
    };
    var sum = row.getAttribute("data-sum");
    if (sum) {
      p.sumLabels = sum.split(" ").map(function (label) {
        return label === "-" ? "" : label;
      });
      p.sumLabels[3] = "num" + (p.sumLabels[3] ? " " + p.sumLabels[3] : "");
    }
    p.search = p.texts.map(function (text) {
      return text.toLowerCase();
    });
    row._p = p;
    return p;
  }

  function procValue(p) {
    var values = p.collapsed ? p.sums : p.own;
    switch (procView.sort) {
      case "pid": return p.pid;
      case "name": return p.texts[1];
      case "command": return p.texts[2];
      case "user": return p.texts[3];
      case "threads": return values.threads;
      case "cpu": return values.cpu;
      default: return values.mem;
    }
  }

  // Stable, so ties keep the server's order; an unavailable value sorts last.
  function sortProc(rows) {
    return rows.sort(function (one, other) {
      var a = procValue(one);
      var b = procValue(other);
      if (a === null || b === null) return a === b ? 0 : a === null ? 1 : -1;
      var order = typeof a === "string" ? collator.compare(a, b) : a - b;
      return procView.descending ? -order : order;
    });
  }

  function treeGlyphs(p, text) {
    var cell = p.row.cells[1];
    if (!p.tree) {
      p.tree = document.createElement("span");
      p.tree.className = "tree";
      p.tree.setAttribute("aria-hidden", "true");
    }
    if (p.tree.textContent !== text) p.tree.textContent = text;
    if (cell.firstChild !== p.tree) cell.insertBefore(p.tree, cell.firstChild);
    if (p.kids.length === 0) {
      if (p.twisty && p.twisty.parentNode) p.twisty.parentNode.removeChild(p.twisty);
      return;
    }
    if (!p.twisty) {
      p.twisty = document.createElement("button");
      p.twisty.type = "button";
      p.twisty.className = "twisty";
      p.twisty.setAttribute("aria-label", "children of " + p.texts[1] + " (pid " + p.pid + ")");
    }
    var mark = p.collapsed ? "\u25b8" : "\u25be";
    if (p.twisty.textContent !== mark) {
      p.twisty.setAttribute("aria-expanded", String(!p.collapsed));
      p.twisty.textContent = mark;
    }
    if (p.tree.nextSibling !== p.twisty) cell.insertBefore(p.twisty, p.tree.nextSibling);
  }

  function dropGlyphs(p) {
    [p.tree, p.twisty].forEach(function (node) {
      if (node && node.parentNode) node.parentNode.removeChild(node);
    });
  }

  // ACC-PERF-16-NO-OVERHEAD: a frame updates the table in place instead of
  // replacing it. A row keeps its element, tree prefix and triangle, and only
  // what the server changed is written, so the browser lays out and paints
  // the changed rows only. The head is static and stays as it is.
  // Own code: htmx's morph extension would move every row back to the
  // server's order and strip the prefix and triangle on each frame.
  function mergeProc(body, next) {
    var live = {};
    (body._rows || Array.prototype.map.call(body.rows, procRow)).forEach(function (p) {
      live[p.pid] = p;
    });
    body._rows = Array.prototype.slice.call(next.rows).map(function (row) {
      var q = procRow(row);
      var p = live[q.pid];
      delete live[q.pid];
      // A new pid, or a reused one with another program, takes the server's row.
      if (!p || p.texts.join("\n") !== q.texts.join("\n")) {
        if (p) body.removeChild(p.row);
        body.appendChild(row);
        return q;
      }
      ["data-raw", "data-sum"].forEach(function (name) {
        var value = row.getAttribute(name);
        if (value === null) p.row.removeAttribute(name);
        else if (p.row.getAttribute(name) !== value) p.row.setAttribute(name, value);
      });
      var cell = p.row.cells[6];
      if (procGraphs(cell) !== procGraphs(row.cells[6])) {
        while (cell.firstChild !== p.cpuText) cell.removeChild(cell.firstChild);
        while (row.cells[6].firstChild !== q.cpuText) cell.insertBefore(row.cells[6].firstChild, p.cpuText);
      }
      // applyProc writes the labels the view shows from these.
      p.depth = q.depth;
      p.own = q.own;
      p.ownLabels = q.ownLabels;
      p.sumLabels = q.sumLabels;
      return p;
    });
    Object.keys(live).forEach(function (pid) {
      body.removeChild(live[pid].row);
    });
  }

  // The graphs of a Cpu% cell as the server wrote them; the text comes last.
  function procGraphs(cell) {
    var html = "";
    for (var node = cell.firstChild; node && node.nodeType === 1; node = node.nextSibling) html += node.outerHTML;
    return html;
  }

  function applyProc() {
    var body = procWrap ? procWrap.querySelector("tbody") : null;
    if (!body) return;
    var focus = procFocus || procFocusKey(document.activeElement);
    procFocus = null;
    // The server's pre-order, kept on the tbody: a frame brings a new one.
    var rows = body._rows || (body._rows = Array.prototype.map.call(body.rows, procRow));
    var query = filterInput ? filterInput.value.trim().toLowerCase() : "";
    var view = procView.view;
    var stack = [];
    rows.forEach(function (p) {
      stack.length = Math.min(stack.length, p.depth);
      p.parent = stack.length > 0 ? stack[stack.length - 1] : null;
      p.kids = [];
      if (p.parent) p.parent.kids.push(p);
      stack.push(p);
      p.sums = { cpu: p.own.cpu, mem: p.own.mem, threads: p.own.threads };
      p.match = query.length === 0 || p.search.some(function (text) {
        return text.indexOf(query) !== -1;
      });
      p.under = false;
    });
    // Pre-order puts every descendant after its parent: one reverse pass sums.
    for (var index = rows.length - 1; index >= 0; index -= 1) {
      var up = rows[index].parent;
      if (!up) continue;
      up.sums.cpu = plus(up.sums.cpu, rows[index].sums.cpu);
      up.sums.mem = plus(up.sums.mem, rows[index].sums.mem);
      up.sums.threads = plus(up.sums.threads, rows[index].sums.threads);
      if (rows[index].match || rows[index].under) up.under = true;
    }

    var order = [];
    var glyphs = {};
    rows.forEach(function (p) {
      var parent = p.kids.length > 0;
      if (view === "all") {
        p.collapsed = false;
        p.shown = p.match;
      } else if (view === "top") {
        // pid 1's children are top-level rows of their own.
        p.collapsed = parent && p.pid !== 1;
        p.shown = (p.parent === null || p.parent.pid === 1) && (p.match || (p.collapsed && p.under));
      } else {
        var stored = Object.prototype.hasOwnProperty.call(procView.toggled, p.pid) ? procView.toggled[p.pid] : procView.nodes === "collapsed";
        // A query opens the ancestors of a match without touching the stored set.
        p.collapsed = parent && stored && !(query && p.under);
        p.shown = false;
      }
      if (view !== "tree" && p.shown) order.push(p);
    });
    if (view === "tree") {
      // Shown rows only: a row whose parent is collapsed is never reached.
      var walk = function (siblings, prefix) {
        var shown = sortProc(siblings).filter(function (p) {
          return p.match || p.under;
        });
        shown.forEach(function (p, at) {
          p.shown = true;
          order.push(p);
          // btop's tree prefix (btop_shared.cpp:181-188, btop_collect.cpp:2754-2760), one cell per level.
          var mark = p.kids.length > 0 ? "" : at === shown.length - 1 ? "\u2514" : p.depth === 0 && at === 0 ? "\u250c" : "\u251c";
          glyphs[p.pid] = prefix + mark;
          if (p.kids.length > 0 && !p.collapsed) walk(p.kids, prefix + "\u2502");
        });
      };
      walk(rows.filter(function (p) {
        return p.parent === null;
      }), "");
    } else {
      sortProc(order);
    }

    // A row already in place stays: a moved row is styled and laid out again.
    var at = body.firstElementChild;
    order.concat(rows.filter(function (p) {
      return !p.shown;
    })).forEach(function (p) {
      var row = p.row;
      if (row.hidden === p.shown) row.hidden = !p.shown;
      // The server graphs a top-level row's subtree only, so a deeper collapsed row is marked to show no graph.
      var collapsed = !p.collapsed ? null : p.parent === null || p.parent.pid === 1 ? "" : "deep";
      if (row.getAttribute("data-collapsed") !== collapsed) {
        if (collapsed === null) row.removeAttribute("data-collapsed");
        else row.setAttribute("data-collapsed", collapsed);
      }
      var labels = p.collapsed && p.sumLabels ? p.sumLabels : p.ownLabels;
      if (p.cpuText.nodeValue !== labels[0]) p.cpuText.nodeValue = labels[0];
      if (row.cells[5].textContent !== labels[1]) row.cells[5].textContent = labels[1];
      if (row.cells[3].textContent !== labels[2]) row.cells[3].textContent = labels[2];
      if (row.cells[6].className !== labels[3]) row.cells[6].className = labels[3];
      if (view === "tree" && p.shown) treeGlyphs(p, glyphs[p.pid]);
      else dropGlyphs(p);
      if (row === at) at = at.nextElementSibling;
      else body.insertBefore(row, at);
    });
    // ACC-VIEW-59-PROC-SORT: the active column carries the direction.
    var heads = procWrap.querySelectorAll("thead th");
    for (var col = 0; col < heads.length; col += 1) {
      if (PROC_KEYS[col] === procView.sort) heads[col].setAttribute("aria-sort", procView.descending ? "descending" : "ascending");
      else heads[col].removeAttribute("aria-sort");
    }
    if (procCount) procCount.textContent = order.length + "/" + rows.length;
    restoreProcFocus(focus, rows);
  }

  // The control a keyboard user is on, as a key that survives a new table.
  function procFocusKey(element) {
    if (!element || !procWrap || !procWrap.contains(element)) return null;
    var row = closest(element, "tr");
    if (element.classList.contains("twisty") && row && row._p) return { pid: row._p.pid };
    if (element.tagName === "BUTTON" && element.parentNode.tagName === "TH") return { col: element.parentNode.cellIndex };
    return null;
  }

  function restoreProcFocus(key, rows) {
    if (!key) return;
    var head = key.col !== undefined ? procWrap.querySelectorAll("thead th")[key.col] : null;
    var target = head ? head.querySelector("button") : null;
    rows.forEach(function (p) {
      if (key.pid !== undefined && p.pid === key.pid && p.twisty && p.twisty.isConnected) target = p.twisty;
    });
    if (target && document.activeElement !== target) target.focus({ preventScroll: true });
  }

  if (procWrap) {
    procWrap.addEventListener("click", function (event) {
      var head = closest(event.target, "th > button");
      if (head) {
        var key = PROC_KEYS[head.parentNode.cellIndex];
        if (key === procView.sort) procView.descending = !procView.descending;
        else sortBy(key);
        commitProc();
        return;
      }
      var twisty = closest(event.target, ".twisty");
      var row = closest(twisty, "tr");
      if (!twisty || !row || !row._p) return;
      procView.toggled[row._p.pid] = !row._p.collapsed;
      commitProc();
    });
    // A frame that lands between press and release can move or replace the
    // control, and the browser would dispatch no click: hold proc frames
    // while a control is pressed, then apply the newest.
    procWrap.addEventListener("pointerdown", function (event) {
      if (closest(event.target, "button")) procHold = true;
    });
    procWrap.addEventListener("keydown", function (event) {
      if ((event.key === " " || event.key === "Enter") && closest(event.target, "button")) procHold = true;
    });
    ["pointerup", "pointercancel", "keyup"].forEach(function (type) {
      document.addEventListener(type, function () {
        if (!procHold) return;
        procHold = false;
        // After the click this release starts has been dispatched.
        window.setTimeout(function () {
          if (!paused) flushSnapshots();
        }, 0);
      });
    });
    // A proc frame, live or flushed after a pause, merges into the table.
    procWrap.addEventListener("htmx:oobBeforeSwap", function (event) {
      var body = procWrap.querySelector("tbody");
      var next = event.target === procWrap ? event.detail.fragment.querySelector("tbody") : null;
      if (!body || !next) return;
      event.detail.shouldSwap = false;
      mergeProc(body, next);
    });
  }

  if (procSort) {
    procSort.addEventListener("change", function () {
      if (PROC_KEYS.indexOf(procSort.value) === -1) return;
      sortBy(procSort.value);
      commitProc();
    });
  }

  document.addEventListener("click", function (event) {
    var chip = closest(event.target, '[data-group="proc-view"] .opt, [data-group="proc-nodes"] .opt, [data-group="proc-dir"] .opt');
    if (!chip) return;
    var value = chip.getAttribute("data-value");
    if (closest(chip, '[data-group="proc-view"]')) {
      procView.view = procView.chosen = value;
    } else if (closest(chip, '[data-group="proc-dir"]')) {
      procView.descending = value === "desc";
    } else {
      // The option sets every node; it clears the per-row overrides.
      procView.nodes = value;
      procView.toggled = {};
    }
    commitProc();
  });

  if (filterInput) {
    filterInput.addEventListener("input", function () {
      window.clearTimeout(filterTimer);
      filterTimer = window.setTimeout(applyProc, 150);
    });
    filterInput.addEventListener("keydown", function (event) {
      if (event.key !== "Escape") return;
      event.preventDefault();
      window.clearTimeout(filterTimer);
      filterInput.value = "";
      applyProc();
    });
  }

  /* ---------- process table: whole rows at rest ---------- */

  // A whole number of rows fills the table area: each row stretches or shrinks
  // by under half a row from var(--row-h), so the last row at rest is never cut
  // and no blank band is left under it. The panel's size does not depend on its
  // content, so observing it catches viewport, band and gadget changes.
  var procPanel = document.getElementById("p-proc");

  function fitProc() {
    var body = procWrap.querySelector("tbody");
    if (!body) return;
    procWrap.style.maxHeight = "";
    var rowH = parseFloat(window.getComputedStyle(procWrap).getPropertyValue("--row-h"));
    var box = procWrap.getBoundingClientRect();
    var head = body.getBoundingClientRect().top - box.top + procWrap.scrollTop;
    var rows = Math.max(1, Math.round((box.height - head) / rowH));
    // Floored to the 1/64 px layout unit; the wrap then ends on the last row's edge.
    var fit = Math.floor(((box.height - head) / rows) * 64) / 64;
    procWrap.style.setProperty("--proc-row-h", fit + "px");
    procWrap.style.maxHeight = head + rows * fit + "px";
  }

  if (procPanel && procWrap) new ResizeObserver(fitProc).observe(procPanel);

  // The GPU legend drops temperature and power by its lane's height (app.css,
  // @container gpu-lane), which no tier query sees, so the tier chip names the
  // drop (ACC-LAYOUT-21). The gpu fragment swaps only the panel's children.
  var tierChip = document.getElementById("tier-chip");
  var gpuPanel = document.getElementById("p-gpu");

  function syncGpuDrop() {
    var meta = gpuPanel.querySelector('.gpu-lane:not([data-selected="false"]) .legend-meta');
    tierChip.classList.toggle("gpu-drop", meta !== null && window.getComputedStyle(meta).display === "none");
  }

  if (tierChip && gpuPanel) new ResizeObserver(syncGpuDrop).observe(gpuPanel);

  /* ---------- initial paint and re-apply after each frame ---------- */

  function afterFrames() {
    syncRescale();
    applyGpuSelection();
    applyProc();
    syncBands();
    refreshTip();
  }

  // ACC-GRAPH-67-SMOOTH-SCALE: a scale change runs on this page's clock. Each
  // frame replaces a plot's series group, so the new group's animation is set
  // to the time since the change began here; a page that joins mid-change
  // starts at the server's elapsed time.
  var rescaleStart = {};

  function syncRescale() {
    var now = document.timeline.currentTime;
    var groups = document.querySelectorAll("[data-chart] [data-rescale]");
    for (var index = 0; index < groups.length; index += 1) {
      var name = closest(groups[index], "[data-chart]").getAttribute("data-chart");
      var parts = groups[index].getAttribute("data-rescale").split(" ");
      var start = rescaleStart[name];
      if (!start || start.id !== parts[0]) {
        start = rescaleStart[name] = { id: parts[0], at: now - Number(parts[1]) };
      }
      var animations = groups[index].getAnimations();
      if (animations.length) animations[0].currentTime = now - start.at;
    }
  }

  // Fires after a live frame's out-of-band swaps have landed, before paint.
  document.body.addEventListener("htmx:sseMessage", function (event) {
    var type = event.detail.type;
    syncRescale();
    if (type === "gpu") {
      applyGpuSelection();
      syncBands();
    } else if (type === "proc") {
      applyProc();
    } else if (type === "mem") {
      syncBands();
    }
    refreshTip();
  });

  paintClock();
  paintLive();
  armStale();
  syncLayoutControls();
  syncProcControls();
  syncMotionControls();
  gpuSelection = readGpuSelection(gpuBoxes().length);
  afterFrames();
})();

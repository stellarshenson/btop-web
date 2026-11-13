// Dashboard interactions: the live state, the CONFIG popover, panel layout,
// GPU selection, pause, the process list (view, tree, filter, sort) and the
// process table fit. One delegated listener per event; the only timers are
// the filter debounce, the stale timeout, which is re-armed once per frame (a
// stream that stops sends no event to notice), and the process rows' merge,
// one per slow pass.
// Theme, mode and the stored layout load in theme.js.
(function () {
  "use strict";

  var GPU_KEY = "btop-web-gpu-selection";
  var config = document.getElementById("config");
  var scrim = document.getElementById("config-scrim");
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
  // ACC-PERF-149-SPLIT-CADENCE: charts and the process list rebuild at the
  // slow period, not the value tick.
  var slowMs = interval ? Number(interval.getAttribute("data-slow-ms")) || updateMs : updateMs;

  var lastTrigger = null;
  var paused = false;
  var stale = false;
  var staleTimer = 0;
  var lastFrame = 0;
  var lastChartAt = 0; // performance.now() of the last slow pass, the chart clock
  var slowSeq = -1;
  var pending = [];
  // The newest compact cpu and mem records held while paused, by event name.
  var pendingFrames = {};
  var filterTimer = 0;
  var gpuSelection = [];

  function closest(node, selector) {
    return node && node.closest ? node.closest(selector) : null;
  }

  /* ---------- tooltips (ACC-VIEW-223-TIP-MINIMAL) ---------- */

  // The server writes each stat's text into data-tip; this code only places
  // the one box. A process cell takes its column header's text, so a table of
  // hundreds of rows repeats no attribute. The box opens after the pointer has
  // rested TIP_WAIT_MS on one stat (DEF-VIEW-64), TIP_GAP right and TIP_DROP
  // below the pointer so it covers neither the cursor nor the stat's own text
  // (ACC-VIEW-111), flipped left or up only when it would leave the viewport
  // and kept TIP_PAD inside it (DEF-VIEW-75); once open it follows the pointer
  // while the pointer stays on the same stat. A frame replaces the stat under
  // a still pointer, so the box re-reads the text the server now sends.
  var TIP_WAIT_MS = 1500; // pointer rest on one stat before the box opens
  var TIP_GAP = 12; // px from the pointer to the box's near side
  var TIP_DROP = 16; // px from the pointer to the box's top
  var TIP_PAD = 4; // px between the box and the viewport edge
  var tip = document.getElementById("tip");
  var tipAt = null; // the open box: { key, text }
  var tipWait = 0; // the pointer-rest timer
  var tipWant = null; // the stat the pointer rests on, not yet open
  var tipX = 0;
  var tipY = 0;

  // DEF-VIEW-148-TIP-COPIES: a node this code creates reads its text from the
  // page's #tips, which the server renders once. A memory text has one
  // attribute per tip set the mem record names, and a text without one for a
  // set takes the plain attribute. The mem record's set is its third field
  // from the end, where a 1.0.4 page reads no tip (DEF-VIEW-150-MIXED-TIP).
  // The load tip is the cpu record's slot 12 (DEF-VIEW-152-LOAD-TIP).
  var tipTexts = document.getElementById("tips");
  function tipText(key, set) {
    return (set && tipTexts.getAttribute("data-" + key + "-" + set)) || tipTexts.getAttribute("data-" + key);
  }

  // A swapped fragment repeats the elements but not the nodes, so a stat's
  // identity through a frame is its place in the document.
  function tipEntry(el, text) {
    var key = "";
    for (var node = el; node && node !== document.body; node = node.parentElement) {
      var index = 0;
      for (var prev = node.previousElementSibling; prev; prev = prev.previousElementSibling) index += 1;
      key += "/" + index;
    }
    return { el: el, key: key, text: text };
  }

  function tipSource(node) {
    if (!node || !node.closest) return null;
    // A plot box carries no tooltip, and neither does its contents.
    if (node.closest("[data-chart]")) return null;
    var own = node.closest("[data-tip]");
    if (own) return tipEntry(own, own.getAttribute("data-tip"));
    var cell = node.closest("td");
    var table = cell ? cell.closest("table.ptable") : null;
    if (!table) return null;
    var head = table.querySelectorAll("thead th")[cell.cellIndex];
    if (!head || !head.hasAttribute("data-tip")) return null;
    return tipEntry(cell, head.getAttribute("data-tip"));
  }

  // The one text run under the pointer: the smallest client rect of a direct
  // text node inside the anchor that contains the pointer. A gauge carries
  // TOTAL and its tick labels across its height, so the union of every run
  // moved the box below the whole gauge (DEF-VIEW-83); only the run actually
  // under the pointer is avoided, and with none under it the box opens at the
  // pointer. A text node's rect is its glyphs, so an element's reserved width
  // (data-w) is not mistaken for text.
  function textRunAt(anchor) {
    var nodes = anchor.querySelectorAll("*");
    var all = [anchor];
    for (var index = 0; index < nodes.length; index += 1) all.push(nodes[index]);
    var under = null;
    for (index = 0; index < all.length; index += 1) {
      for (var child = all[index].firstChild; child; child = child.nextSibling) {
        if (child.nodeType !== 3 || !child.nodeValue.trim()) continue;
        var range = document.createRange();
        range.selectNodeContents(child);
        var boxes = range.getClientRects();
        for (var at = 0; at < boxes.length; at += 1) {
          var box = boxes[at];
          if (box.width <= 0 || box.height <= 0) continue;
          if (tipX < box.left || tipX > box.right || tipY < box.top || tipY > box.bottom) continue;
          if (!under || box.width * box.height < under.width * under.height) under = box;
        }
      }
    }
    return under;
  }

  // The box opens TIP_GAP right and TIP_DROP below the pointer. It flips to
  // the pointer's left or above it only when that side would leave the
  // viewport, and it stays TIP_PAD inside the viewport. Where the pointer sits
  // on a text run, the box moves just below that run, or above it when below
  // is off screen, so the hovered value stays readable.
  function placeTip(anchor) {
    var width = tip.offsetWidth;
    var height = tip.offsetHeight;
    var vw = window.innerWidth;
    var vh = window.innerHeight;
    var x = tipX + TIP_GAP;
    if (x + width > vw - TIP_PAD) x = tipX - TIP_GAP - width;
    var y = tipY + TIP_DROP;
    if (y + height > vh - TIP_PAD) y = tipY - TIP_DROP - height;
    var text = textRunAt(anchor);
    if (text && x < text.right && x + width > text.left && y < text.bottom && y + height > text.top) {
      var below = text.bottom + TIP_GAP;
      y = below + height <= vh - TIP_PAD ? below : text.top - TIP_GAP - height;
    }
    put(
      Math.max(TIP_PAD, Math.min(x, vw - TIP_PAD - width)),
      Math.max(TIP_PAD, Math.min(y, vh - TIP_PAD - height))
    );
  }

  function put(x, y) {
    tip.style.left = x + "px";
    tip.style.top = y + "px";
  }

  function hideTip() {
    window.clearTimeout(tipWait);
    tipWait = 0;
    tipWant = null;
    tipAt = null;
    if (tip) tip.hidden = true;
  }

  function openTip(source) {
    tipAt = { key: source.key, text: source.text };
    tipWant = null;
    // DEF-VIEW-84: the first line is strong, a second line muted.
    var cut = source.text.indexOf("\n");
    var first = document.createElement("span");
    first.className = "tip-first";
    first.textContent = cut < 0 ? source.text : source.text.slice(0, cut);
    tip.textContent = "";
    tip.appendChild(first);
    if (cut >= 0) {
      var rest = document.createElement("span");
      rest.className = "tip-src";
      rest.textContent = source.text.slice(cut + 1);
      tip.appendChild(rest);
    }
    tip.hidden = false;
    tip.style.visibility = "hidden";
    placeTip(source.el);
    tip.style.visibility = "";
  }

  // The rest timer fired: open only if the pointer is still on that stat.
  function openRest() {
    tipWait = 0;
    var want = tipWant;
    tipWant = null;
    var source = tipSource(document.elementFromPoint(tipX, tipY));
    if (!source || (want && source.key !== want.key)) return;
    openTip(source);
  }

  // A frame replaces the stat under a still pointer; re-read its text.
  function refreshTip() {
    if (!tip || !tipAt || tip.hidden) return;
    var source = tipSource(document.elementFromPoint(tipX, tipY));
    if (!source || source.key !== tipAt.key) return hideTip();
    if (source.text !== tipAt.text) openTip(source);
  }

  if (tip) {
    document.addEventListener("pointermove", function (event) {
      tipX = event.clientX;
      tipY = event.clientY;
      var source = tipSource(event.target);
      if (!source) return hideTip();
      if (tipAt && tipAt.key === source.key) return placeTip(source.el);
      if (tipWant && tipWant.key === source.key) return;
      hideTip();
      tipWant = source;
      tipWait = window.setTimeout(openRest, TIP_WAIT_MS);
    });
    document.addEventListener("pointerdown", hideTip);
    document.addEventListener("keydown", function (event) {
      if (event.key === "Escape") hideTip();
    });
    document.documentElement.addEventListener("pointerleave", hideTip);
    window.addEventListener("blur", hideTip);
    window.addEventListener("resize", hideTip);
    // Capture, because the process table scrolls inside the page.
    window.addEventListener("scroll", hideTip, true);
  }

  /* ---------- clock and live state ---------- */

  function paintClock() {
    var now = new Date();
    var text = String(now.getHours()).padStart(2, "0") + ":" + String(now.getMinutes()).padStart(2, "0");
    var clocks = document.querySelectorAll(".hdr-clock");
    for (var index = 0; index < clocks.length; index += 1) {
      if (clocks[index].textContent !== text) clocks[index].textContent = text;
    }
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
    // ACC-VIEW-120: a stopped page shows the true value at once; nothing
    // queues while it is stopped.
    if (stale || paused) cancelNumberRuns();
    paintDetailStale();
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

  /* ---------- copy the telemetry as JSON (ACC-VIEW-182-COPY-JSON) ---------- */

  // The button copies the server's own document from the JSON stream's once
  // mode, so the clipboard holds the same object every stream consumer gets
  // and the page keeps no second serialiser. The fetch runs on click only,
  // so the frame loop never waits on it.
  var copyButton = document.getElementById("copy-json");
  var copyStatus = document.getElementById("copy-status");

  // DEF-VIEW-122-POPUP-COPY: both copy chips return to rest after 1.5 s, each on
  // its own timer. `rest` is the popup chip's word; the top bar's chip is an icon.
  function flashCopied(button, status, rest) {
    button.classList.add("copied");
    if (rest) button.textContent = "copied";
    if (copyStatus) copyStatus.textContent = status;
    window.clearTimeout(button._copyTimer);
    button._copyTimer = window.setTimeout(function () {
      button.classList.remove("copied");
      if (rest) button.textContent = rest;
      if (copyStatus) copyStatus.textContent = "";
    }, 1500);
  }

  // The selection fallback for a browser that refuses the async clipboard or
  // does not offer it (an insecure origin, a denied permission).
  function selectCopy(text) {
    var back = document.activeElement;
    var area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.top = "-1000px";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    var copied = false;
    try {
      copied = document.execCommand("copy");
    } catch (error) {
      copied = false;
    }
    document.body.removeChild(area);
    if (back && back.focus) back.focus();
    if (!copied) throw new Error("the browser refused the copy");
  }

  function writeClipboard(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text).catch(function () {
        selectCopy(text);
      });
    }
    selectCopy(text);
    return Promise.resolve();
  }

  // ACC-VIEW-210-COPY-HELD: the newest telemetry event, unparsed, and the time
  // it arrived. A click writes it in its own task, with no server request.
  var heldFrame = null;
  var heldAt = 0;

  function holdFrame(event) {
    heldFrame = event.data;
    heldAt = Date.now();
  }

  if (copyButton) {
    copyButton.addEventListener("click", function () {
      new Promise(function (resolve) {
        if (heldFrame === null) throw new Error("no frame to copy");
        resolve(writeClipboard('{"age_ms":' + (Date.now() - heldAt) + "," + heldFrame.slice(1)));
      })
        .then(function () {
          flashCopied(copyButton, "Telemetry copied as JSON");
        })
        .catch(function () {
          if (copyStatus) copyStatus.textContent = "Copy failed";
        });
    });
  }

  // DEF-VIEW-119-STALE-TAB: each new /events connection states the server's
  // build stamp once. A page from another build reloads, so new markup never
  // lands under an old stylesheet. One reload per stamp: a server that keeps
  // answering with another stamp cannot loop the page.
  var pageStamp = new URL(document.currentScript.src).searchParams.get("v");

  function checkStamp(event) {
    if (event.data === pageStamp) return;
    try {
      if (window.sessionStorage.getItem("btop-web-reloaded-for") === event.data) return;
      window.sessionStorage.setItem("btop-web-reloaded-for", event.data);
    } catch (error) {
      // Storage is optional; without it the stamp comparison alone holds.
    }
    window.location.reload();
  }

  // A reconnect of the same EventSource opens again; the same function adds once.
  document.body.addEventListener("htmx:sseOpen", function (event) {
    event.detail.source.addEventListener("stamp", checkStamp);
    event.detail.source.addEventListener("telemetry", holdFrame);
  });

  // Fires for every frame, paused or not, before the frame is applied.
  document.body.addEventListener("htmx:sseBeforeMessage", function (event) {
    var type = event.detail.type;
    if (type === "cpu" || type === "mem") {
      var frame = splitRecord(event.detail.data);
      // An agent from the previous release sends HTML, not a record. Leave
      // the event alone so htmx still swaps that HTML into the panels.
      if (frame[0] !== (type === "cpu" ? "c1" : "m1")) return;
      if (type === "cpu") readCpuMeta(frame);
      event.preventDefault();
      if (paused) {
        pendingFrames[type] = frame;
      } else {
        if (type === "cpu") applyCpuFrame(frame);
        else applyMemFrame(frame);
        afterValueFrame(type);
      }
      return;
    }
    var held = procHold && type === "proc";
    if (!paused && !held) {
      if (type === "proc") procFocus = procFocusKey(document.activeElement);
      return;
    }
    event.preventDefault();
    rememberSnapshot(event);
  });

  /* ---------- the cpu and mem data frames (ACC-PERF-133-PAGE-COST) ---------- */

  // The server sends the values as one pipe-delimited record; the field order
  // is documented in collector/frame.rs and mirrored here. The page's nodes
  // are written in place, so a tick parses no HTML and replaces no subtree.
  // The records carry an empty field for an unavailable value.

  // The applier remembers the last text and attributes it wrote to each node
  // and compares against them, so the steady-state path reads no DOM value
  // back. A cache belongs to one node generation: a node the page re-creates
  // (a mount, a row, a client-mode member) starts empty, and an out-of-band
  // swap that merges an older agent's HTML into a panel drops every cache at
  // once, because htmx edits the very nodes those references point at.
  var cacheGen = 0;
  var DATA_PANELS = {
    "p-cpu-stat": true, "p-cpu-gauge": true, "p-cores": true,
    "p-mem-stat": true, "p-mem-gauge": true, "p-storage": true
  };

  function nodeCache(node) {
    var cache = node._cache;
    if (!cache || cache.gen !== cacheGen) cache = node._cache = { gen: cacheGen };
    return cache;
  }

  function cached(node, key, selector) {
    var cache = nodeCache(node);
    if (cache[key] === undefined) cache[key] = node.querySelector(selector);
    return cache[key];
  }

  function cachedList(node, key, selector) {
    var cache = nodeCache(node);
    if (cache[key] === undefined) cache[key] = Array.prototype.slice.call(node.querySelectorAll(selector));
    return cache[key];
  }

  // A [data-w] host with a running number effect owns its children. The new
  // reading becomes the effect's target and its settled text; writing
  // textContent here would replace the effect's layers, and numFinish would
  // then put the older reading back. Every applier write reaches this
  // function, so no writer can replace a running effect's layers.
  function frameText(node, text) {
    if (!node) return;
    var state = node._numState;
    if (state && state.el === node && state.run) {
      if (state.text !== text) {
        state.text = text;
        if (state.run.retarget) state.run.retarget(text);
      }
      // The shadow carries the newest reading too, because syncNumbers reads
      // it back for a host the applier wrote.
      nodeCache(node).text = text;
      return;
    }
    var cache = nodeCache(node);
    if (cache.text === text) return;
    cache.text = text;
    node.textContent = text;
  }

  function frameAttr(node, name, value) {
    var cache = nodeCache(node);
    var attrs = cache.attrs || (cache.attrs = {});
    if (attrs[name] === value) return;
    attrs[name] = value;
    if (value === null) node.removeAttribute(name);
    else node.setAttribute(name, value);
  }

  // Split a record on the pipe, skipping an escaped one, and decode each field
  // in the same pass. The server escapes the backslash first, so a backslash
  // always begins a two-character escape; the applier reads the decoded fields
  // by index and decodes nothing again.
  function splitRecord(text) {
    var fields = [];
    var field = "";
    for (var index = 0; index < text.length; index += 1) {
      var ch = text[index];
      if (ch === "\\") {
        index += 1;
        ch = text[index];
        field += ch === "n" ? "\n" : ch;
      } else if (ch === "|") {
        fields.push(field);
        field = "";
      } else {
        field += ch;
      }
    }
    fields.push(field);
    return fields;
  }

  function svgNode(name) {
    return document.createElementNS("http://www.w3.org/2000/svg", name);
  }

  // The CPU event's metadata fields, and the page's interval chip, which the
  // first page renders and every frame re-states.
  function readCpuMeta(frame) {
    if (frame[0] !== "c1") return;
    var sent = Number(frame[1]);
    if (sent > 0) updateMs = sent;
    var windowSent = Number(frame[2]);
    if (windowSent > 0) windowMs = windowSent;
    var slowSent = Number(frame[3]);
    if (slowSent > 0 && slowSent !== slowMs) {
      slowMs = slowSent;
      forgetPasses();
    }
    var seq = Number(frame[4]);
    if (seq !== slowSeq) {
      // A restarted server counts again from 1.
      if (seq < slowSeq) forgetPasses();
      slowSeq = seq;
      // A new slow result carries the charts a whole period further left.
      lastChartAt = window.performance.now();
      notePass(seq);
      // The pause control sits in the CONFIG popover, and opening that
      // closes a popup, so a paused page's popup holds its first body.
      if (!paused) askDetail();
    }
    if (interval) {
      frameAttr(interval, "data-update-ms", String(updateMs));
      frameAttr(interval, "data-window-ms", String(windowMs));
      frameAttr(interval, "data-slow-ms", String(slowMs));
      frameAttr(interval, "data-slow-seq", String(slowSeq));
      frameText(interval, updateMs + " ms");
    }
    var now = Date.now();
    if (lastFrame > 0 && sampleAge) sampleAge.textContent = ((now - lastFrame) / 1000).toFixed(1) + " s";
    lastFrame = now;
    if (stale) {
      stale = false;
      paintLive();
    }
    armStale();
    paintClock();
    syncMotion();
  }

  // CPU_HISTORY and MEM_HISTORY are both 467 by 28 (src/chart.rs). A chart is
  // absent until the first slow pass has a path; the client creates the same
  // svg then. `_seq` is the slow pass the path belongs to, so syncMotion
  // restarts a chart only when its own new path is on the page (DEF-GRAPH-128).
  function setChart(box, line, area, seq) {
    if (!box) return;
    var svg = cached(box, "svg", "svg.chart");
    if (!line) {
      if (svg) {
        svg.remove();
        nodeCache(box).svg = null;
      }
      return;
    }
    if (!svg) {
      box.innerHTML = '<svg class="chart" viewBox="0 0 467 28" preserveAspectRatio="none" aria-hidden="true"><g class="series"><path class="area"></path><path class="line"></path></g></svg>';
      svg = nodeCache(box).svg = box.querySelector("svg.chart");
    }
    var areaPath = cached(svg, "area", "path.area");
    var linePath = cached(svg, "line", "path.line");
    frameAttr(areaPath, "d", area);
    frameAttr(linePath, "d", line);
    svg._seq = seq;
  }

  function setStatTile(tile, value, band) {
    if (!tile) return;
    var span = cached(tile, "value", ".stat-value");
    if (!value) {
      if (span) {
        tile.innerHTML = '<span class="stat-na">unavailable</span>';
        nodeCache(tile).value = null;
      }
      return;
    }
    if (!span) {
      tile.innerHTML = '<span class="stat-value" data-w="100" data-range="100" id="cpu-total"></span><span class="stat-unit">%</span>';
      span = nodeCache(tile).value = tile.querySelector(".stat-value");
    }
    frameText(span, value);
    frameAttr(span, "class", "stat-value " + band);
  }

  function setGaugeAria(gauge, label, unavailable) {
    if (label) {
      frameAttr(gauge, "aria-label", label);
      frameAttr(gauge, "data-state", null);
    } else {
      frameAttr(gauge, "aria-label", unavailable);
      frameAttr(gauge, "data-state", "unavailable");
    }
  }

  // One gauge shape's arc. The server sends the path, so the client draws no
  // geometry; an empty path means the gauge is unavailable and draws no fill.
  function setGaugePath(svg, d, band) {
    var path = cached(svg, "fill", "path.g-fill");
    if (!d) {
      if (path) {
        path.remove();
        nodeCache(svg).fill = null;
      }
      return;
    }
    if (!path) {
      path = svgNode("path");
      svg.insertBefore(path, cached(svg, "tick", "path.g-tick"));
      nodeCache(svg).fill = path;
    }
    frameAttr(path, "class", "g-fill " + band);
    frameAttr(path, "d", d);
  }

  function setWideFill(gauge, band, to) {
    var wide = cached(gauge, "wide", "svg.g-wide");
    var use = cached(wide, "use", "use.g-fill");
    if (!to) {
      if (use) {
        use.remove();
        nodeCache(wide).use = null;
      }
      return;
    }
    if (!use) {
      use = svgNode("use");
      wide.appendChild(use);
      nodeCache(wide).use = use;
    }
    frameAttr(use, "class", "g-fill " + band);
    frameAttr(use, "href", "#g-wide-arc");
    frameAttr(use, "style", "--from:0;--to:" + to);
  }

  // The two shapes' big value text, or the unavailable label the page draws
  // in its place. The unit tspan keeps the value's own text node separate.
  function setGaugeValue(svg, value, unit, label) {
    var text = cached(svg, "value", "text.g-value");
    var name = cached(svg, "label", "text.g-label");
    if (!value) {
      if (text) {
        text.remove();
        nodeCache(svg).value = null;
      }
      if (name) {
        frameText(name, "unavailable");
        frameAttr(name, "y", "70");
      }
      return;
    }
    if (!text) {
      text = svgNode("text");
      text.setAttribute("class", "g-value");
      text.setAttribute("x", "54");
      text.setAttribute("y", "66");
      text.appendChild(document.createTextNode(""));
      var created = svgNode("tspan");
      created.setAttribute("class", "g-unit");
      created.setAttribute("dx", "2.5");
      text.appendChild(created);
      if (name) svg.insertBefore(text, name);
      else svg.appendChild(text);
      nodeCache(svg).value = text;
    }
    var cache = nodeCache(text);
    if (cache.value !== value) {
      cache.value = value;
      text.firstChild.nodeValue = value;
    }
    frameText(cached(text, "unit", "tspan.g-unit"), unit);
    if (name) {
      frameText(name, label);
      frameAttr(name, "y", "78");
    }
  }

  // The big value inside a gauge's readout: the template's
  // <b>value<i>unit</i></b><span>label</span>, or the unavailable line. The
  // shape changes with the value's presence, so the mode decides a rebuild.
  function setGaugeRead(read, value, unit, label) {
    if (!read) return;
    var cache = nodeCache(read);
    var mode = value ? "value" : "unavailable";
    if (cache.mode !== mode) {
      cache.mode = mode;
      read.innerHTML = value ? "<b></b><span></span>" : "<span></span>";
      cache.label = read.querySelector("span");
      if (value) {
        var bold = read.querySelector("b");
        cache.value = bold.appendChild(document.createTextNode(""));
        cache.unit = bold.appendChild(document.createElement("i"));
      } else {
        cache.value = null;
        cache.unit = null;
      }
    }
    if (value) {
      if (cache.value.nodeValue !== value) cache.value.nodeValue = value;
      frameText(cache.unit, unit);
      frameText(cache.label, label);
    } else {
      frameText(cache.label, "unavailable");
    }
  }

  /* ---------- the gauge readout's number effect ---------- */

  // ACC-VIEW-231-ALL-NUM: the gauge's centre readout runs the configured
  // effect like a text host. SVG text cannot hold the effect's span layers,
  // so a [data-w] host in a .g-num overlay carries them; the painted copy
  // stays laid out under the overlay, hidden by data-num-gauge, and the
  // gauge's aria-label carries the settled reading, because the overlay and
  // the readout it covers are hidden from a screen reader. Colours come from
  // the tokens, so a theme switch needs no re-placement; the placement copies
  // the paint's size, spacing and baseline.

  function zeroSpan() {
    var probe = document.createElement("span");
    probe.style.cssText = "display:inline-block;width:0;height:0;margin:0;padding:0;border:0";
    return probe;
  }

  function numPx(value) {
    var at = parseFloat(value);
    return at === at ? at : 0;
  }

  // The copy the overlay stands on: the wide layout's HTML readout, else the
  // one shape SVG the container query shows. A hidden panel or an unavailable
  // value has none.
  function gaugePaint(gauge) {
    var read = gauge.querySelector(".g-read b");
    if (read && read.getClientRects().length) {
      return { box: read, parent: read.parentNode, html: true, unit: read.querySelector("i") };
    }
    var values = gauge.querySelectorAll(".g-value");
    for (var index = 0; index < values.length; index += 1) {
      if (!values[index].getClientRects().length) continue;
      return {
        box: values[index],
        parent: gauge,
        html: false,
        unit: values[index].querySelector(".g-unit")
      };
    }
    return null;
  }

  function gaugeBox(gauge) {
    var box = document.createElement("div");
    box.className = "g-num";
    box.setAttribute("aria-hidden", "true");
    var host = box.appendChild(document.createElement("span"));
    host.setAttribute("data-w", "");
    box.appendChild(document.createElement("i"));
    box.appendChild(zeroSpan());
    nodeCache(gauge).num = box;
    return box;
  }

  // The overlay copies the paint's type at its rendered size (an SVG user
  // unit times the shape's screen scale), and its top is placed from a
  // zero-size calibration inline-block, whose bottom margin edge sits on the
  // line's baseline: the paint's baseline is its screen CTM's y, or a zero
  // span in the HTML box. Both are viewport deltas, so no containing block is
  // read, and the HTML copy lives inside .g-read, whose translate would
  // otherwise rasterize the two copies at different subpixel phases.
  function gaugePlace(gauge, paint) {
    var box = cached(gauge, "num", ".g-num");
    var unit = box.children[1];
    var style = window.getComputedStyle(paint.box);
    var scale = 1;
    var size = numPx(style.fontSize);
    var ctm = null;
    if (!paint.html) {
      ctm = paint.box.getScreenCTM();
      if (!ctm) return;
      scale = ctm.a;
      size *= scale;
    }
    box.style.fontSize = size + "px";
    box.style.letterSpacing = numPx(style.letterSpacing) * scale + "px";
    var unitStyle = window.getComputedStyle(paint.unit);
    unit.style.fontSize = numPx(unitStyle.fontSize) * scale + "px";
    unit.style.letterSpacing = numPx(unitStyle.letterSpacing) * scale + "px";
    unit.style.marginLeft = (paint.html
      ? numPx(unitStyle.marginLeft)
      : numPx(paint.unit.getAttribute("dx")) * scale) + "px";
    var top = gauge.getBoundingClientRect().top;
    var target;
    if (paint.html) {
      var probe = zeroSpan();
      paint.box.appendChild(probe);
      target = probe.getBoundingClientRect().top - top;
      probe.remove();
    } else {
      target = ctm.d * Number(paint.box.getAttribute("y")) + ctm.f - top;
    }
    box.style.top = "0px";
    var cal = box.lastChild.getBoundingClientRect().top - top;
    box.style.top = (target - cal).toFixed(3) + "px";
  }

  // The host takes the settled reading through frameText, so a running effect
  // retargets instead of being painted over.
  function gaugeText(box, state) {
    frameText(box.firstChild, state.value);
    frameAttr(box.firstChild, "data-range", state.range || null);
    frameText(box.children[1], state.unit);
  }

  // Detection and placement read layout, so the per-frame path below runs
  // them only when the overlay is missing or stopped; the fit pass runs them
  // after a resize, a shape switch or a face change. A hidden panel or an
  // unavailable value keeps the host's text current but shows no overlay.
  function gaugeShow(gauge) {
    var state = gauge._num;
    if (!state) return;
    state.paint = gaugePaint(gauge);
    var box = cached(gauge, "num", ".g-num");
    if (!box && state.value && state.paint) {
      box = gaugeBox(gauge);
      state.paint.parent.appendChild(box);
    }
    if (box) gaugeText(box, state);
    if (!state.value || !state.paint) {
      if (box) box.classList.remove("on");
      frameAttr(gauge, "data-num-gauge", null);
      return;
    }
    if (box.parentNode !== state.paint.parent) state.paint.parent.appendChild(box);
    box.classList.add("on");
    frameAttr(gauge, "data-num-gauge", "");
    gaugePlace(gauge, state.paint);
  }

  function gaugeNumber(gauge, value, unit, range) {
    var state = gauge._num || (gauge._num = { paint: null });
    state.value = value || "";
    state.unit = unit || "";
    state.range = range || "";
    var box = cached(gauge, "num", ".g-num");
    if (state.value && box && box.classList.contains("on")) {
      gaugeText(box, state);
      return;
    }
    gaugeShow(gauge);
  }

  function setMeter(node, value) {
    if (!node) return;
    if (value) {
      frameAttr(node, "style", "--value:" + value);
      frameAttr(node, "data-state", null);
    } else {
      frameAttr(node, "style", null);
      frameAttr(node, "data-state", "unavailable");
    }
  }

  function coreLabel(name) {
    return "C" + String(name.replace(/^cpu/, "")).padStart(2, "0");
  }

  function setCoreRow(row, name, value, display) {
    frameAttr(row, "data-tip", tipText("core") + " " + name.replace(/^cpu/, ""));
    frameText(cached(row, "label", ".cl"), coreLabel(name));
    var meter = cached(row, "meter", ".meter");
    var cv = cached(row, "cv", ".cv");
    var cell = cached(cv, "cell", "span[data-w]");
    var unit = cached(cv, "unit", "i");
    if (!value) {
      frameAttr(row, "style", null);
      if (meter) frameAttr(meter, "data-state", "unavailable");
      if (unit) {
        unit.remove();
        nodeCache(cv).unit = null;
      }
      frameText(cell, "");
      return;
    }
    frameAttr(row, "style", "--value:" + value);
    if (meter) frameAttr(meter, "data-state", null);
    if (!unit) {
      unit = document.createElement("i");
      unit.textContent = "%";
      cv.appendChild(unit);
      nodeCache(cv).unit = unit;
    }
    frameText(cell, display);
  }

  function newCoreRow(name, value, display) {
    var row = document.createElement("div");
    row.className = "core-row";
    row.innerHTML = value
      ? '<span class="cl"></span><span class="meter"></span><span class="cv"><span data-w="100"></span><i>%</i></span>'
      : '<span class="cl"></span><span class="meter"></span><span class="cv"><span data-w="100"></span></span>';
    setCoreRow(row, name, value, display);
    return row;
  }

  // The CPU stat header's optional facts. The first page renders one only
  // when its value is known, so a frame creates the span on the first value
  // and removes it when the field is empty; the order is the template's.
  function setCpuStatFacts(stat, frame) {
    var box = cached(stat, "facts", ".stat-facts");
    if (!box) return;
    var frequency = frame[19];
    var node = cached(box, "freq", '[data-field="cpu-freq"]');
    if (frequency) {
      if (!node) {
        node = document.createElement("span");
        node.setAttribute("data-field", "cpu-freq");
        node.setAttribute("data-tip", tipText("cpu-freq"));
        var freq = document.createElement("b");
        freq.setAttribute("data-w", "00.00 GHz");
        node.appendChild(freq);
        box.insertBefore(node, box.querySelector('[data-field="cpu-temp"], [data-field="cpu-load"]'));
        nodeCache(box).freq = node;
      }
      frameText(cached(node, "b", "b"), frequency);
    } else if (node) {
      node.remove();
      nodeCache(box).freq = null;
    }
    var temperature = frame[20];
    node = cached(box, "temp", '[data-field="cpu-temp"]');
    if (temperature) {
      if (!node) {
        node = document.createElement("span");
        node.className = "cpu-temp";
        node.setAttribute("data-field", "cpu-temp");
        node.setAttribute("data-tip", tipText("cpu-temp"));
        var value = document.createElement("b");
        value.setAttribute("data-w", "100\u00b0C");
        node.appendChild(value);
        box.insertBefore(node, box.querySelector('[data-field="cpu-load"]'));
        nodeCache(box).temp = node;
      }
      frameText(cached(node, "b", "b"), temperature);
    } else if (node) {
      node.remove();
      nodeCache(box).temp = null;
    }
    var tip = frame[12];
    node = cached(box, "load", '[data-field="cpu-load"]');
    if (tip || frame[26]) {
      if (!node) {
        node = document.createElement("span");
        node.setAttribute("data-field", "cpu-load");
        box.appendChild(node);
        nodeCache(box).load = node;
      }
      if (tip) {
        if (!cached(node, "b", "b")) {
          node.innerHTML = 'load <b></b>/1m <b></b>/5m <b></b>/15m';
          // The new children void the node's text shadow and the figures list.
          nodeCache(node).text = undefined;
          nodeCache(node).b = node.querySelector("b");
          nodeCache(stat).figures = undefined;
        }
        frameAttr(node, "data-tip", tip);
      } else {
        frameText(node, "load unavailable");
        frameAttr(node, "data-tip", tipText("load-container"));
        if (nodeCache(node).b) {
          nodeCache(node).b = null;
          nodeCache(stat).figures = undefined;
        }
      }
    } else if (node) {
      node.remove();
      nodeCache(box).load = null;
      nodeCache(stat).figures = undefined;
    }
  }

  // The CPU gauge header's name and core count: the first page renders each
  // only when the snapshot carries it.
  function setCpuGaugeHeader(header, frame) {
    var name = frame[22];
    var node = cached(header, "name", '[data-field="cpu-name"]');
    if (name) {
      if (!node) {
        node = document.createElement("div");
        node.className = "cpu-name";
        node.setAttribute("data-p", "3");
        node.setAttribute("data-field", "cpu-name");
        header.insertBefore(node, header.querySelector('[data-field="cpu-gauge-cores"]'));
        nodeCache(header).name = node;
      }
      frameText(node, name);
      frameAttr(node, "title", name);
    } else if (node) {
      node.remove();
      nodeCache(header).name = null;
    }
    var cores = frame[21];
    node = cached(header, "cores", '[data-field="cpu-gauge-cores"]');
    if (cores) {
      if (!node) {
        node = document.createElement("span");
        node.className = "head-right meta";
        node.setAttribute("data-p", "3");
        node.setAttribute("data-field", "cpu-gauge-cores");
        node.setAttribute("data-tip", tipText("cpu-cores"));
        header.appendChild(node);
        nodeCache(header).cores = node;
      }
      frameText(node, cores + " cores");
    } else if (node) {
      node.remove();
      nodeCache(header).cores = null;
    }
  }

  // One gauge fact's value: the template's <b> with its reserved width, or
  // the unavailable span when the field is empty.
  function setFactValue(fact, value, reserve) {
    if (!fact) return;
    var b = cached(fact, "b", "b");
    if (!value) {
      if (!b) return;
      b.remove();
      nodeCache(fact).b = null;
      var span = document.createElement("span");
      span.textContent = "unavailable";
      fact.appendChild(span);
      return;
    }
    if (!b) {
      var label = cached(fact, "label", "span");
      while (label.nextSibling) label.nextSibling.remove();
      b = document.createElement("b");
      b.setAttribute("data-w", reserve);
      fact.appendChild(b);
      nodeCache(fact).b = b;
    }
    frameText(b, value);
  }

  // The load fact: three figures with their windows, or the template's
  // unavailable line.
  function setLoadFact(fact, loads, reserve, tip) {
    if (!fact) return;
    var b = cached(fact, "b", "b");
    if (!tip) {
      if (!b) return;
      b.remove();
      nodeCache(fact).b = null;
      frameAttr(fact, "class", "fact");
      var span = document.createElement("span");
      span.textContent = "unavailable";
      fact.appendChild(span);
      frameAttr(fact, "data-tip", tipText("load"));
      return;
    }
    if (!b) {
      frameAttr(fact, "class", "fact load");
      fact.innerHTML = '<span>load</span><b><span class="load-cell"><span></span>/1m</span> <span class="load-cell"><span></span>/5m</span> <span class="load-cell"><span></span>/15m</span></b>';
      b = nodeCache(fact).b = fact.querySelector("b");
    }
    frameAttr(fact, "data-tip", tip);
    var cells = cachedList(b, "cells", ".load-cell > span");
    for (var index = 0; index < cells.length; index += 1) {
      if (reserve) frameAttr(cells[index], "data-w", reserve);
      frameText(cells[index], loads[index]);
    }
  }

  function applyCpuFrame(frame) {
    if (frame[0] !== "c1") return;
    var total = frame[5];
    var band = frame[6];
    var bandGauge = frame[7];
    var loads = [frame[13], frame[14], frame[15]];
    var reserve = frame[11];
    var loadTip = frame[12];
    var stat = document.getElementById("p-cpu-stat");
    var gaugePanel = document.getElementById("p-cpu-gauge");
    if (stat) {
      setCpuStatFacts(stat, frame);
      setStatTile(cached(stat, "tile", ".stat-tile"), total, band);
      var figures = cachedList(stat, "figures", '[data-field="cpu-load"] b');
      for (var index = 0; index < figures.length; index += 1) {
        frameText(figures[index], loads[index]);
        if (reserve) frameAttr(figures[index], "data-w", reserve);
      }
      setChart(cached(stat, "chart", '[data-chart="cpu"]'), frame[frame.length - 2], frame[frame.length - 1], Number(frame[4]));
    }
    if (gaugePanel) {
      var header = cached(gaugePanel, "header", "header.ph");
      if (header) setCpuGaugeHeader(header, frame);
      var gauge = cached(gaugePanel, "gauge", '[data-gauge="cpu"]');
      if (gauge) {
        var circle = cached(gauge, "circle", "svg.g-circle");
        var squircle = cached(gauge, "squircle", "svg.g-squircle");
        setGaugeAria(gauge, total ? "CPU total " + total + " percent" : "", "CPU unavailable");
        setGaugePath(circle, frame[8], bandGauge);
        setGaugePath(squircle, frame[9], bandGauge);
        setWideFill(gauge, bandGauge, frame[10]);
        setGaugeValue(circle, total, "%", "total");
        setGaugeValue(squircle, total, "%", "total");
        setGaugeRead(cached(gauge, "read", ".g-read"), total, "%", "total");
        gaugeNumber(gauge, total, "%", "100");
      }
      var facts = cachedList(gaugePanel, "facts", ".facts .fact");
      if (facts.length) {
        setLoadFact(facts[0], loads, reserve, loadTip);
        var values = [frame[16], frame[17], frame[18]];
        var reserves = ["100%", "100%", "100.00%"];
        for (index = 0; index < values.length; index += 1) {
          setFactValue(facts[index + 1], values[index], reserves[index]);
        }
      }
    }
    var cores = document.getElementById("p-cores");
    if (cores) {
      var coresHeader = cached(cores, "header", "header.ph");
      var body = cached(cores, "body", ".cores-body");
      var count = coresHeader ? cached(coresHeader, "count", ".head-right.meta[data-tip]") : null;
      var withheld = coresHeader ? cached(coresHeader, "withheld", "[data-withheld]") : null;
      if (frame[24]) {
        // A container without a cpuset names no cores: the template draws the
        // header's unavailable state and no rows.
        if (body) {
          body.remove();
          nodeCache(cores).body = null;
        }
        if (count) {
          count.remove();
          nodeCache(coresHeader).count = null;
        }
        if (coresHeader && !withheld) {
          withheld = document.createElement("span");
          withheld.className = "head-right meta";
          withheld.setAttribute("data-withheld", "");
          withheld.setAttribute("title", tipText("cores-withheld"));
          withheld.textContent = "unavailable";
          coresHeader.appendChild(withheld);
          nodeCache(coresHeader).withheld = withheld;
        }
      } else {
        if (withheld) {
          withheld.remove();
          nodeCache(coresHeader).withheld = null;
        }
        if (!body) {
          body = document.createElement("div");
          body.className = "cores-body";
          cores.appendChild(body);
          nodeCache(cores).body = body;
        }
        var rows = Number(frame[25]);
        frameAttr(body, "style", "--rows:" + frame[23]);
        var nodes = cachedList(body, "rows", ".core-row");
        for (index = 0; index < rows; index += 1) {
          var node = nodes[index];
          var name = frame[27 + index * 3];
          var value = frame[28 + index * 3];
          var display = frame[29 + index * 3];
          if (!node) {
            node = newCoreRow(name, value, display);
            body.appendChild(node);
            nodeCache(body).rows = undefined;
          } else {
            setCoreRow(node, name, value, display);
          }
        }
        nodes = cachedList(body, "rows", ".core-row");
        if (rows < nodes.length) {
          for (index = rows; index < nodes.length; index += 1) nodes[index].remove();
          nodeCache(body).rows = undefined;
        }
        if (rows > 0) {
          if (!count && coresHeader) {
            count = document.createElement("span");
            count.className = "head-right meta";
            count.setAttribute("data-tip", tipText("cores-count"));
            coresHeader.appendChild(count);
            nodeCache(coresHeader).count = count;
          }
          frameText(count, frame[25] + " \u00b7 0\u2013100%");
        } else if (count) {
          count.remove();
          nodeCache(coresHeader).count = null;
        }
      }
    }
  }

  // The memory gauge header's total, rendered only when the snapshot has one.
  function setMemGaugeTotal(gaugePanel, frame) {
    var header = cached(gaugePanel, "header", "header.ph");
    if (!header) return;
    var total = frame[9];
    var node = cached(header, "total", '[data-field="mem-gauge-total"]');
    if (!total) {
      if (node) {
        node.remove();
        nodeCache(header).total = null;
      }
      return;
    }
    if (!node) {
      node = document.createElement("span");
      node.className = "head-right meta";
      node.setAttribute("data-p", "3");
      node.setAttribute("data-field", "mem-gauge-total");
      var inner = document.createElement("span");
      inner.appendChild(document.createTextNode("of total "));
      var value = document.createElement("span");
      value.className = "meta-value";
      value.setAttribute("data-w", "000.0 MiB");
      inner.appendChild(value);
      node.appendChild(inner);
      header.appendChild(node);
      nodeCache(header).total = node;
    }
    var body = cached(node, "body", "span");
    var meta = cached(body, "meta", ".meta-value");
    frameText(meta, total);
    var suffix = nodeCache(body).suffix;
    if (suffix === undefined) {
      var last = body.lastChild;
      suffix = last && last !== meta && last.nodeType === 3 ? last : null;
      nodeCache(body).suffix = suffix;
    }
    if (frame[10]) {
      if (!suffix) {
        suffix = document.createTextNode("");
        body.appendChild(suffix);
        nodeCache(body).suffix = suffix;
      }
      if (suffix.nodeValue !== frame[10]) suffix.nodeValue = frame[10];
    } else if (suffix && suffix.nodeValue !== "") {
      suffix.nodeValue = "";
    }
    frameAttr(node, "data-tip", tipText("total", frame[frame.length - 3]));
  }

  // One Memory stat header fact: the template's <b> with its 100% width, or
  // no span at all when the value is unavailable.
  function setMemStatFact(facts, field, value, tip, label) {
    var key = "fact-" + field;
    var node = cached(facts, key, '[data-field="' + field + '"]');
    if (!value) {
      if (node) {
        node.remove();
        nodeCache(facts)[key] = null;
      }
      return;
    }
    if (!node) {
      node = document.createElement("span");
      node.setAttribute("data-field", field);
      var b = document.createElement("b");
      b.setAttribute("data-w", "100%");
      node.appendChild(b);
      node.appendChild(document.createTextNode(" " + label));
      facts.insertBefore(node, field === "mem-used-fact" ? facts.firstChild : null);
      nodeCache(facts)[key] = node;
    }
    frameText(cached(node, "b", "b"), value);
    frameAttr(node, "data-tip", tip);
  }

  function setMemStatHeader(stat, frame) {
    var header = cached(stat, "header", "header.ph .head-right");
    if (!header) return;
    var node = cached(header, "total", '[data-field="mem-stat-total"]');
    if (frame[9]) {
      if (!node) {
        node = document.createElement("span");
        node.className = "meta";
        node.setAttribute("data-field", "mem-stat-total");
        node.setAttribute("data-w", "of 000.0 MiB");
        header.insertBefore(node, cached(header, "facts", '[data-field="mem-stat-facts"]'));
        nodeCache(header).total = node;
      }
      frameText(node, "of " + frame[9] + frame[10]);
      frameAttr(node, "data-tip", tipText("total", frame[frame.length - 3]));
    } else if (node) {
      node.remove();
      nodeCache(header).total = null;
    }
    var facts = cached(header, "facts", '[data-field="mem-stat-facts"]');
    if (!facts) return;
    setMemStatFact(facts, "mem-used-fact", frame[5], tipText("used", frame[frame.length - 3]), "used");
    setMemStatFact(facts, "mem-avail-fact", frame[6], tipText("avail"), "available");
  }

  var MEM_ROW_ORDER = ["used", "cache", "other", "other-cache", "free", "free-cache", "swap"];

  function newMemRow(kind, size, pct, label, tip) {
    var row = document.createElement("div");
    row.className = kind === "swap" ? "mem-row swap" : "mem-row";
    row.setAttribute("data-row", kind);
    var head = document.createElement("span");
    if (kind !== "free-cache") {
      var key = document.createElement("i");
      key.className = "mem-key";
      key.setAttribute("data-seg", kind);
      head.appendChild(key);
    }
    head.appendChild(document.createTextNode(label));
    var value = document.createElement("b");
    value.setAttribute("data-w", kind === "swap" ? "000.0 / 000.0 GiB" : "000.0 MiB");
    var share = document.createElement("span");
    share.className = "pct";
    share.setAttribute("data-w", "100%");
    row.appendChild(head);
    row.appendChild(value);
    row.appendChild(share);
    setMemRow(row, size, pct, label, tip);
    return row;
  }

  function setMemRow(row, size, pct, label, tip) {
    frameAttr(row, "data-tip", tip);
    var head = cached(row, "head", "span");
    var text = nodeCache(head).label;
    if (text === undefined) {
      var last = head.lastChild;
      text = last && last.nodeType === 3 ? last : null;
      nodeCache(head).label = text;
    }
    if (text && text.nodeValue !== label) text.nodeValue = label;
    frameText(cached(row, "size", "b"), size);
    frameText(cached(row, "pct", ".pct"), pct);
  }

  // Where a created row belongs: before the first row the frame lists after
  // it, or last. Only a changed set reaches this, so the scan costs nothing
  // on the steady-state path.
  function memRowAnchor(panel, kind) {
    var at = MEM_ROW_ORDER.indexOf(kind);
    var rows = panel.querySelectorAll(".mem-row[data-row]");
    for (var index = 0; index < rows.length; index += 1) {
      if (MEM_ROW_ORDER.indexOf(rows[index].getAttribute("data-row")) > at) return rows[index];
    }
    return null;
  }

  function newMount(label) {
    var node = document.createElement("div");
    node.className = "mount";
    var head = document.createElement("div");
    head.className = "mount-head";
    var name = document.createElement("span");
    name.className = "mount-label";
    name.textContent = label;
    head.appendChild(name);
    node.appendChild(head);
    var used = document.createElement("div");
    used.className = "mount-row";
    used.innerHTML = '<span class="mount-key">used</span><span class="meter meter-used"></span><span class="mount-pct" data-w="100%"></span><span class="mount-size" data-w="000.0 Byte"></span>';
    var free = document.createElement("div");
    free.className = "mount-row free";
    free.innerHTML = '<span class="mount-key">free</span><span class="meter meter-free"></span><span class="mount-pct" data-w="100%"></span><span class="mount-size" data-w="000.0 Byte"></span>';
    node.appendChild(used);
    node.appendChild(free);
    return node;
  }

  // One mount block; the IO share and the total are rendered only when the
  // record carries them, in the template's order.
  function setMount(node, frame, at, tip) {
    frameAttr(node, "data-tip", tip);
    var label = frame[at];
    var name = cached(node, "name", ".mount-label");
    frameText(name, label);
    frameAttr(name, "data-tip", label);
    var io = frame[at + 1];
    var ioNode = cached(node, "io", ".mount-io");
    if (io) {
      if (!ioNode) {
        var head = cached(node, "head", ".mount-head");
        ioNode = document.createElement("span");
        ioNode.className = "mount-io";
        ioNode.setAttribute("data-tip", tipText("mount-io"));
        ioNode.appendChild(document.createTextNode("IO "));
        var ioValue = document.createElement("span");
        ioValue.setAttribute("data-w", "100%");
        ioNode.appendChild(ioValue);
        head.insertBefore(ioNode, cached(head, "b", "b"));
        nodeCache(node).io = ioNode;
      }
      frameText(cached(ioNode, "value", "span"), io);
    } else if (ioNode) {
      ioNode.remove();
      nodeCache(node).io = null;
    }
    var total = frame[at + 2];
    var totalNode = cached(node, "total", ".mount-head b");
    if (total) {
      if (!totalNode) {
        var mountHead = cached(node, "head", ".mount-head");
        totalNode = document.createElement("b");
        totalNode.setAttribute("data-w", "000.0 Byte");
        mountHead.appendChild(totalNode);
        nodeCache(node).total = totalNode;
        nodeCache(mountHead).b = totalNode;
      }
      frameText(totalNode, total);
    } else if (totalNode) {
      totalNode.remove();
      nodeCache(node).total = null;
      nodeCache(cached(node, "head", ".mount-head")).b = null;
    }
    var rows = cachedList(node, "rows", ".mount-row");
    if (rows[0]) {
      setMeter(cached(rows[0], "meter", ".meter-used"), frame[at + 3]);
      frameText(cached(rows[0], "pct", ".mount-pct"), frame[at + 4]);
      frameText(cached(rows[0], "size", ".mount-size"), frame[at + 5]);
    }
    if (rows[1]) {
      setMeter(cached(rows[1], "meter", ".meter-free"), frame[at + 6]);
      frameText(cached(rows[1], "pct", ".mount-pct"), frame[at + 7]);
      frameText(cached(rows[1], "size", ".mount-size"), frame[at + 8]);
    }
  }

  function applyMemFrame(frame) {
    if (frame[0] !== "m1") return;
    var seq = Number(frame[1]);
    // Parse every field first: the panels below are independent, and the
    // storage records follow the gauge's variable-length lists.
    var at = 15;
    var spans = Number(frame[at]);
    at += 1;
    var names = [];
    var circle = [];
    var squircle = [];
    var froms = [];
    var tos = [];
    for (var index = 0; index < spans; index += 1) {
      names.push(frame[at]);
      circle.push(frame[at + 1]);
      squircle.push(frame[at + 2]);
      froms.push(frame[at + 3]);
      tos.push(frame[at + 4]);
      at += 5;
    }
    var segments = Number(frame[at]);
    at += 1;
    var barNames = [];
    var barWidths = [];
    for (index = 0; index < segments; index += 1) {
      barNames.push(frame[at]);
      barWidths.push(frame[at + 1]);
      at += 2;
    }
    var rows = Number(frame[at]);
    at += 1;
    var rowData = [];
    for (index = 0; index < rows; index += 1) {
      rowData.push([frame[at], frame[at + 1], frame[at + 2], frame[at + 3], tipText("row-" + frame[at], frame[frame.length - 3])]);
      at += 5;
    }
    var mountCount = Number(frame[at]);
    at += 1;
    var stat = document.getElementById("p-mem-stat");
    var gaugePanel = document.getElementById("p-mem-gauge");
    var storage = document.getElementById("p-storage");
    if (stat) {
      setMemStatHeader(stat, frame);
      var tile = cached(stat, "tile", ".stat-tile");
      var span = tile ? cached(tile, "value", ".stat-value") : null;
      if (tile && frame[2]) {
        if (!span) {
          tile.innerHTML = '<span class="stat-value" data-w="000.0" id="mem-used"></span><span class="stat-unit"></span>';
          span = nodeCache(tile).value = tile.querySelector(".stat-value");
          nodeCache(tile).unit = tile.querySelector(".stat-unit");
        }
        frameText(span, frame[2]);
        frameAttr(span, "data-range", frame[4]);
        frameText(cached(tile, "unit", ".stat-unit"), frame[3]);
      } else if (tile && span) {
        tile.innerHTML = '<span class="stat-na">unavailable</span>';
        nodeCache(tile).value = null;
        nodeCache(tile).unit = null;
      }
      setChart(cached(stat, "chart", '[data-chart="mem"]'), frame[frame.length - 2], frame[frame.length - 1], seq);
    }
    if (gaugePanel) {
      setMemGaugeTotal(gaugePanel, frame);
      var gauge = cached(gaugePanel, "gauge", '[data-gauge="mem"]');
      if (gauge) {
        var aria = frame[7] ? "Memory " + frame[7] + " used, " + frame[5].replace("%", "") + " percent" : "";
        setGaugeAria(gauge, aria, "Memory unavailable");
        var shapes = [cached(gauge, "circle", "svg.g-circle"), cached(gauge, "squircle", "svg.g-squircle")];
        for (var shape = 0; shape < shapes.length; shape += 1) {
          var paths = cachedList(shapes[shape], "fills", "path.g-fill");
          var madePath = false;
          for (index = 0; index < spans; index += 1) {
            var path = paths[index];
            if (!path) {
              path = svgNode("path");
              shapes[shape].insertBefore(path, cached(shapes[shape], "tick", "path.g-tick"));
              madePath = true;
            }
            frameAttr(path, "class", "g-fill");
            frameAttr(path, "data-seg", names[index]);
            frameAttr(path, "d", shape === 0 ? circle[index] : squircle[index]);
          }
          if (madePath || spans < paths.length) {
            for (index = spans; index < paths.length; index += 1) paths[index].remove();
            nodeCache(shapes[shape]).fills = undefined;
          }
        }
        var wide = cached(gauge, "wide", "svg.g-wide");
        var uses = cachedList(wide, "uses", "use.g-fill");
        var madeUse = false;
        for (index = 0; index < spans; index += 1) {
          var use = uses[index];
          if (!use) {
            use = svgNode("use");
            wide.appendChild(use);
            madeUse = true;
          }
          frameAttr(use, "class", "g-fill");
          frameAttr(use, "data-seg", names[index]);
          frameAttr(use, "href", "#g-wide-arc");
          frameAttr(use, "style", "--from:" + froms[index] + ";--to:" + tos[index]);
        }
        if (madeUse || spans < uses.length) {
          for (index = spans; index < uses.length; index += 1) uses[index].remove();
          nodeCache(wide).uses = undefined;
        }
        setGaugeValue(cached(gauge, "circle", "svg.g-circle"), frame[2], frame[3], "used");
        setGaugeValue(cached(gauge, "squircle", "svg.g-squircle"), frame[2], frame[3], "used");
        setGaugeRead(cached(gauge, "read", ".g-read"), frame[2], frame[3], "used");
        gaugeNumber(gauge, frame[2], frame[3], frame[4]);
        var bar = cached(gaugePanel, "bar", ".mem-bar");
        frameAttr(bar, "data-state", segments === 0 ? "unavailable" : null);
        var bars = cachedList(bar, "segs", ".mem-seg");
        var madeSeg = false;
        for (index = 0; index < segments; index += 1) {
          var segment = bars[index];
          if (!segment) {
            segment = document.createElement("span");
            bar.appendChild(segment);
            madeSeg = true;
          }
          frameAttr(segment, "class", "mem-seg");
          frameAttr(segment, "data-seg", barNames[index]);
          frameAttr(segment, "style", "width:" + barWidths[index] + "%");
        }
        if (madeSeg || segments < bars.length) {
          for (index = segments; index < bars.length; index += 1) bars[index].remove();
          nodeCache(bar).segs = undefined;
        }
        var rowsPanel = cached(gaugePanel, "rows", '[data-field="mem-gauge-facts"]');
        if (rowsPanel) {
          var rowMap = nodeCache(rowsPanel).rowMap;
          if (rowMap === undefined) {
            rowMap = {};
            var listed = rowsPanel.querySelectorAll(".mem-row[data-row]");
            for (index = 0; index < listed.length; index += 1) rowMap[listed[index].getAttribute("data-row")] = listed[index];
            nodeCache(rowsPanel).rowMap = rowMap;
          }
          var sig = "";
          for (index = 0; index < rowData.length; index += 1) sig += rowData[index][0] + "|";
          if (nodeCache(rowsPanel).sig !== sig) {
            nodeCache(rowsPanel).sig = sig;
            var seen = {};
            for (index = 0; index < rowData.length; index += 1) {
              var kind = rowData[index][0];
              seen[kind] = true;
              var row = rowMap[kind];
              if (!row) {
                row = newMemRow(kind, rowData[index][1], rowData[index][2], rowData[index][3], rowData[index][4]);
                rowsPanel.insertBefore(row, memRowAnchor(rowsPanel, kind));
                rowMap[kind] = row;
              } else {
                setMemRow(row, rowData[index][1], rowData[index][2], rowData[index][3], rowData[index][4]);
              }
            }
            for (var gone in rowMap) {
              if (!seen[gone]) {
                rowMap[gone].remove();
                delete rowMap[gone];
              }
            }
          } else {
            for (index = 0; index < rowData.length; index += 1) {
              var current = rowMap[rowData[index][0]];
              if (current) setMemRow(current, rowData[index][1], rowData[index][2], rowData[index][3], rowData[index][4]);
            }
          }
        }
      }
    }
    if (storage) {
      var body = cached(storage, "body", ".storage-body");
      var mountTip = tipText("mount");
      var mounts = body ? cachedList(body, "mounts", ".mount") : [];
      for (var mount = 0; mount < mountCount; mount += 1) {
        var node = mounts[mount];
        if (!node) {
          node = newMount(frame[at]);
          if (body) {
            body.appendChild(node);
            mounts.push(node);
          }
        }
        setMount(node, frame, at, mountTip);
        at += 9;
      }
      if (body && mountCount < mounts.length) {
        for (index = mountCount; index < mounts.length; index += 1) mounts[index].remove();
        nodeCache(body).mounts = undefined;
      }
    }
  }

  // The syncs the htmx:sseMessage handler runs after an out-of-band frame.
  // A data frame is applied here instead, so htmx cancels the swap and never
  // fires its own message event.
  function afterValueFrame(type) {
    syncRescale();
    fitChangedGauges();
    if (type === "mem") syncBandsIfChanged();
    syncMotion();
    syncNumbers();
    refreshTip();
  }

  /* ---------- plot motion on its own clock (ACC-GRAPH-98-ANIM-CLOCK) ---------- */

  // Between readouts every panel chart scrolls left at the display frame rate:
  // one slow slot is the plot width times the slow period over the chart's
  // window, and the server draws each new slow pass exactly that far left, so
  // the new path continues the motion instead of restarting it. Only the svg
  // moves; the grid under the box, the axes, the labels and the process row
  // sparklines stay. The server's tail, a sixth of the plot's width, keeps the
  // curve at the right edge and holds the newest smoothed knot, never a raw
  // sample.
  // DEF-PERF-77-PAGE-COST: the move is a linear compositor animation of the
  // chart's svg, not a style write per display frame, so the main thread runs
  // no frame pipeline while the charts scroll. ACC-PERF-133-PAGE-COST: the svg
  // is a CSS box, so the animation's style update needs no layout, where a
  // <g>'s transform lays the SVG out and tests the hover again on every frame.
  var MOTION_KEY = "btop-web-motion";
  var motionPref = true;
  try {
    motionPref = window.localStorage.getItem(MOTION_KEY) !== "off";
  } catch (error) {
    // Storage is optional; motion stays on for this page view.
  }

  // The OS reduced-motion query is not consulted: the owner's machine sets it
  // on every page, so only the CONFIG motion toggle stops the clock.
  function motionAllowed() {
    return motionPref && windowMs > 0 && lastChartAt > 0 && !paused && !document.hidden;
  }

  // DEF-GRAPH-130-PLOT-STEPS: a popup plot's place is a function of the clock and
  // of the pass its path holds, not of when the path arrived: pass n's time on
  // this page is detailAnchor + n * slowMs. The scroll runs on past one slot at
  // the same speed, far enough to cross a stall of 20 s, so a late body shows as
  // the curve's end receding from the right edge, and a body that lands fills in
  // data at the place the curve has already reached.
  // shortcut: the anchor only falls, so a server that lost time while the popup
  // stayed open (a late tick skipped, a suspended host) leaves the curve's end
  // that far from the edge until the popup opens again.
  var DETAIL_RUN_MS = 30000;
  var PASSES_KEPT = 16; // the newest passes a new popup's anchor reads
  var passZeros = []; // each pass's arrival, as the time of pass 0 it implies
  var detailAnchor = null;

  // The earliest arrival is the one least delayed, so a late one moves nothing,
  // and a new anchor can only move a curve left.
  function notePass(seq) {
    var zero = window.performance.now() - seq * slowMs;
    passZeros.push(zero);
    if (passZeros.length > PASSES_KEPT) passZeros.shift();
    if (detailAnchor === null || zero < detailAnchor) detailAnchor = zero;
  }

  function forgetPasses() {
    passZeros = [];
    detailAnchor = null;
  }

  // ACC-PERF-220-FPS-CAP: a pass shows at most 25 steps a second, one step per
  // 40 ms at least. The count comes from the pass's own duration, so the same
  // displacement is still travelled inside the same time.
  var STEP_MS = 40;

  function shiftSteps(ms) {
    return Math.max(1, Math.floor(ms / STEP_MS));
  }

  // A CSS animation, as a panel chart's, with its start set to the pass's time,
  // so the new path and the new phase land in one frame on the same compositor
  // layer. A paused or hidden page, the motion toggle off, a body that says
  // unavailable and a process that ended hold the plot where it stands. A stale
  // page does not: the stale timer fires after two ticks, so a proxy that holds
  // the stream for a moment would show as a stop and then a step; a stream that
  // stays away ends at the run's end instead.
  function syncDetailPlot(svg, allowed) {
    var holder = closest(svg, "[data-slow-seq]");
    var at = null;
    if (allowed && holder && detailState.hidden && detailEnded.hidden && detailAnchor !== null) {
      at = detailAnchor + Number(holder.getAttribute("data-slow-seq")) * slowMs;
    }
    if (svg._at === at) return;
    svg._at = at;
    if (at !== null) {
      svg.style.setProperty("--shift-to", (-100 * DETAIL_RUN_MS / windowMs).toFixed(3) + "%");
      svg.style.animationDuration = DETAIL_RUN_MS + "ms";
      svg.style.setProperty("--shift-steps", String(shiftSteps(DETAIL_RUN_MS)));
      svg.style.animationName = "chart-shift-a";
    }
    var run = svg.getAnimations()[0];
    if (!run) return;
    if (at === null) run.pause();
    else run.startTime = at;
  }

  // Reconcile each chart's animation with the chart clock: start it on the svg
  // the last frame brought, with the negative delay that continues the slow
  // pass's phase, and freeze it where it stands whenever motion is not allowed.
  // A CSS animation, not one created from script: a script animation on a node
  // this new runs on the main thread until it is promoted. One slow slot is
  // that share of the svg's own width, travelled over one slow period.
  // DEF-GRAPH-128-PLOT-JUMP: a chart restarts only once its own new path is on
  // the page, or the old path jumps back one slot until the new one comes. A
  // panel chart's path comes in its own SSE event, whose patch stamps svg._seq.
  // A popup plot's comes by its own fetch and runs on the pass's time.
  // DEF-GRAPH-134-TOGGLE-STEP: a frozen chart starts again at the larger of its own
  // phase and the clock's phase, so it never moves right, and it moves left only by
  // the difference when the clock's is larger. A new path comes a slot further left,
  // so a frozen chart's phase returns to zero with that path and is not larger than
  // the clock's when motion returns.
  // DEF-LAYOUT-133-GPU-LATE: a GPU line gets no sample until a wake's first sweep
  // finishes, so a pass's path can come less than a slot further left. A lane that
  // begins a pass without a reading stays still until a pass begins with one.
  function syncMotion() {
    var groups = document.querySelectorAll("[data-chart] g.series, [data-chart] g.gpu-series");
    var allowed = motionAllowed() && !stale;
    var elapsed = Math.min(slowMs, Math.max(0, window.performance.now() - lastChartAt));
    for (var index = 0; index < groups.length; index += 1) {
      var svg = groups[index].ownerSVGElement;
      if (closest(svg, '[data-chart^="proc-detail"]')) {
        syncDetailPlot(svg, motionAllowed());
        continue;
      }
      var newPass = svg._pass !== svg._seq;
      if (newPass) {
        svg._pass = svg._seq;
        var lane = closest(svg, ".gpu-lane");
        svg._unread = !!lane && !lane.querySelector(".legend-value > [data-w]");
      }
      var moves = allowed && !svg._unread;
      if (svg._run === "run") {
        if (!moves) {
          svg.style.animationPlayState = "paused";
          svg._run = "pause";
          continue;
        }
        if (!newPass) continue;
      }
      if (!moves) {
        var held = newPass && svg._run === "pause" && !svg._unread && svg.getAnimations()[0];
        if (held) held.currentTime = held.effect.getTiming().delay;
        continue;
      }
      var phase = elapsed;
      var frozen = svg._run === "pause" && svg.getAnimations()[0];
      if (frozen) phase = Math.max(phase, frozen.effect.getComputedTiming().progress * slowMs);
      // A changed animation name is what restarts a CSS animation, and two
      // identical keyframe sets alternate so a resume restarts in one write.
      svg._flip = !svg._flip;
      svg.style.setProperty("--shift-to", (-100 * slowMs / windowMs).toFixed(3) + "%");
      svg.style.animationDuration = slowMs + "ms";
      svg.style.setProperty("--shift-steps", String(shiftSteps(slowMs)));
      svg.style.animationDelay = -phase + "ms";
      svg.style.animationName = svg._flip ? "chart-shift-b" : "chart-shift-a";
      svg.style.animationPlayState = "running";
      svg._run = "run";
    }
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
    if (!motionPref) cancelNumberRuns();
    syncMotionControls();
    syncMotion();
  });

  document.addEventListener("visibilitychange", syncMotion);

  /* ---------- number change effects (ACC-VIEW-108-NUM-MOTION) ---------- */

  // The setting is per browser, stored like the motion toggle. Effects run on
  // every [data-w] host (ACC-VIEW-231-ALL-NUM, overriding DEF-VIEW-81). A host
  // reserves the widest form its data-w declares; a host whose neighbours
  // cannot move carries an empty data-w (fixed-width table cells, the proc
  // bar's free space, the absolute .g-num gauge overlay), so a tween or an
  // overlay moves no neighbour (ACC-LAYOUT-74). Tabular figures give 0 the
  // advance and height of every other digit. Only the app's motion toggle
  // stops them; the OS reduced-motion query is not consulted, because the
  // owner's machine sets it on every page.
  var NUM_KEY = "btop-web-num-motion";
  var NUM_MODES = ["none", "spring", "blur", "odometer", "spring-blur"];
  // ACC-VIEW-120: one discrete effect lasts at most four fifths of the current
  // interval, so it settles before the next reading; NUM_MS caps it at the
  // 250 ms interval's own duration.
  var NUM_MS = 200;
  // ACC-VIEW-203: a spring retargets, so it need not fit one interval. Each
  // retarget resets this span, so the number rests this soon after the value
  // stops.
  var NUM_SPRING_MS = 800;

  // ACC-VIEW-126: a change starts an effect only when it moves at least this
  // fraction of the value's display range (data-range on the stat); a smaller
  // change updates the text alone, so the motion settles between changes.
  var NUM_RATE = 0.01;

  function numDuration() {
    var span = document.getElementById("update-interval");
    var ms = span ? Number(span.getAttribute("data-update-ms")) || updateMs : updateMs;
    return Math.max(1, Math.min(NUM_MS, ms * 0.8));
  }

  // ACC-VIEW-120: every started effect ends exactly once, naturally or on a
  // stop, so the two counts on <html> prove no transition was cut.
  var numStarted = 0;
  var numCompleted = 0;

  function numCount() {
    document.documentElement.setAttribute("data-animations-started", String(numStarted));
    document.documentElement.setAttribute("data-animations-completed", String(numCompleted));
  }

  numCount();
  // Cap on echoes: each one is another painted text layer; past three the
  // opacity is too faint to read, so more would cost paint time per tick and
  // add nothing visible.
  var NUM_ECHOES = 3;
  var numMode = "none";
  try {
    var numStored = window.localStorage.getItem(NUM_KEY);
    if (NUM_MODES.indexOf(numStored) !== -1) numMode = numStored;
  } catch (error) {
    // Storage is optional; none holds for this page view.
  }
  // One record per [data-w] host: the last reading, the unit beside it and the
  // texts the echoes may show. A record follows its host's element, so a panel
  // or row that moves keeps its history; a place is matched only when a frame
  // replaced that host's node (see syncNumbers).
  var numStates = [];
  var numByEl = new WeakMap();

  function numParts(text) {
    var match = /^(\D*)(\d+(?:\.\d+)?)([\s\S]*)$/.exec(text);
    if (!match) return null;
    var dot = match[2].indexOf(".");
    return {
      pre: match[1],
      value: Number(match[2]),
      decimals: dot === -1 ? 0 : match[2].length - dot - 1,
      post: match[3]
    };
  }

  function numFormat(parts, value) {
    var text = parts.decimals > 0 ? value.toFixed(parts.decimals) : String(Math.round(value));
    return parts.pre + text + parts.post;
  }

  // The stat's full scale in the unit its text shows: 100 for a percentage,
  // the memory total, the process graph's ceiling. A stat without one has no
  // threshold, so every change animates.
  function numRange(el) {
    var range = Number(el.getAttribute("data-range"));
    return range > 0 ? range : 0;
  }

  function numSignificant(el, before, after) {
    var range = numRange(el);
    return !range || Math.abs(after.value - before.value) >= range * NUM_RATE;
  }

  // The unit or qualifier the template puts just beside a value: the network
  // legend's " MiB/s" text node, the memory tile's unit span. A unit step
  // changes that text without changing the host's own digits, so a tween must
  // not cross it. A sibling that is a host, or a wrapper holding other hosts,
  // belongs to other numbers and is no unit.
  function hostUnit(el) {
    var next = el.nextSibling;
    if (!next) return "";
    if (next.nodeType === 3) return next.nodeValue;
    if (next.nodeType !== 1 || next.hasAttribute("data-w") || next.querySelector("[data-w]")) return "";
    return next.textContent;
  }

  function numLine(el) {
    var line = parseFloat(window.getComputedStyle(el).lineHeight);
    return line > 0 ? line : el.getBoundingClientRect().height;
  }

  // An echo is the old text in an absolute layer over the current value, so
  // it fades without moving anything. z-index -1 puts it behind the number.
  function numEcho(el, text, opacity, drift, duration) {
    var layer = document.createElement("span");
    layer.className = "num-echo";
    layer.textContent = text;
    layer.setAttribute("aria-hidden", "true");
    el.appendChild(layer);
    var animation = layer.animate(
      [
        { opacity: String(opacity), transform: "translateY(0)" },
        { opacity: "0", transform: "translateY(" + drift + "px)" }
      ],
      { duration: Math.max(1, duration), easing: "ease-out", fill: "forwards" }
    );
    animation.onfinish = function () {
      layer.remove();
    };
    return { layer: layer, animation: animation };
  }

  // ACC-VIEW-120: a frame replaces a panel's nodes, so a running effect carries
  // its own layers into the node that replaced them and keeps its animation
  // state. A continuous effect retargets at once; a discrete one applies the
  // newest reading when it ends, so a burst coalesces to the latest value. The
  // host marker is an attribute, not a class: htmx settles the class and style
  // of a swapped element back to the server's own, which would undo a class
  // 20 ms in.
  function numAdopt(state, el) {
    if (state.el === el) return;
    var old = state.el;
    old.removeAttribute("data-num-host");
    el.textContent = "";
    while (old.firstChild) el.appendChild(old.firstChild);
    el.setAttribute("data-num-host", "");
    state.el = el;
    numByEl.set(el, state);
    el._numState = state;
  }

  // A run carries an optional retarget for continuous motion: a spring takes
  // the newest reading as its target and keeps its position and velocity
  // (ACC-VIEW-120/121). A run without one is a discrete transition that holds
  // the newest reading until it ends.
  function numBegin(state, stop, retarget) {
    state.run = { stop: stop, retarget: retarget || null };
    numStarted += 1;
    numCount();
  }

  // End one effect: its node shows the newest reading, never a value that a
  // later frame replaced.
  function numFinish(state) {
    if (!state.run) return;
    state.run = null;
    numCompleted += 1;
    numCount();
    state.el.textContent = state.text;
    state.el.removeAttribute("data-num-host");
    // Only a node the applier writes carries a text shadow; an out-of-band
    // panel's node is read back on the next sync.
    var cache = state.el._cache;
    if (cache && typeof cache.text === "string") cache.text = state.text;
  }

  // A spring count-up: the text is rewritten from a critically damped spring,
  // so the digits ease in and travel instead of swapping. k = 100 and c = 20,
  // because the owner asked to "make spring less 'sudden', more gradual,
  // smoothened" (ACC-VIEW-203): from rest it covers 90 percent of a change in
  // about 390 ms with no overshoot, so a value that changes on every tick glides
  // toward the newest reading. With trail, each intermediate value also leaves
  // an echo.
  // The closed form, not a per-frame Euler step, so the motion does not depend
  // on the display cadence.
  //
  // ACC-VIEW-120/121: this is continuous motion. A new reading mid-run is a
  // new target for the same mass: the spring starts again from the position
  // and velocity the number has now, so it neither jumps back nor restarts.
  // The settle timer moves with it, so the number comes to rest about
  // NUM_SPRING_MS after the last reading.
  function numSpring(state, text, before, after, trail) {
    var ms = NUM_SPRING_MS;
    // The echo drifts a fifth of the host's own line, so a footer-sized
    // number's ghost stays inside its line; the 6 px cap keeps the stat look.
    var drift = -Math.min(6, numLine(state.el) * 0.2);
    // The echoes keep a discrete effect's spacing and lifetime, so a longer
    // spring shows no more of them at once.
    var echoMs = numDuration();
    var omega = Math.sqrt(100);
    var parts = after;
    var from = before.value;
    var to = after.value;
    var velocity = 0;
    var started = window.performance.now();
    var echoAt = started;
    var echoes = [];
    var frame = 0;
    var timer = 0;
    // The tween is decorative: the sr-only copy carries the settled reading,
    // so a screen reader during the spring reads a value the page displays
    // (matching the odometer's aria-hidden cells and plain copy).
    var reading = document.createElement("span");
    reading.className = "sr-only";
    reading.textContent = text;
    var tween = document.createElement("span");
    tween.className = "num-tween";
    tween.setAttribute("aria-hidden", "true");
    state.el.textContent = "";
    state.el.appendChild(reading);
    state.el.appendChild(tween);
    state.el.setAttribute("data-num-host", "");

    // The spring's position and velocity at `now`, from the current run. A
    // frame's timestamp can precede `started` after a long swap, and the closed
    // form is not valid before zero.
    function motion(now) {
      var t = Math.max(0, now - started) / 1000;
      var decay = Math.exp(-omega * t);
      var gap = from - to;
      var gain = velocity + omega * gap;
      return {
        value: to + decay * (gap + gain * t),
        velocity: decay * (velocity - omega * gain * t)
      };
    }

    function draw(number) {
      if (number < 0 && from >= 0 && to >= 0) number = 0;
      tween.textContent = numFormat(parts, number);
    }

    function finish() {
      window.clearTimeout(timer);
      window.cancelAnimationFrame(frame);
      for (var echo = 0; echo < echoes.length; echo += 1) {
        echoes[echo].animation.cancel();
        echoes[echo].layer.remove();
      }
      numFinish(state);
    }

    // A new reading retargets the running spring. A different prefix, suffix
    // or decimal count cannot tween: end here, showing this reading at once.
    function retarget(text) {
      var next = numParts(text);
      if (!next || next.pre !== parts.pre || next.post !== parts.post ||
          next.decimals !== parts.decimals) {
        finish();
        return false;
      }
      reading.textContent = text;
      var now = window.performance.now();
      var at = motion(now);
      from = at.value;
      velocity = at.velocity;
      to = next.value;
      parts = next;
      started = now;
      window.clearTimeout(timer);
      timer = window.setTimeout(finish, ms);
      return true;
    }

    function step(now) {
      var at = motion(now);
      if (now - started >= ms) {
        finish();
        return;
      }
      draw(at.value);
      if (trail && now - echoAt >= echoMs / 8) {
        echoAt = now;
        echoes.push(numEcho(state.el, tween.textContent, 0.3, drift, Math.min(echoMs, ms - (now - started))));
        if (echoes.length > NUM_ECHOES) {
          var oldest = echoes.shift();
          oldest.animation.cancel();
          oldest.layer.remove();
        }
      }
      frame = window.requestAnimationFrame(step);
    }

    draw(from);
    // The timer, not the display cadence, ends the effect.
    timer = window.setTimeout(finish, ms);
    numBegin(state, finish, retarget);
    frame = window.requestAnimationFrame(step);
  }

  // Motion blur: the last values sit behind the current one at falling
  // opacity and drift away. Only the current text is real; the layers are
  // aria-hidden and removed within the effect's duration. A discrete
  // transition: the trail has one fixed end state, so a reading arriving
  // mid-run is applied when the trail is gone (ACC-VIEW-120).
  function numBlur(state, text, history) {
    var ms = numDuration();
    // The same line-relative drift as the spring: a small host's echo stays
    // inside its own line.
    var drift = -Math.min(6, numLine(state.el) * 0.2);
    var opacity = [0.3, 0.17, 0.09];
    var echoes = [];
    var tween = document.createElement("span");
    tween.className = "num-tween";
    tween.textContent = text;
    state.el.textContent = "";
    state.el.appendChild(tween);
    state.el.setAttribute("data-num-host", "");
    for (var echo = 0; echo < history.length && echo < NUM_ECHOES; echo += 1) {
      echoes.push(numEcho(state.el, history[echo], opacity[echo], drift, ms));
    }

    function finish() {
      window.clearTimeout(timer);
      numFinish(state);
    }

    function stop() {
      for (var at = 0; at < echoes.length; at += 1) {
        echoes[at].animation.cancel();
        echoes[at].layer.remove();
      }
      finish();
    }

    var timer = window.setTimeout(finish, ms);
    numBegin(state, stop, null);
  }

  // Odometer: each changed digit rolls through a 0-9 strip, up or down with
  // the value. The cells are inline, so they take the host's own layout and
  // the effect reads no geometry: with tabular figures every digit keeps its
  // advance and 0 stands in the place of any other. A width change aligns the
  // old reading on the new one's right edge, so a carry rolls in its new place
  // and a new leading digit appears static.
  // A discrete transition: the strips have fixed stops, so a reading arriving
  // mid-roll is applied when the roll ends (ACC-VIEW-120).
  function numOdometer(state, oldText, newText, before, after) {
    var ms = numDuration();
    var line = numLine(state.el);
    // DEF-VIEW-136-ODO-DROP: a slot is a whole number of device px, so every stop
    // paints on the device row the resting text paints on.
    var ratio = window.devicePixelRatio || 1;
    var slotHeight = Math.round(line * ratio) / ratio;
    var up = after.value >= before.value;
    var old = oldText.length >= newText.length
      ? oldText.slice(oldText.length - newText.length)
      : " ".repeat(newText.length - oldText.length) + oldText;
    state.el.textContent = "";
    // The roll stops are decorative: without a plain copy the host's only
    // text is the concatenated stops, so a screen reader or a selection
    // during the roll reads a number the page never displays.
    var reading = document.createElement("span");
    reading.className = "sr-only";
    reading.textContent = newText;
    state.el.appendChild(reading);
    state.el.setAttribute("data-num-host", "");
    var running = [];
    for (var at = 0; at < newText.length; at += 1) {
      var cell = document.createElement("span");
      cell.className = "num-odo-cell";
      cell.setAttribute("aria-hidden", "true");
      cell.style.lineHeight = line + "px";
      cell.style.height = line + "px";
      var oldGlyph = old.charAt(at);
      var glyph = newText.charAt(at);
      var rolls = oldGlyph >= "0" && oldGlyph <= "9" && glyph >= "0" && glyph <= "9" &&
        oldGlyph !== glyph;
      if (rolls) {
        var fromDigit = Number(oldGlyph);
        var toDigit = Number(glyph);
        var strip = document.createElement("span");
        strip.className = "num-odo-strip";
        // The strip holds only the stops the roll crosses, so a carry of one
        // step builds two digits instead of the old twenty.
        var offset = 0;
        var spot = 0;
        if (up) {
          spot = toDigit >= fromDigit ? toDigit : toDigit + 10;
          offset = spot - fromDigit;
          for (var slot = fromDigit; slot <= spot; slot += 1) {
            var digit = document.createElement("span");
            digit.textContent = String(slot % 10);
            digit.style.height = slotHeight + "px";
            strip.appendChild(digit);
          }
        } else {
          spot = toDigit <= fromDigit ? toDigit : toDigit - 10;
          offset = fromDigit - spot;
          for (slot = spot; slot <= fromDigit; slot += 1) {
            digit = document.createElement("span");
            digit.textContent = String((slot + 20) % 10);
            digit.style.height = slotHeight + "px";
            strip.appendChild(digit);
          }
        }
        var start = up ? 0 : offset;
        var end = up ? offset : 0;
        strip.style.transform = "translateY(" + -start * slotHeight + "px)";
        cell.appendChild(strip);
        running.push(strip.animate(
          [
            { transform: "translateY(" + -start * slotHeight + "px)" },
            { transform: "translateY(" + -end * slotHeight + "px)" }
          ],
          { duration: ms, easing: "cubic-bezier(.33,0,.2,1)", fill: "forwards" }
        ));
      } else {
        var fixed = document.createElement("span");
        fixed.className = "num-odo-static";
        fixed.textContent = glyph;
        cell.appendChild(fixed);
      }
      state.el.appendChild(cell);
    }

    function finish() {
      window.clearTimeout(timer);
      numFinish(state);
    }

    function stop() {
      for (var at = 0; at < running.length; at += 1) running[at].cancel();
      finish();
    }

    var timer = window.setTimeout(finish, ms);
    numBegin(state, stop, null);
  }

  function playNumber(state, oldText, newText, history, shown) {
    if (numMode === "none" || !motionPref || document.hidden) return;
    if (!shown) return;
    var before = numParts(oldText);
    var after = numParts(newText);
    if (!before || !after || before.value === after.value) return;
    // A changed prefix, suffix or number of decimals is a unit or scale
    // change; a tween between different forms would read as invented digits.
    if (before.pre !== after.pre || before.post !== after.post || before.decimals !== after.decimals) return;
    // ACC-VIEW-126: a change under the stat's own rate threshold updates the
    // text without moving anything, so the effect settles between changes.
    if (!numSignificant(state.el, before, after)) return;
    if (numMode === "odometer") {
      numOdometer(state, oldText, newText, before, after);
    } else if (numMode === "blur") {
      numBlur(state, newText, history);
    } else {
      numSpring(state, newText, before, after, numMode === "spring-blur");
    }
  }

  function cancelNumberRuns() {
    for (var index = 0; index < numStates.length; index += 1) {
      if (numStates[index].run) numStates[index].run.stop();
    }
  }

  // A node an applier writes carries the last text written to it; a host no
  // applier writes (an axis label, a host in a panel replaced whole) has no
  // shadow, so its text is read once.
  function hostText(el) {
    var cache = el._cache;
    return cache && cache.gen === cacheGen && typeof cache.text === "string" ? cache.text : el.textContent;
  }

  // After each frame, pair each host with the record it held: by the host's
  // own element, so a panel or row that moves keeps its history, and by place
  // when a frame replaced that host's node. A change starts an effect; the
  // same text does nothing, so a replaced node with an unchanged value stays
  // still and a hidden tab records the truth without replaying. A unit step
  // beside the host ends a running effect at once and starts none: a tween
  // across a 1024x scale would show hundreds of the new unit.
  function syncNumbers() {
    var hosts = document.querySelectorAll("[data-w]");
    // Read every host's visibility up front, before any effect writes a
    // layer: the reads share one layout, where a check inside the loop pays
    // a full layout per started effect (measured at 16 long tasks per 4 s).
    var shown = [];
    for (var at = 0; at < hosts.length; at += 1) shown[at] = hosts[at].getClientRects().length > 0;
    var previous = numStates;
    var sameCount = hosts.length === previous.length;
    for (var old = 0; old < previous.length; old += 1) previous[old].seen = false;
    var states = [];
    for (var index = 0; index < hosts.length; index += 1) {
      var el = hosts[index];
      var text = hostText(el);
      var unit = hostUnit(el);
      var state = numByEl.get(el);
      if (!state && sameCount) {
        // A frame replaced this host's node in place; its record is here.
        // shortcut: index pairing runs only while the [data-w] host count is
        // unchanged, so a host replaced by a frame that adds or removes a
        // process row records without history and snaps; upgrade to a stable
        // row key when templates carry one.
        var placed = previous[index];
        if (placed && !placed.seen && !placed.el.isConnected) state = placed;
      }
      if (!state) {
        state = { el: el, text: text, unit: unit, history: [], run: null, seen: true };
        numByEl.set(el, state);
        el._numState = state;
        states.push(state);
        continue;
      }
      state.seen = true;
      numByEl.set(el, state);
      el._numState = state;
      if (state.run) {
        if (state.el === el) {
          // The node stayed, so its text is the effect's own paint; only a
          // unit step beside it can end the effect early.
          if (state.unit !== unit) {
            state.unit = unit;
            state.history = [];
            state.run.stop();
          }
          states.push(state);
          continue;
        }
        // A frame replaced the node mid-effect: carry the layers across and
        // take this reading as the target (ACC-VIEW-120/121).
        numAdopt(state, el);
        if (state.unit !== unit) {
          state.text = text;
          state.unit = unit;
          state.history = [];
          state.run.stop();
        } else if (state.text !== text) {
          state.text = text;
          if (state.run.retarget) state.run.retarget(text);
        }
        states.push(state);
        continue;
      }
      if (state.el !== el) {
        state.el = el;
        numByEl.set(el, state);
      }
      if (state.text !== text) {
        if (state.unit === unit) {
          state.history = [state.text].concat(state.history).slice(0, NUM_ECHOES);
          playNumber(state, state.text, text, state.history, shown[index]);
        } else {
          state.history = [];
        }
        state.text = text;
      }
      state.unit = unit;
      states.push(state);
    }
    // A host that left the page ends its effect where it stood.
    for (old = 0; old < previous.length; old += 1) {
      var gone = previous[old];
      if (!gone.seen && gone.run) gone.run.stop();
    }
    numStates = states;
  }

  function syncNumControls() {
    var chips = document.querySelectorAll('[data-group="num-motion"] .opt');
    for (var index = 0; index < chips.length; index += 1) {
      var on = chips[index].getAttribute("data-value") === numMode;
      chips[index].classList.toggle("on", on);
      chips[index].setAttribute("aria-checked", String(on));
    }
  }

  // ACC-VIEW-108: a change of effect never interrupts a transition already in
  // flight. That transition finishes under its own mechanism and applies the
  // newest reading; the new choice starts with the next value change. Only the
  // motion toggle, pause and a hidden tab stop a transition early.
  document.addEventListener("click", function (event) {
    var chip = closest(event.target, '[data-group="num-motion"] .opt');
    if (!chip) return;
    var value = chip.getAttribute("data-value");
    if (NUM_MODES.indexOf(value) === -1) return;
    numMode = value;
    try {
      window.localStorage.setItem(NUM_KEY, numMode);
    } catch (error) {
      // Storage is optional; the choice holds for this page view.
    }
    syncNumControls();
  });

  document.addEventListener("visibilitychange", function () {
    if (document.hidden) cancelNumberRuns();
    syncNumbers();
  });

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

  // ACC-VIEW-199-POPUP-DIM: one scrim, shown while the popover or a process's
  // details are open, so closing one leaves it for the other.
  function syncScrim() {
    if (scrim) scrim.hidden = (!config || config.hidden) && detailPid === null;
  }

  function closeConfig(returnTo) {
    if (!config || config.hidden) return;
    config.hidden = true;
    syncScrim();
    setExpanded(false);
    if (!returnTo) return;
    var trigger = returnTo.isConnected ? returnTo : visibleToggle();
    if (trigger) trigger.focus();
  }

  function openConfig(trigger) {
    if (!config) return;
    lastTrigger = trigger;
    config.hidden = false;
    syncScrim();
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
      if (config.hidden) return;
      if (event.key === "Escape") {
        event.preventDefault();
        closeConfig(lastTrigger);
        return;
      }
      if (event.key === "Tab") cycleTab(event, config);
    });
  }

  // ACC-VIEW-104-CONFIG-DIM: Tab and Shift+Tab cycle inside the open box.
  function cycleTab(event, box) {
    var items = [];
    var nodes = box.querySelectorAll("button, select, input, textarea, a[href], [tabindex]");
    for (var index = 0; index < nodes.length; index += 1) {
      if (nodes[index].disabled) continue;
      if (nodes[index].getAttribute("tabindex") === "-1") continue;
      if (nodes[index].getClientRects().length === 0) continue;
      items.push(nodes[index]);
    }
    if (items.length === 0) return;
    var first = items[0];
    var last = items[items.length - 1];
    var active = document.activeElement;
    var outside = !box.contains(active);
    if (event.shiftKey && (outside || active === first)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (outside || active === last)) {
      event.preventDefault();
      first.focus();
    }
  }

  /* ---------- panel layout (ACC-VIEW-40-GADGETS) ---------- */

  var layoutApi = window.btopLayout;
  var layout = layoutApi ? layoutApi.read() : null;
  // ACC-LAYOUT-158: a stored arrangement is restored before the first frame.
  if (layout) layoutApi.apply(layout);

  // A band whose panels are all hidden - by the layout, a viewport drop or an
  // empty panel - gives its height to the others. A child's computed display
  // is its own, so a hidden band does not hide its panels from this check.
  // Only band 3 can empty on a frame: the GPU panel with no device and the
  // storage panel with no mount. Read those two markers, which need no layout,
  // and skip the computed-style sweep while they hold; the layout paths call
  // this function themselves. Each panel not shown carries data-gone, which
  // app.css reads for band 3's slot widths (ACC-LAYOUT-228-EQUAL-WIDTH) and
  // for the phone band heights (DEF-LAYOUT-145-PHONE-BANDS).
  var band3Known = "";

  function band3State() {
    return (document.querySelector('[data-panel="gpu"] .gpu-body > .unavailable') ? "0" : "1") +
      (document.querySelector('[data-panel="storage"] .mount') ? "1" : "0");
  }

  function syncBandsIfChanged() {
    if (band3State() !== band3Known) syncBands();
  }

  function syncBands() {
    band3Known = band3State();
    var bands = document.querySelectorAll(".band");
    for (var index = 0; index < bands.length; index += 1) {
      var panels = bands[index].querySelectorAll(".panel");
      var shown = false;
      for (var inner = 0; inner < panels.length; inner += 1) {
        var gone = window.getComputedStyle(panels[inner]).display === "none";
        panels[inner].toggleAttribute("data-gone", gone);
        if (!gone) shown = true;
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
    applyGpuSelection(); // a drag can move the GPU panel to another band
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
      applyGpuSelection();
    });
  }

  /* ---------- drag a panel to a new place (ACC-LAYOUT-222-DRAG-LIVE) ---------- */

  // The header is the drag handle. A press moves nothing until the pointer
  // travels DRAG_STEP px, so a click leaves the layout alone. During the drag
  // the panel follows the pointer and the panel order is a list: entering
  // another panel inserts the dragged one at its place, and the panels between
  // shift one place over. A release stores what is shown beside the panel
  // settings; Escape or a pointer cancel restores the order the drag started
  // from. Pointer Events carry mouse, touch and pen alike.
  var DRAG_STEP = 4;
  var drag = null; // {panel, id, pointerId, x, y, fx, fy, active, start, under}

  function dragPanel(event) {
    var header = closest(event.target, ".ph");
    var panel = header ? closest(header, ".panel[data-panel]") : null;
    if (!panel) return null;
    if (closest(event.target, "input, select, textarea, button, a")) return null;
    return panel;
  }

  // The dragged panel has pointer-events: none, so this finds the one below it.
  function panelAt(x, y) {
    var under = document.elementFromPoint(x, y);
    return under ? closest(under, ".panel[data-panel]") : null;
  }

  function showOrder(order) {
    layout.order = order;
    layoutApi.apply(layout);
    syncBands();
    applyGpuSelection(); // a move can put the GPU panel in another band
  }

  // The grab point stays under the pointer; the slot, and so the width, of
  // the panel changes as it moves, so the x offset is a share of the width.
  function follow(x, y) {
    var style = drag.panel.style;
    style.transform = "";
    var box = drag.panel.getBoundingClientRect();
    style.transform = "translate(" + (x - box.left - drag.fx * box.width) + "px, " + (y - box.top - drag.fy) + "px)";
  }

  function endDrag(keep) {
    var moved = drag.active && layout.order.join() !== drag.start.join();
    drag.panel.removeAttribute("data-dragging");
    drag.panel.style.transform = "";
    if (moved && keep) commitLayout();
    else if (moved) showOrder(drag.start);
    drag = null;
  }

  if (layout) {
    document.addEventListener("pointerdown", function (event) {
      if (event.button !== 0 || drag) return;
      var panel = dragPanel(event);
      if (!panel) return;
      var box = panel.getBoundingClientRect();
      drag = {
        panel: panel,
        id: panel.getAttribute("data-panel"),
        pointerId: event.pointerId,
        x: event.clientX,
        y: event.clientY,
        fx: (event.clientX - box.left) / box.width,
        fy: event.clientY - box.top,
        active: false,
        start: layout.order.slice(),
        under: null
      };
      event.preventDefault(); // the header is held, not selected
    });

    document.addEventListener("pointermove", function (event) {
      if (!drag || event.pointerId !== drag.pointerId) return;
      if (!drag.active) {
        if (Math.abs(event.clientX - drag.x) < DRAG_STEP && Math.abs(event.clientY - drag.y) < DRAG_STEP) return;
        drag.active = true;
        drag.panel.setAttribute("data-dragging", "");
      }
      var under = panelAt(event.clientX, event.clientY);
      if (under && under !== drag.under) {
        var order = layout.order.slice();
        order.splice(order.indexOf(drag.id), 1);
        order.splice(layout.order.indexOf(under.getAttribute("data-panel")), 0, drag.id);
        showOrder(order);
        // A panel the shift brings under a still pointer is not entered, so
        // two panels of unequal width cannot trade places on every move.
        under = panelAt(event.clientX, event.clientY);
      }
      drag.under = under;
      follow(event.clientX, event.clientY);
    });

    document.addEventListener("pointerup", function (event) {
      if (drag && event.pointerId === drag.pointerId) endDrag(true);
    });

    document.addEventListener("pointercancel", function (event) {
      if (drag && event.pointerId === drag.pointerId) endDrag(false);
    });

    document.addEventListener("keydown", function (event) {
      if (event.key !== "Escape" || !drag || !drag.active) return;
      event.preventDefault();
      endDrag(false);
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

  // The lane count reserves height on whichever band holds the GPU panel: a
  // swap can move it out of band 3 (ACC-LAYOUT-158), and the band it left must
  // not keep the reservation.
  function gpuBand() {
    var panel = document.getElementById("p-gpu");
    return panel ? panel.parentNode : null;
  }

  // A write to the band's custom property restyles every panel under it, so an
  // unchanged count writes nothing.
  function setGpuLanes(count) {
    var band = count > 0 ? gpuBand() : null;
    var bands = document.querySelectorAll(".band");
    for (var index = 0; index < bands.length; index += 1) {
      var lanes = bands[index] === band ? String(count) : "";
      if (bands[index].style.getPropertyValue("--gpu-lanes") === lanes) continue;
      bands[index].toggleAttribute("data-gpu-lanes", lanes !== "");
      if (lanes) bands[index].style.setProperty("--gpu-lanes", lanes);
      else bands[index].style.removeProperty("--gpu-lanes");
    }
  }

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
      setGpuLanes(0);
      syncGpuChips(gpuSelection);
      return;
    }
    var selection = gpuSelection.filter(function (index) {
      return index < boxes.length;
    });
    // Rows that arrive after a first paint with none read the stored choice here.
    if (selection.length === 0) selection = readGpuSelection(boxes.length);
    gpuSelection = selection;
    // The lane of a GPU and its legend entry both carry data-gpu.
    var items = document.querySelectorAll("#p-gpu [data-gpu]");
    for (var index = 0; index < items.length; index += 1) {
      var on = selection.indexOf(Number(items[index].getAttribute("data-gpu"))) !== -1;
      items[index].setAttribute("data-selected", String(on));
    }
    // The GPU's band reserves 18 px per visible lane (app.css, .band[data-gpu-lanes]).
    setGpuLanes(selection.length);
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
    if (pendingFrames.cpu) applyCpuFrame(pendingFrames.cpu);
    if (pendingFrames.mem) applyMemFrame(pendingFrames.mem);
    pendingFrames = {};
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
  // ACC-VIEW-140-COLUMN-MENU: the hidden process column set, this browser's own.
  var PROC_COL_KEY = "btop-web-proc-cols";
  var PROC_VIEWS = ["top", "all", "tree"];
  // The sort key of each column, in proc.html's cell order.
  var PROC_KEYS = ["pid", "name", "command", "threads", "user", "mem", "cpu"];
  var procWrap = document.getElementById("proc-table-wrap");
  var procView = readProcView();
  var procCols = readProcCols();
  var procHold = false;
  var procFocus = null;
  var collator = new Intl.Collator();

  // view: the stored choice, else the server's starting view (#p-proc
  // data-proc-view); `chosen` keeps a viewer who never picked a view on it.
  function readProcView() {
    var panel = document.getElementById("p-proc");
    var start = panel ? panel.getAttribute("data-proc-view") : null;
    var state = { view: PROC_VIEWS.indexOf(start) !== -1 ? start : "top", chosen: null, level: 3, toggled: {}, sort: "mem", descending: true };
    try {
      var stored = JSON.parse(window.localStorage.getItem(PROC_KEY));
      if (!stored || stored.v !== 1) return state;
      if (PROC_VIEWS.indexOf(stored.view) !== -1) state.view = state.chosen = stored.view;
      if (stored.level >= 1 && stored.level <= 9) state.level = Math.round(stored.level);
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
      window.localStorage.setItem(PROC_KEY, JSON.stringify({ v: 1, view: procView.chosen || undefined, level: procView.level, toggled: procView.toggled, sort: procView.sort, descending: procView.descending }));
    } catch (error) {
      // Storage is optional; the choice holds for this page view.
    }
    syncProcControls();
    applyProc();
  }

  // ACC-VIEW-140-COLUMN-MENU: the header's right-click menu switches a column
  // off for this browser. Program (column 1) carries the tree and is never
  // hidden; the server still sends every cell, so the frames do not change.
  function readProcCols() {
    try {
      var stored = JSON.parse(window.localStorage.getItem(PROC_COL_KEY));
      if (Array.isArray(stored)) {
        return PROC_KEYS.filter(function (key) {
          return key !== PROC_KEYS[1] && stored.indexOf(key) !== -1;
        });
      }
    } catch (error) {
      // Storage is optional; every column stays on for this page view.
    }
    return [];
  }

  function colHidden(key) {
    return key !== PROC_KEYS[1] && procCols.indexOf(key) !== -1;
  }

  // The colgroup entry is what frees the column's width under table-layout:
  // fixed; the header and the cells go with it, so nothing clips.
  function applyProcCols() {
    var cols = procWrap.querySelectorAll("colgroup col");
    var heads = procWrap.querySelectorAll("thead th");
    var rows = procWrap.querySelector("tbody").rows;
    for (var index = 0; index < PROC_KEYS.length; index += 1) {
      var off = colHidden(PROC_KEYS[index]);
      if (cols[index]) cols[index].classList.toggle("col-off", off);
      if (heads[index]) heads[index].classList.toggle("col-off", off);
      for (var row = 0; row < rows.length; row += 1) {
        if (rows[row].cells[index]) rows[row].cells[index].classList.toggle("col-off", off);
      }
    }
  }

  // The config menu's sort select and direction chips mirror the column headers.
  var procSort = document.getElementById("proc-sort");

  function syncProcControls() {
    var chosen = { "proc-view": procView.view, "proc-level": String(procView.level), "proc-dir": procView.descending ? "desc" : "asc" };
    var chips = document.querySelectorAll('[data-group="proc-view"] .opt, [data-group="proc-level"] .opt, [data-group="proc-dir"] .opt');
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

  // The value cells hold a [data-w] span, so a number effect can own the text.
  // The template writes one where the value exists; a row whose value is
  // unavailable gets an empty one here, so every row's cells keep their place.
  function procValueSpan(cell) {
    var span = cell.querySelector("[data-w]");
    if (!span) {
      span = document.createElement("span");
      span.setAttribute("data-w", "");
      cell.appendChild(span);
    }
    return span;
  }

  // One row's data, parsed once per frame.
  function procRow(row) {
    var cells = row.cells;
    var raw = (row.getAttribute("data-raw") || "").split(" ");
    var cpuValue = procValueSpan(cells[6]);
    var memValue = procValueSpan(cells[5]);
    var threadsValue = procValueSpan(cells[3]);
    var p = {
      row: row,
      pid: Number(cells[0].textContent),
      depth: Number(raw[0]) || 0,
      own: { cpu: rawNumber(raw[1]), mem: rawNumber(raw[2]), threads: rawNumber(raw[3]) },
      // Cpu%, MemB and Threads labels and the Cpu% class, own and summed.
      ownLabels: [cpuValue.textContent, memValue.textContent, threadsValue.textContent, cells[6].className],
      sumLabels: null,
      // btop filters on pid, name, command and user: cells 0, 1, 2 and 4.
      texts: [cells[0].textContent, cells[1].textContent, cells[2].textContent, cells[4].textContent],
      cpuValue: cpuValue,
      memValue: memValue,
      threadsValue: threadsValue
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

  // ACC-VIEW-102-TREE-SORT: a tree row ranks by its whole branch, folded or not;
  // a flat row ranks by the value it shows.
  function procValue(p) {
    var values = procView.view === "tree" || p.collapsed ? p.sums : p.own;
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

  // ACC-VIEW-180-PROC-TREE: the prefix is drawn as elements at one fixed
  // step per level, never typed, so it does not depend on a glyph's advance
  // in the current face. `sig` carries one letter per ancestor level, "l"
  // for a rule that continues and "b" for a blank slot, then the row's own
  // elbow: "m" middle sibling, "s" last sibling, "f" first root. The step
  // elements and the button are reused; only a changed signature writes.
  function treeGlyphs(p, sig) {
    var cell = p.row.cells[1];
    if (!p.tree) {
      p.tree = document.createElement("span");
      p.tree.className = "tree";
      p.tree.setAttribute("aria-hidden", "true");
    }
    var leaf = p.kids.length === 0;
    if (p.leaf !== leaf) {
      p.leaf = leaf;
      p.tree.className = leaf ? "tree leaf" : "tree";
    }
    if (p.sig !== sig) {
      p.sig = sig;
      var branch = { m: "tb-mid", s: "tb-last", f: "tb-first" }[sig.charAt(sig.length - 1)];
      var count = sig.length;
      while (p.tree.childNodes.length < count) p.tree.appendChild(document.createElement("i"));
      while (p.tree.childNodes.length > count) p.tree.removeChild(p.tree.lastChild);
      for (var at = 0; at < count; at += 1) {
        var step = at === count - 1 ? "tstep tbranch " + branch : sig.charAt(at) === "l" ? "tstep tline" : "tstep";
        if (p.tree.childNodes[at].className !== step) p.tree.childNodes[at].className = step;
      }
    }
    if (cell.firstChild !== p.tree) cell.insertBefore(p.tree, cell.firstChild);
    if (leaf) {
      if (p.twisty && p.twisty.parentNode) p.twisty.parentNode.removeChild(p.twisty);
      return;
    }
    if (!p.twisty) {
      p.twisty = document.createElement("button");
      p.twisty.type = "button";
      p.twisty.className = "twisty";
      // ACC-VIEW-211-ROW-KEYS: Tab passes the list body.
      p.twisty.tabIndex = -1;
      p.twisty.setAttribute("aria-label", "children of " + p.texts[1] + " (pid " + p.pid + ")");
      p.twisty.appendChild(twistyIcon());
    }
    var expanded = String(!p.collapsed);
    if (p.twisty.getAttribute("aria-expanded") !== expanded) {
      p.twisty.setAttribute("aria-expanded", expanded);
      // DEF-VIEW-105-TOGGLE-GONE: a rotated svg turns the shape about its own
      // 0 0 and clips it, so the path itself points right or down instead.
      p.twisty.firstChild.firstChild.setAttribute("d", p.collapsed ? "M0 0L4 2.5L0 5Z" : "M0 0L4 0L2 5Z");
    }
    if (p.tree.nextSibling !== p.twisty) cell.insertBefore(p.twisty, p.tree.nextSibling);
  }

  // The toggle's icon: a solid triangle that points right when collapsed and
  // down when expanded; treeGlyphs swaps the path as aria-expanded changes.
  // aria-label names the subtree.
  function twistyIcon() {
    var svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 4 5");
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("focusable", "false");
    var path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", "M0 0L4 2.5L0 5Z");
    svg.appendChild(path);
    return svg;
  }

  function dropGlyphs(p) {
    [p.tree, p.twisty].forEach(function (node) {
      if (node && node.parentNode) node.parentNode.removeChild(node);
    });
  }

  // ACC-PERF-16-NO-OVERHEAD: a frame updates the table in place instead of
  // replacing it. A row keeps its element, tree steps and toggle, and only
  // what the server changed is written, so the browser lays out and paints
  // the changed rows only. The head is static and stays as it is.
  // Own code: htmx's morph extension would move every row back to the
  // server's order and strip the steps and toggle on each frame.
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
        while (cell.firstChild !== p.cpuValue) cell.removeChild(cell.firstChild);
        while (row.cells[6].firstChild !== q.cpuValue) {
          cell.insertBefore(row.cells[6].firstChild, p.cpuValue);
        }
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

  // The graphs of a Cpu% cell as the server wrote them; the value span comes last.
  function procGraphs(cell) {
    var html = "";
    for (var node = cell.firstChild; node && node.nodeType === 1; node = node.nextSibling) {
      if (node.hasAttribute("data-w")) break;
      html += node.outerHTML;
    }
    return html;
  }

  // ACC-VIEW-231-ALL-NUM: the shown/all process count is two live numbers, so
  // each is its own [data-w] host with "/" between them for the first host's
  // unit; the bar's filter takes the free space to their right
  // (#p-proc .head-right's auto margin), so neither count's width moves a
  // neighbour. With no rows the bar shows the one word instead.
  function setProcCount(shown, total) {
    if (!procCount) return;
    if (!total) {
      if (procCount.textContent !== "unavailable") procCount.textContent = "unavailable";
      return;
    }
    var hosts = procCount.querySelectorAll("[data-w]");
    if (hosts.length !== 2) {
      procCount.textContent = "";
      hosts = [document.createElement("span"), document.createElement("span")];
      hosts[0].setAttribute("data-w", "");
      hosts[1].setAttribute("data-w", "");
      procCount.appendChild(hosts[0]);
      procCount.appendChild(document.createTextNode("/"));
      procCount.appendChild(hosts[1]);
    }
    // The tile's reserve covers every shown count and holds the "/" and the
    // total in place when the shown count changes digit count.
    var tile = document.getElementById("proc-total");
    if (tile) frameAttr(hosts[0], "data-w", tile.getAttribute("data-w"));
    frameText(hosts[0], String(shown));
    frameText(hosts[1], String(total));
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
      // ACC-VIEW-206-FILTER-MATCHES: a query in the top view lists the matches themselves, as the all view does.
      if (view === "all" || (view === "top" && query)) {
        p.collapsed = false;
        p.shown = p.match;
      } else if (view === "top") {
        // pid 1's children are top-level rows of their own.
        p.collapsed = parent && p.pid !== 1;
        p.shown = p.parent === null || p.parent.pid === 1;
      } else {
        // ACC-VIEW-103-TREE-DEPTH: a node at or below the display level starts
        // folded; a toggle click overrides that for its pid until the level
        // changes. A query opens the ancestors of a match without touching the
        // stored set.
        var stored = Object.prototype.hasOwnProperty.call(procView.toggled, p.pid) ? procView.toggled[p.pid] : p.depth >= procView.level;
        p.collapsed = parent && stored && !(query && p.under);
        p.shown = false;
      }
      if (view !== "tree" && p.shown) order.push(p);
    });
    if (view === "tree") {
      // ACC-VIEW-180-PROC-TREE: the same pre-order btop walks in
      // `btop_shared.cpp:181-188` and `btop_collect.cpp:2754-2760`, as the
      // signature treeGlyphs draws: one letter per ancestor level ("l" while
      // that ancestor has a following sibling, "b" under the last one), then
      // the row's own elbow - "s" for the last shown sibling, "f" for the
      // first root, "m" otherwise.
      var walk = function (siblings, bars) {
        var shown = sortProc(siblings).filter(function (p) {
          return p.match || p.under;
        });
        var prefix = bars.map(function (more) {
          return more ? "l" : "b";
        }).join("");
        shown.forEach(function (p, at) {
          var last = at === shown.length - 1;
          p.shown = true;
          order.push(p);
          glyphs[p.pid] = prefix + (last ? "s" : p.depth === 0 && at === 0 ? "f" : "m");
          if (p.kids.length > 0 && !p.collapsed) walk(p.kids, bars.concat([!last]));
        });
      };
      walk(rows.filter(function (p) {
        return p.parent === null;
      }), []);
    } else {
      sortProc(order);
    }

    // DEF-VIEW-65-TREE-GLYPHS: the Program column widens with the level so the
    // prefix never clips; the name ellipsizes into what is left.
    if (procWrap) {
      var level = String(procView.level);
      if (procWrap.style.getPropertyValue("--tree-level") !== level) procWrap.style.setProperty("--tree-level", level);
      procWrap.toggleAttribute("data-tree", view === "tree");
      procWrap.toggleAttribute("data-applied", true);
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
      // ACC-VIEW-205-TREE-TALLY: a tree row shows its whole branch, folded or not, as it ranks in procValue.
      var labels = (view === "tree" || p.collapsed) && p.sumLabels ? p.sumLabels : p.ownLabels;
      frameText(p.cpuValue, labels[0]);
      frameText(p.memValue, labels[1]);
      frameText(p.threadsValue, labels[2]);
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
    // ACC-VIEW-140-COLUMN-MENU: new rows from a frame take the classes again.
    applyProcCols();
    setProcCount(order.length, rows.length);
    restoreProcFocus(focus);
  }

  // The header button a keyboard user is on, as a key that survives a new table.
  function procFocusKey(element) {
    if (!element || !procWrap || !procWrap.contains(element)) return null;
    if (element.tagName === "BUTTON" && element.parentNode.tagName === "TH") return { col: element.parentNode.cellIndex };
    return null;
  }

  function restoreProcFocus(key) {
    if (!key) return;
    var target = procWrap.querySelectorAll("thead th")[key.col].querySelector("button");
    if (document.activeElement !== target) target.focus({ preventScroll: true });
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
      // A query holds this row open (applyProc): a stored fold would land when it clears.
      if (filterInput && filterInput.value.trim() && row._p.under) return;
      procView.toggled[row._p.pid] = !row._p.collapsed;
      commitProc();
    });
    // A frame that lands between press and release can move or replace the
    // control, and the browser would dispatch no click: hold proc frames
    // while a control is pressed, then apply the newest.
    procWrap.addEventListener("pointerdown", function (event) {
      if (closest(event.target, "button, #proc-body tr")) procHold = true;
    });
    // ACC-VIEW-211-ROW-KEYS: a clicked triangle takes no focus, so no key folds it and it draws no outline.
    procWrap.addEventListener("mousedown", function (event) {
      if (closest(event.target, ".twisty")) event.preventDefault();
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
    // ACC-PERF-133-PAGE-COST: the same slow pass brings every chart's new path,
    // so the rows land in a later frame; together the two miss the frame budget.
    var procNext = null;
    var mergeNext = function () {
      // A merge never lands under a pressed control, as for a held frame above.
      if (procHold) {
        window.setTimeout(mergeNext, 50);
        return;
      }
      mergeProc(procWrap.querySelector("tbody"), procNext);
      procNext = null;
      applyProc();
      syncNumbers();
      refreshTip();
    };
    procWrap.addEventListener("htmx:oobBeforeSwap", function (event) {
      var next = event.target === procWrap ? event.detail.fragment.querySelector("tbody") : null;
      if (!procWrap.querySelector("tbody") || !next) return;
      event.detail.shouldSwap = false;
      if (!procNext) window.setTimeout(mergeNext, 50);
      procNext = next;
    });
  }

  /* ---------- process popup (ACC-VIEW-176-PROC-POPUP) ---------- */

  // A click on a row opens that process's popup.
  // The server renders the body; this code places the box and asks for a new
  // body once per slow pass, and the sampler stays the only reader. The box
  // covers the table up to the Cpu% column, so it hides rows and no plot
  // (ACC-GRAPH-24-UNOBSCURED). The header and its close chip stay in the page,
  // so a new body never takes the focus; the command block is replaced only
  // when its text changes, so a selection in it survives.
  var detail = document.getElementById("proc-detail");
  var detailBody = document.getElementById("proc-detail-body");
  var detailState = document.getElementById("proc-detail-state");
  var detailEnded = document.getElementById("proc-detail-ended");
  var detailStale = document.getElementById("proc-detail-stale");
  var detailClose = document.getElementById("proc-detail-close");
  var detailPid = null;
  var detailBusy = false;

  function placeDetail() {
    if (detailPid === null) return;
    var box = procWrap.getBoundingClientRect();
    var cpu = procWrap.querySelectorAll("thead th")[6];
    var right = cpu && cpu.offsetWidth > 0 ? cpu.getBoundingClientRect().left : box.right;
    detail.style.left = box.left + "px";
    detail.style.top = box.top + "px";
    detail.style.width = right - box.left + "px";
    detail.style.height = box.height + "px";
  }

  // ACC-VIEW-209-POPUP-STALE: the page's stale state shows in the state slot;
  // Process Ended outranks it.
  function paintDetailStale() {
    if (detail) detailStale.hidden = !stale || !detailEnded.hidden;
  }

  // ACC-VIEW-204-PROC-ENDED: a 404 says the process ended; a request that failed
  // says nothing of the process, so it keeps the unavailable chip.
  function showDetail(html, ended) {
    // ACC-VIEW-209-POPUP-STALE: a request that fails with the stream gone takes no known end away.
    if (html === null && stale && !detailEnded.hidden) return;
    detailState.hidden = html !== null || ended;
    detailEnded.hidden = !ended;
    paintDetailStale();
    var live = detailBody.querySelector(".pd-live");
    if (html === null) {
      if (!live) detailBody.innerHTML = '<p class="unavailable">Process details unavailable</p>';
      else if (ended) detailBody.querySelector('[data-fact="state"]').textContent = "ended";
      syncMotion();
      return;
    }
    var fresh = document.createElement("template");
    fresh.innerHTML = html;
    var body = fresh.content.querySelector(".pd-live");
    notePass(Number(body.getAttribute("data-slow-seq")));
    if (!live) {
      detailBody.textContent = "";
      detailBody.appendChild(fresh.content);
    } else {
      // In place, as the panels are, so no new compositor layer comes each pass.
      patchAttributes(live, body);
      patchChildren(live, body);
      var command = detailBody.querySelector(".pd-cmd");
      var next = fresh.content.querySelector(".pd-cmd");
      if (command.textContent !== next.textContent) command.replaceWith(next);
    }
    syncRescale();
    syncMotion();
    refreshTip();
    syncNumbers();
  }

  // A body that does not arrive marks the popup and keeps the last figures; the
  // next slow pass asks again.
  function askDetail() {
    if (detailPid === null || detailBusy) return;
    var pid = detailPid;
    var ended = false;
    detailBusy = true;
    window.fetch("proc/" + pid + window.location.search, { cache: "no-store" })
      .then(function (response) {
        // ACC-VIEW-209-POPUP-STALE: with the stream gone a 404 may be a proxy's, which says nothing of the process.
        ended = response.status === 404 && !stale;
        return response.ok ? response.text() : null;
      })
      .catch(function () {
        return null;
      })
      .then(function (html) {
        detailBusy = false;
        if (pid === detailPid) showDetail(html, ended);
      });
  }

  function openDetail(p) {
    detailPid = p.pid;
    detailBusy = false;
    detailAnchor = passZeros.length ? Math.min.apply(null, passZeros) : null;
    document.getElementById("proc-detail-title").textContent = "PID " + p.pid + " " + p.texts[1];
    detailBody.textContent = "";
    detailState.hidden = true;
    detailEnded.hidden = true;
    paintDetailStale();
    detail.hidden = false;
    syncScrim();
    placeDetail();
    detailClose.focus();
    askDetail();
  }

  function closeDetail() {
    detailPid = null;
    detail.hidden = true;
    syncScrim();
    detailBody.textContent = "";
  }

  if (detail && procWrap) {
    document.addEventListener("click", function (event) {
      // A drag that selected text is not a click.
      if (!window.getSelection().isCollapsed) return;
      var row = closest(event.target, "#proc-body tr");
      if (row && row._p && row._p.pid !== detailPid && !closest(event.target, "button")) {
        openDetail(row._p);
        return;
      }
      if (detailPid === null || detail.contains(event.target)) return;
      closeDetail();
    });
    document.addEventListener("keydown", function (event) {
      if (detailPid === null) return;
      if (event.key === "Escape") {
        event.preventDefault();
        closeDetail();
      } else if (event.key === "Tab") {
        cycleTab(event, detail);
      }
    });
    detailClose.addEventListener("click", closeDetail);
    detailBody.addEventListener("click", function (event) {
      var copy = closest(event.target, ".pd-copy");
      if (!copy) return;
      writeClipboard(detailBody.querySelector(".pd-cmd").textContent)
        .then(function () {
          flashCopied(copy, "Command line copied", "copy");
        })
        .catch(function () {
          if (copyStatus) copyStatus.textContent = "Copy failed";
        });
    });
    window.addEventListener("resize", placeDetail);
  }

  /* ---------- process column menu (ACC-VIEW-140-COLUMN-MENU) ---------- */

  // The header row's right-click menu, after `</main>` so no swapped fragment
  // holds it. A click toggles one column and the menu stays open, so several
  // switch in one visit; Escape, a click outside and a second right-click close.
  var colMenu = document.getElementById("proc-col-menu");
  var colMenuReturn = null;

  function syncColMenu() {
    var items = colMenu.querySelectorAll("[data-col]");
    for (var index = 0; index < items.length; index += 1) {
      var on = !colHidden(items[index].getAttribute("data-col"));
      items[index].classList.toggle("on", on);
      items[index].setAttribute("aria-checked", String(on));
    }
  }

  function closeColMenu(refocus) {
    if (colMenu.hidden) return;
    colMenu.hidden = true;
    if (refocus && colMenuReturn && colMenuReturn.isConnected) colMenuReturn.focus();
    colMenuReturn = null;
  }

  function openColMenu(head, x, y) {
    syncColMenu();
    colMenuReturn = head.querySelector("button") || head;
    colMenu.hidden = false;
    colMenu.style.left = Math.max(0, Math.min(x, window.innerWidth - colMenu.offsetWidth)) + "px";
    colMenu.style.top = Math.max(0, Math.min(y, window.innerHeight - colMenu.offsetHeight)) + "px";
    var first = colMenu.querySelector('.opt.on:not([disabled])') || colMenu.querySelector("button:not([disabled])");
    if (first) first.focus();
  }

  if (colMenu && procWrap) {
    // The process header owns the right-click: its menu replaces the panel
    // menu's, so that handler must not also open.
    procWrap.addEventListener("contextmenu", function (event) {
      var head = closest(event.target, "thead th");
      if (!head) return;
      event.preventDefault();
      event.stopPropagation();
      if (!colMenu.hidden) closeColMenu(false);
      else openColMenu(head, event.clientX, event.clientY);
    });
    procWrap.addEventListener("keydown", function (event) {
      if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
      var head = closest(document.activeElement, "thead th");
      if (!head) return;
      event.preventDefault();
      var box = head.getBoundingClientRect();
      openColMenu(head, box.left + 8, box.top + 8);
    });
    colMenu.addEventListener("contextmenu", function (event) {
      event.preventDefault();
    });
    colMenu.addEventListener("click", function (event) {
      var item = closest(event.target, "[data-col]");
      if (!item || item.disabled) return;
      var key = item.getAttribute("data-col");
      if (colHidden(key)) procCols.splice(procCols.indexOf(key), 1);
      else procCols.push(key);
      try {
        window.localStorage.setItem(PROC_COL_KEY, JSON.stringify(procCols));
      } catch (error) {
        // Storage is optional; the choice holds for this page view.
      }
      syncColMenu();
      applyProcCols();
    });
    colMenu.addEventListener("keydown", function (event) {
      var items = Array.prototype.slice.call(colMenu.querySelectorAll('[role="menuitemcheckbox"]:not([disabled])'));
      var at = items.indexOf(document.activeElement);
      var to = { ArrowDown: at + 1, ArrowUp: at - 1, Home: 0, End: items.length - 1 }[event.key];
      if (to !== undefined) {
        items[(to + items.length) % items.length].focus();
      } else if (event.key === "Escape" || event.key === "Tab") {
        closeColMenu(true);
      } else {
        return;
      }
      event.preventDefault();
      event.stopPropagation(); // Escape here leaves an open CONFIG popover open.
    });
    document.addEventListener("pointerdown", function (event) {
      if (event.button === 0 && !colMenu.contains(event.target)) closeColMenu(false);
    });
    // Any second right-click closes it, on the header, on the menu or elsewhere.
    document.addEventListener("contextmenu", function (event) {
      if (!colMenu.hidden) closeColMenu(false);
    });
    window.addEventListener("resize", function () {
      closeColMenu(false);
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
    var chip = closest(event.target, '[data-group="proc-view"] .opt, [data-group="proc-level"] .opt, [data-group="proc-dir"] .opt');
    if (!chip) return;
    var value = chip.getAttribute("data-value");
    if (closest(chip, '[data-group="proc-view"]')) {
      procView.view = procView.chosen = value;
    } else if (closest(chip, '[data-group="proc-dir"]')) {
      procView.descending = value === "desc";
    } else {
      // The level sets every node; it clears the per-row overrides.
      procView.level = Number(value);
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

  if (procPanel && procWrap) {
    new ResizeObserver(function () {
      fitProc();
      placeDetail();
    }).observe(procPanel);
  }

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

  /* ---------- gauge text fit (ACC-VIEW-162-GAUGE-FIT) ---------- */

  // The gauge shape is chosen in app.css (DEF-LAYOUT-90-LOAD-SHIFT): the
  // fragment carries both the circle and the squircle and a container query on
  // the gauge body's height shows the one that fits, before the first paint.
  // Nothing here changes a shape or a gauge box; this code only fits the
  // glyphs inside the visible shape, so a bigger value, a wider face or a
  // narrow gauge never clips. The value box holds the design readout 23.8 GiB
  // in both faces and excludes 1234.5 GiB; the label box spans both label
  // baselines, 78 and the unavailable state's 70; a scale label takes the
  // 0.78 em by 1.175 em box chart.rs checks (GAUGE_SCALE_CHAR_EM) at its own
  // design size - 8 px for 0, 50 and 100, 5 px for the eight minor labels
  // (ACC-VIEW-173) - which bounds the drawn advance per character: the narrow
  // face reaches 0.758 em at 8 px at 1280x720 (0.737 at 1440x900, 0.703 at
  // 1024x768) and the system face 0.637 em. The design size is read from the
  // stylesheet before any step is written and the floors are fixed. A step
  // back up needs GAUGE_FIT_SLACK units of clear space inside the box: that
  // is the hysteresis, so a text that only just fits keeps its step instead
  // of oscillating with the frame's rounding.
  // ACC-VIEW-189-GAUGE-ROOM: the gauge draws --gauge-grow times its design box
  // and the readout's user units shrink by the same factor, so its boxes, sizes
  // and step divide by it too and each step renders at the px it did before.
  var GAUGE_GROW = 1.06;
  var GAUGE_VALUE_BOX = { x: 54 - 30 / GAUGE_GROW, y: 49, w: 60 / GAUGE_GROW, h: 23 };
  var GAUGE_LABEL_BOX = { x: 54 - 34 / GAUGE_GROW, y: 59, w: 68 / GAUGE_GROW, h: 23 };
  var GAUGE_SCALE_BOX = { char_em: 0.78, height_em: 1.175 };
  var GAUGE_FIT_FLOOR = { value: 10 / GAUGE_GROW, label: 7 / GAUGE_GROW, scale: 6, "scale-minor": 5 };
  var GAUGE_FIT_DESIGN = { value: 16 / GAUGE_GROW, label: 9 / GAUGE_GROW, scale: 8, "scale-minor": 5 };
  var GAUGE_FIT_STEP = { value: 1 / GAUGE_GROW, label: 1 / GAUGE_GROW, scale: 1, "scale-minor": 1 };
  var GAUGE_FIT_SLACK = 0.5;
  var GAUGE_FIT_EPS = 0.01;
  var gaugeDesign = {};
  var gaugeFit = {};

  // The stylesheet's design size, read once per text kind before any step is
  // written; a stepped size never becomes the design size. A kind first seen
  // in a hidden panel falls back to the CSS design size, and the rendered
  // check keeps a zero computed size out of it.
  function gaugeDesignSize(kind, node) {
    if (!gaugeDesign[kind]) {
      var size = node.getClientRects().length
        ? parseFloat(window.getComputedStyle(node).fontSize) : 0;
      gaugeDesign[kind] = size > 0 ? size : GAUGE_FIT_DESIGN[kind];
    }
    return gaugeDesign[kind];
  }

  function gaugeBBoxFits(bbox, box, slack) {
    return bbox.x >= box.x + slack - GAUGE_FIT_EPS &&
      bbox.y >= box.y + slack - GAUGE_FIT_EPS &&
      bbox.x + bbox.width <= box.x + box.w - slack + GAUGE_FIT_EPS &&
      bbox.y + bbox.height <= box.y + box.h - slack + GAUGE_FIT_EPS;
  }

  // The box and the step are cached per text key, with the text, the face and
  // the gauge's rendered width: an unchanged frame re-measures nothing, and a
  // resize (a new rendered width) or a font switch (a new face) re-measures.
  function fitGaugeText(node, key, box, kind, scale) {
    var design = gaugeDesignSize(kind, node);
    var floor = GAUGE_FIT_FLOOR[kind];
    var step = GAUGE_FIT_STEP[kind];
    var text = node.textContent;
    var font = document.documentElement.getAttribute("data-font") || "system";
    var sig = box.x + "," + box.y + "," + box.w + "," + box.h;
    var cached = gaugeFit[key];
    var size;
    if (cached && cached.text === text && cached.font === font &&
        cached.box === sig && cached.scale === scale) {
      size = cached.size;
    } else {
      var bboxAt = function (at) {
        node.style.fontSize = at + "px";
        return node.getBBox();
      };
      // Start from the step already chosen for the key, so a value that keeps
      // overflowing measures once at its step instead of walking down from the
      // design size every frame; a value that shrank climbs back with slack.
      size = Math.min(design, cached ? cached.size : design);
      var bbox = bboxAt(size);
      // A hidden panel has no rendered box: the design size is kept under the
      // zero rendered width, so it is fitted when the panel is shown.
      if (bbox.width === 0 && bbox.height === 0) {
        gaugeFit[key] = { text: text, font: font, box: sig, scale: scale, size: design };
        return;
      }
      while (size > floor + GAUGE_FIT_EPS && !gaugeBBoxFits(bbox, box, 0)) {
        size -= step;
        bbox = bboxAt(size);
      }
      while (size + step <= design + GAUGE_FIT_EPS &&
          gaugeBBoxFits(bboxAt(size + step), box, GAUGE_FIT_SLACK)) {
        size += step;
      }
      gaugeFit[key] = { text: text, font: font, box: sig, scale: scale, size: size };
    }
    if (node.style.fontSize !== size + "px") node.style.fontSize = size + "px";
  }

  // Both shapes carry the same readout in the same user units, so a step fits
  // both; the hidden one has no box to measure and is fitted when the
  // container query shows it.
  function fitGauge(gauge) {
    var svgs = gauge.querySelectorAll("svg");
    var name = gauge.getAttribute("data-gauge") || "gauge";
    for (var index = 0; index < svgs.length; index += 1) {
      var svg = svgs[index];
      if (!svg.getClientRects().length) continue;
      var key = name + ":" + index;
      var scale = Math.round(svg.getBoundingClientRect().width * 10) / 10;
      var value = svg.querySelector(".g-value");
      if (value) fitGaugeText(value, key + ":value", GAUGE_VALUE_BOX, "value", scale);
      var label = svg.querySelector(".g-label");
      if (label) fitGaugeText(label, key + ":label", GAUGE_LABEL_BOX, "label", scale);
      var scales = svg.querySelectorAll(".g-scale");
      for (var at = 0; at < scales.length; at += 1) {
        var scaleNode = scales[at];
        var kind = scaleNode.classList.contains("minor") ? "scale-minor" : "scale";
        var design = gaugeDesignSize(kind, scaleNode);
        var x = Number(scaleNode.getAttribute("x"));
        var y = Number(scaleNode.getAttribute("y"));
        var boxWidth = GAUGE_SCALE_BOX.char_em * design * scaleNode.textContent.length;
        var boxHeight = GAUGE_SCALE_BOX.height_em * design;
        fitGaugeText(scaleNode, key + ":" + kind + ":" + at,
          { x: x - boxWidth / 2, y: y - boxHeight / 2, w: boxWidth, h: boxHeight },
          kind, scale);
      }
    }
  }

  function fitGauges() {
    var gauges = document.querySelectorAll("[data-gauge]");
    for (var index = 0; index < gauges.length; index += 1) fitGauge(gauges[index]);
    // ACC-VIEW-231-ALL-NUM: the fit moves the paint's type and baseline, so
    // every shown overlay is placed again from it.
    for (index = 0; index < gauges.length; index += 1) gaugeShow(gauges[index]);
  }

  // A panel size, a viewport change or a shape switch moves the gauge without
  // an SSE frame; the observer re-fits the visible shape, and the fit is
  // idempotent. The gauge itself is observed too: a width-token change can
  // resize it while its body keeps its height, and the fit's rendered-width
  // key must see that.
  var gaugeBodies = document.querySelectorAll(".gauge-body");
  if (gaugeBodies.length && window.ResizeObserver) {
    var gaugeObserver = new ResizeObserver(fitGauges);
    for (var gaugeIndex = 0; gaugeIndex < gaugeBodies.length; gaugeIndex += 1) {
      gaugeObserver.observe(gaugeBodies[gaugeIndex]);
    }
    var gaugeBoxes = document.querySelectorAll("[data-gauge]");
    for (gaugeIndex = 0; gaugeIndex < gaugeBoxes.length; gaugeIndex += 1) {
      gaugeObserver.observe(gaugeBoxes[gaugeIndex]);
    }
  }

  // ACC-VIEW-162-GAUGE-FIT: a face that finishes loading, or the FONT control
  // changing the root attribute while paused, moves every text's metrics
  // without a frame. Fonts drop the cached steps; the attribute re-runs the
  // fit, whose font key then measures each text again.
  if (document.fonts && document.fonts.ready) {
    document.fonts.ready.then(function () {
      gaugeFit = {};
      fitGauges();
    });
  }
  if (window.MutationObserver) {
    new MutationObserver(fitGauges).observe(document.documentElement,
      { attributes: true, attributeFilter: ["data-font"] });
  }

  /* ---------- initial paint and re-apply after each frame ---------- */

  function afterFrames() {
    fitGauges();
    syncRescale();
    applyGpuSelection();
    applyProc();
    syncBands();
    syncMotion();
    syncNumbers();
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

  // ACC-PERF-133-PAGE-COST: a value tick feeds every panel its event names, and
  // replacing a panel's nodes makes the browser clean up, style, lay out, paint
  // and raster the whole subtree and build a new compositor layer for each chart,
  // for a handful of changed numbers. The frame is written into the nodes already
  // on the page, paired by place because a fragment keeps its shape between
  // frames; an equal node is not touched and a differing one is replaced.
  // Own code: htmx's morph extension would remove what the page adds to these
  // nodes, the chart's animation and the gauge text's fitted size.
  function patchAttributes(node, next) {
    var from = next.attributes;
    for (var index = 0; index < from.length; index += 1) {
      // applyGpuSelection owns a GPU's data-selected.
      if (from[index].name === "data-selected") continue;
      if (node.getAttribute(from[index].name) !== from[index].value) node.setAttribute(from[index].name, from[index].value);
    }
    var own = node.attributes;
    for (index = own.length - 1; index >= 0; index -= 1) {
      // fitGaugeText owns a gauge text's style and syncMotion a chart svg's.
      if (own[index].name === "style" && (node.nodeName === "text" || node.nodeName === "svg")) continue;
      if (!next.hasAttribute(own[index].name)) node.removeAttribute(own[index].name);
    }
  }

  // A number effect keeps its layers inside the value's node, and numAdopt
  // carries them across a replaced node, so while an effect can run a changed
  // value's node is replaced.
  var numberEffects = false;

  function changedValue(have, want) {
    return want.nodeType === 1 && want.hasAttribute("data-w") &&
      (have.hasAttribute("data-num-host") || have.textContent !== want.textContent);
  }

  function patchChildren(node, next) {
    var have = node.firstChild;
    var want = next.firstChild;
    while (want) {
      var after = want.nextSibling;
      if (have && have.nodeName === want.nodeName && !(numberEffects && changedValue(have, want))) {
        if (want.nodeType === 1) {
          patchAttributes(have, want);
          patchChildren(have, want);
        } else if (have.nodeValue !== want.nodeValue) {
          have.nodeValue = want.nodeValue;
        }
        have = have.nextSibling;
      } else {
        gaugeNew = true;
        var gone = have;
        have = have ? have.nextSibling : null;
        if (gone) node.replaceChild(want, gone);
        else node.appendChild(want);
      }
      want = after;
    }
    while (have) {
      var rest = have.nextSibling;
      node.removeChild(have);
      have = rest;
    }
  }

  // htmx hands an innerHTML swap the cloned wrapper element and an outerHTML
  // swap (the interval span) a DocumentFragment.
  document.body.addEventListener("htmx:oobBeforeSwap", function (event) {
    var target = event.detail.target;
    var source = event.detail.fragment;
    if (!event.detail.shouldSwap || !target || !source) return;
    // htmx edits the panel's own nodes, so every reference the applier cached
    // for them may now be stale.
    if (DATA_PANELS[target.id]) cacheGen += 1;
    numberEffects = numMode !== "none" || numStarted !== numCompleted;
    if (source.nodeType === 1) {
      patchChildren(target, source);
    } else {
      patchAttributes(target, source.firstElementChild);
      patchChildren(target, source.firstElementChild);
    }
    var charts = target.querySelectorAll("[data-chart] > .chart");
    for (var index = 0; index < charts.length; index += 1) charts[index]._seq = slowSeq;
    event.detail.shouldSwap = false;
  });
  document.body.addEventListener("htmx:afterSwap", syncNumbers);

  // A gauge text keeps its fitted size through a patch. Tabular figures give
  // every digit the advance of 0, so only new nodes or a new digit count or unit
  // need a fit, whose geometry reads force a layout.
  var gaugeNew = true;
  var gaugeForm = "";

  function fitChangedGauges() {
    var values = document.querySelectorAll("[data-gauge] .g-value");
    var form = "";
    for (var index = 0; index < values.length; index += 1) form += values[index].textContent.replace(/\d/g, "0") + "\n";
    if (!gaugeNew && form === gaugeForm) return;
    gaugeNew = false;
    gaugeForm = form;
    fitGauges();
  }

  // Fires after a live frame's out-of-band swaps have landed, before paint.
  document.body.addEventListener("htmx:sseMessage", function (event) {
    var type = event.detail.type;
    syncRescale();
    if (type === "cpu" || type === "mem") fitChangedGauges();
    if (type === "gpu") {
      applyGpuSelection();
      syncBandsIfChanged();
    } else if (type === "mem") {
      syncBandsIfChanged();
    }
    syncMotion();
    syncNumbers();
    refreshTip();
  });

  paintClock();
  paintLive();
  armStale();
  syncLayoutControls();
  syncProcControls();
  syncMotionControls();
  syncNumControls();
  gpuSelection = readGpuSelection(gpuBoxes().length);
  afterFrames();
})();

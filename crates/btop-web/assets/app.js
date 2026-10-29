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
  // hundreds of rows repeats no attribute. The box opens after the pointer has
  // rested TIP_WAIT_MS on one stat (DEF-VIEW-64), TIP_GAP right and TIP_DROP
  // below the pointer so it covers neither the cursor nor the stat's own text
  // (ACC-VIEW-111), flipped left or up only when it would leave the viewport
  // and kept TIP_PAD inside it (DEF-VIEW-75). A frame replaces the stat under
  // a still pointer, so the box re-reads the text the server now sends.
  var TIP_WAIT_MS = 3000; // pointer rest on one stat before the box opens
  var TIP_GAP = 12; // px from the pointer to the box's near side
  var TIP_DROP = 16; // px from the pointer to the box's top
  var TIP_PAD = 4; // px between the box and the viewport edge
  var tip = document.getElementById("tip");
  var tipAt = null; // the open box: { key, text }
  var tipWait = 0; // the pointer-rest timer
  var tipWant = null; // the stat the pointer rests on, not yet open
  var tipX = 0;
  var tipY = 0;

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

  // The line boxes of a stat's own text: the numbers their own place
  // protects, tall enough that no digit is clipped. A bar or a gauge with no
  // text falls back to its whole box.
  function textBox(anchor) {
    var left = Infinity;
    var top = Infinity;
    var right = -Infinity;
    var bottom = -Infinity;
    var nodes = anchor.querySelectorAll("*");
    var all = [anchor];
    for (var index = 0; index < nodes.length; index += 1) all.push(nodes[index]);
    for (index = 0; index < all.length; index += 1) {
      var hasText = false;
      for (var child = all[index].firstChild; child; child = child.nextSibling) {
        if (child.nodeType === 3 && child.nodeValue.trim()) hasText = true;
      }
      if (!hasText) continue;
      var box = all[index].getBoundingClientRect();
      if (box.width <= 0 || box.height <= 0) continue;
      left = Math.min(left, box.left);
      top = Math.min(top, box.top);
      right = Math.max(right, box.right);
      bottom = Math.max(bottom, box.bottom);
    }
    if (left === Infinity) return anchor.getBoundingClientRect();
    return { left: left, top: top, right: right, bottom: bottom };
  }

  // The box opens TIP_GAP right and TIP_DROP below the pointer. It flips to
  // the pointer's left or above it only when that side would leave the
  // viewport, and it stays TIP_PAD inside the viewport. Where the pointer sits
  // on the stat's own text, the box moves just below that text, or above it
  // when below is off screen, so the hovered value stays readable.
  function placeTip(anchor) {
    var width = tip.offsetWidth;
    var height = tip.offsetHeight;
    var vw = window.innerWidth;
    var vh = window.innerHeight;
    var x = tipX + TIP_GAP;
    if (x + width > vw - TIP_PAD) x = tipX - TIP_GAP - width;
    var y = tipY + TIP_DROP;
    if (y + height > vh - TIP_PAD) y = tipY - TIP_DROP - height;
    var text = textBox(anchor);
    if (x < text.right && x + width > text.left && y < text.bottom && y + height > text.top) {
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
    tip.textContent = source.text;
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
      if ((tipAt && tipAt.key === source.key) || (tipWant && tipWant.key === source.key)) return;
      hideTip();
      tipWant = source;
      tipWait = window.setTimeout(openRest, TIP_WAIT_MS);
    });
    document.addEventListener("pointerdown", hideTip);
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
    // ACC-VIEW-120: a stopped page shows the true value at once; nothing
    // queues while it is stopped.
    if (stale || paused) cancelNumberRuns();
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
  var motionFrame = 0;

  // The OS reduced-motion query is not consulted: the owner's machine sets it
  // on every page, so only the CONFIG motion toggle stops the clock.
  function motionAllowed() {
    return motionPref && windowMs > 0 && lastFrameAt > 0 && !paused && !stale && !document.hidden;
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
    if (!motionPref) cancelNumberRuns();
    syncMotionControls();
    syncMotion();
  });

  document.addEventListener("visibilitychange", syncMotion);

  /* ---------- number change effects (ACC-VIEW-108-NUM-MOTION) ---------- */

  // The setting is per browser, stored like the motion toggle. Effects run on
  // the [data-w] values only: each reserves its widest form, so a tween or an
  // overlay moves no neighbour (ACC-LAYOUT-74), and tabular figures give 0 the
  // advance and height of every other digit. Only the app's motion toggle
  // stops them; the OS reduced-motion query is not consulted, because the
  // owner's machine sets it on every page.
  var NUM_KEY = "btop-web-num-motion";
  var NUM_MODES = ["none", "spring", "blur", "odometer", "spring-blur", "segment"];
  // ACC-VIEW-120: one effect lasts at most four fifths of the current interval,
  // so it ends before the next frame replaces the value; NUM_MS caps it at the
  // 250 ms interval's own duration.
  var NUM_MS = 200;

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
  // One record per [data-w] in document order: the target text and the texts
  // the echoes may show. A frame replaces the nodes, so the order is the key.
  var numStates = [];
  var numRuns = {};

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
  // state. The new node's text is the newest reading; it is applied when the
  // effect ends, so a burst coalesces to the latest value. The host marker is
  // an attribute, not a class: htmx settles the class and style of a swapped
  // element back to the server's own, which would undo a class 20 ms in.
  function numAdopt(index, el) {
    var run = numRuns[index];
    if (!run || run.host.el === el) return;
    var old = run.host.el;
    old.removeAttribute("data-num-host");
    el.textContent = "";
    while (old.firstChild) el.appendChild(old.firstChild);
    el.setAttribute("data-num-host", "");
    run.host.el = el;
  }

  function numBegin(index, host, stop) {
    numRuns[index] = { host: host, stop: stop };
    numStarted += 1;
    numCount();
  }

  // End one effect: its node shows the newest reading, never a value that a
  // later frame replaced.
  function numFinish(index) {
    if (!numRuns[index]) return;
    delete numRuns[index];
    numCompleted += 1;
    numCount();
    var state = numStates[index];
    if (!state) return;
    state.el.textContent = state.text;
    state.el.removeAttribute("data-num-host");
  }

  // A spring count-up: the text is rewritten from an underdamped spring, so
  // the digits travel and overshoot briefly instead of swapping. k = 1600 and
  // c = 60 leave one short overshoot and settle well inside the effect's
  // duration. With trail, each intermediate value also leaves an echo behind.
  // The closed form, not a per-frame Euler step: a 60 Hz frame makes c*dt = 1,
  // the over-damped case, which hides the designed overshoot at that cadence.
  function numSpring(index, el, before, after, trail) {
    var host = { el: el };
    var ms = numDuration();
    var from = before.value;
    var to = after.value;
    var omega = Math.sqrt(1600);
    var zeta = 60 / (2 * omega);
    var omegaD = omega * Math.sqrt(1 - zeta * zeta);
    var value = from;
    var started = window.performance.now();
    var echoAt = 0;
    var echoes = [];
    var frame = 0;
    var tween = document.createElement("span");
    tween.className = "num-tween";
    host.el.textContent = "";
    host.el.appendChild(tween);
    host.el.setAttribute("data-num-host", "");

    function draw(number) {
      if (number < 0 && from >= 0 && to >= 0) number = 0;
      tween.textContent = numFormat(after, number);
    }

    function finish() {
      window.clearTimeout(timer);
      window.cancelAnimationFrame(frame);
      for (var echo = 0; echo < echoes.length; echo += 1) {
        echoes[echo].animation.cancel();
        echoes[echo].layer.remove();
      }
      numFinish(index);
    }

    function step(now) {
      var elapsed = now - started;
      // The frame timestamp, not the wall clock: an even display cadence.
      var t = elapsed / 1000;
      var decay = Math.exp(-zeta * omega * t);
      var swing = Math.cos(omegaD * t) + ((zeta * omega) / omegaD) * Math.sin(omegaD * t);
      value = to + decay * (from - to) * swing;
      if (elapsed >= ms) {
        finish();
        return;
      }
      draw(value);
      if (trail && elapsed - echoAt >= ms / 8) {
        echoAt = elapsed;
        echoes.push(numEcho(host.el, tween.textContent, 0.3, -6, ms - elapsed));
        if (echoes.length > NUM_ECHOES) {
          var oldest = echoes.shift();
          oldest.animation.cancel();
          oldest.layer.remove();
        }
      }
      frame = window.requestAnimationFrame(step);
    }

    draw(from);
    // The timer, not the display cadence, ends the effect inside the interval.
    var timer = window.setTimeout(finish, ms);
    numBegin(index, host, finish);
    frame = window.requestAnimationFrame(step);
  }

  // Motion blur: the last values sit behind the current one at falling
  // opacity and drift away. Only the current text is real; the layers are
  // aria-hidden and removed within the effect's duration.
  function numBlur(index, el, text, history) {
    var host = { el: el };
    var ms = numDuration();
    var opacity = [0.3, 0.17, 0.09];
    var echoes = [];
    var tween = document.createElement("span");
    tween.className = "num-tween";
    tween.textContent = text;
    host.el.textContent = "";
    host.el.appendChild(tween);
    host.el.setAttribute("data-num-host", "");
    for (var echo = 0; echo < history.length && echo < NUM_ECHOES; echo += 1) {
      echoes.push(numEcho(host.el, history[echo], opacity[echo], -6, ms));
    }

    function finish() {
      window.clearTimeout(timer);
      numFinish(index);
    }

    function stop() {
      for (var at = 0; at < echoes.length; at += 1) {
        echoes[at].animation.cancel();
        echoes[at].layer.remove();
      }
      finish();
    }

    var timer = window.setTimeout(finish, ms);
    numBegin(index, host, stop);
  }

  // Odometer: each changed digit rolls through a 0-9 strip, up or down with
  // the value. The cells are absolute and take their x from the rendered
  // text, so with tabular figures 0 keeps its place against every digit.
  // A length change moves every cell, so that update swaps instead of rolling.
  function numOdometer(index, el, oldText, newText, before, after) {
    var host = { el: el };
    var ms = numDuration();
    var line = numLine(host.el);
    var base = document.createElement("span");
    base.className = "num-base";
    base.textContent = newText;
    var row = document.createElement("span");
    row.className = "num-odo";
    row.setAttribute("aria-hidden", "true");
    row.style.lineHeight = line + "px";
    host.el.textContent = "";
    host.el.appendChild(base);
    host.el.appendChild(row);
    host.el.setAttribute("data-num-host", "");
    var node = base.firstChild;
    var box = host.el.getBoundingClientRect();
    var up = after.value >= before.value;
    var running = [];
    for (var at = 0; at < newText.length; at += 1) {
      var range = document.createRange();
      range.setStart(node, at);
      range.setEnd(node, at + 1);
      var cellBox = range.getBoundingClientRect();
      var cell = document.createElement("span");
      cell.className = "num-odo-cell";
      cell.style.left = cellBox.left - box.left + "px";
      cell.style.width = cellBox.width + "px";
      cell.style.height = line + "px";
      var oldGlyph = oldText.charAt(at);
      var glyph = newText.charAt(at);
      var rolls = oldText.length === newText.length && oldGlyph >= "0" && oldGlyph <= "9" &&
        glyph >= "0" && glyph <= "9" && oldGlyph !== glyph;
      if (rolls) {
        var strip = document.createElement("span");
        strip.className = "num-odo-strip";
        for (var slot = 0; slot < 20; slot += 1) {
          var digit = document.createElement("span");
          digit.textContent = String(slot % 10);
          strip.appendChild(digit);
        }
        var fromDigit = Number(oldGlyph);
        var toDigit = Number(glyph);
        var target = up ? (toDigit >= fromDigit ? toDigit : toDigit + 10) : toDigit;
        strip.style.transform = "translateY(" + -fromDigit * line + "px)";
        cell.appendChild(strip);
        running.push(strip.animate(
          [
            { transform: "translateY(" + -fromDigit * line + "px)" },
            { transform: "translateY(" + -target * line + "px)" }
          ],
          { duration: ms, easing: "cubic-bezier(.33,0,.2,1)", fill: "forwards" }
        ));
      } else {
        var fixed = document.createElement("span");
        fixed.className = "num-odo-static";
        fixed.textContent = glyph;
        cell.appendChild(fixed);
      }
      row.appendChild(cell);
    }

    function finish() {
      window.clearTimeout(timer);
      numFinish(index);
    }

    function stop() {
      for (var at = 0; at < running.length; at += 1) running[at].cancel();
      finish();
    }

    var timer = window.setTimeout(finish, ms);
    numBegin(index, host, stop);
  }

  // Segment flip: the digits are drawn as seven-segment boxes in their own
  // slots; only the segments that differ between the old and the new glyph
  // fade on or off, so a digit that keeps its glyph is untouched. Cells take
  // their x and width from the hidden text, so the tabular advance holds and
  // no neighbour moves; the prefix and suffix keep the text font.
  var NUM_SEGMENTS = {
    "0": "abcdef", "1": "bc", "2": "abdeg", "3": "abcdg", "4": "bcfg",
    "5": "acdfg", "6": "acdefg", "7": "abc", "8": "abcdefg", "9": "abcdfg"
  };
  // An unlit segment stays faintly visible: the shape reads as a display at
  // rest instead of a different set of glyphs.
  var NUM_SEG_OFF = 0.07;

  // Segment boxes for one cell: a, d and g horizontal, b, c, e and f
  // vertical. Thickness scales with the cell and stays at least 1 px.
  function numSegmentShape(width, height) {
    var pad = Math.max(1, Math.round(height * 0.08));
    var thick = Math.max(1, Math.round(Math.min(width, height) * 0.1));
    var across = Math.max(2 * thick + 1, width - 2 * pad);
    var span = Math.max(1, (height - 2 * pad - 3 * thick) / 2);
    var middle = pad + (height - 2 * pad - thick) / 2;
    var lower = pad + thick + span + thick;
    return {
      a: [pad + thick, pad, across - 2 * thick, thick],
      g: [pad + thick, middle, across - 2 * thick, thick],
      d: [pad + thick, pad + height - 2 * pad - thick, across - 2 * thick, thick],
      f: [pad, pad + thick, thick, span],
      b: [width - pad - thick, pad + thick, thick, span],
      e: [pad, lower, thick, span],
      c: [width - pad - thick, lower, thick, span]
    };
  }

  function numSegment(index, el, oldText, newText, before, after) {
    var host = { el: el };
    var ms = numDuration();
    var base = document.createElement("span");
    base.className = "num-base";
    base.textContent = newText;
    var row = document.createElement("span");
    row.className = "num-seg-row";
    row.setAttribute("aria-hidden", "true");
    host.el.textContent = "";
    host.el.appendChild(base);
    host.el.appendChild(row);
    host.el.setAttribute("data-num-host", "");
    var node = base.firstChild;
    var box = host.el.getBoundingClientRect();
    var offset = after.pre.length;
    var oldDigits = oldText.slice(before.pre.length, oldText.length - before.post.length);
    var newDigits = newText.slice(offset, newText.length - after.post.length);
    var running = [];
    for (var at = 0; at < newDigits.length; at += 1) {
      var range = document.createRange();
      range.setStart(node, offset + at);
      range.setEnd(node, offset + at + 1);
      var glyphBox = range.getBoundingClientRect();
      var width = glyphBox.width;
      var height = glyphBox.height || numLine(host.el);
      var glyph = newDigits.charAt(at);
      var was = oldDigits.charAt(at);
      var cell = document.createElement("span");
      cell.className = "num-seg-cell";
      cell.style.left = glyphBox.left - box.left + "px";
      cell.style.top = glyphBox.top - box.top + "px";
      cell.style.width = width + "px";
      cell.style.height = height + "px";
      row.appendChild(cell);
      if (glyph === ".") {
        var radius = Math.max(1, Math.round(height * 0.06));
        var dot = document.createElement("span");
        dot.className = "num-seg-dot";
        dot.style.left = (width - 2 * radius) / 2 + "px";
        dot.style.top = height - radius * 3 + "px";
        dot.style.width = 2 * radius + "px";
        dot.style.height = 2 * radius + "px";
        if (was === ".") {
          dot.style.opacity = "1";
        } else {
          dot.style.opacity = "0";
          running.push(dot.animate(
            [{ opacity: 0 }, { opacity: 1 }],
            { duration: ms, easing: "ease-out", fill: "forwards" }
          ));
        }
        cell.appendChild(dot);
        continue;
      }
      var shape = numSegmentShape(width, height);
      var wasLit = NUM_SEGMENTS[was] || "";
      var isLit = NUM_SEGMENTS[glyph] || "";
      for (var key = 0; key < "abcdefg".length; key += 1) {
        var name = "abcdefg".charAt(key);
        var part = shape[name];
        var segment = document.createElement("span");
        segment.className = "num-seg";
        segment.style.left = part[0] + "px";
        segment.style.top = part[1] + "px";
        segment.style.width = part[2] + "px";
        segment.style.height = part[3] + "px";
        var from = wasLit.indexOf(name) === -1 ? NUM_SEG_OFF : 1;
        var to = isLit.indexOf(name) === -1 ? NUM_SEG_OFF : 1;
        segment.style.opacity = String(to);
        cell.appendChild(segment);
        if (from !== to) {
          running.push(segment.animate(
            [{ opacity: from }, { opacity: to }],
            { duration: ms, easing: "ease-out", fill: "forwards" }
          ));
        }
      }
    }

    function finish() {
      window.clearTimeout(timer);
      numFinish(index);
    }

    function stop() {
      for (var at = 0; at < running.length; at += 1) running[at].cancel();
      finish();
    }

    var timer = window.setTimeout(finish, ms);
    numBegin(index, host, stop);
  }

  function playNumber(index, el, oldText, newText, history) {
    if (numMode === "none" || !motionPref || document.hidden) return;
    if (!el.getClientRects().length) return;
    var before = numParts(oldText);
    var after = numParts(newText);
    if (!before || !after || before.value === after.value) return;
    // A changed prefix, suffix or number of decimals is a unit or scale
    // change; a tween between different forms would read as invented digits.
    if (before.pre !== after.pre || before.post !== after.post || before.decimals !== after.decimals) return;
    if (numMode === "odometer") {
      if (oldText.length === newText.length) numOdometer(index, el, oldText, newText, before, after);
    } else if (numMode === "blur") {
      numBlur(index, el, newText, history);
    } else if (numMode === "segment") {
      numSegment(index, el, oldText, newText, before, after);
    } else {
      numSpring(index, el, before, after, numMode === "spring-blur");
    }
  }

  function cancelNumberRuns() {
    var keys = Object.keys(numRuns);
    for (var index = 0; index < keys.length; index += 1) numRuns[keys[index]].stop();
    numRuns = {};
  }

  // After each frame, pair the values by document order. A change starts an
  // effect; the same text does nothing, so a replaced node with an unchanged
  // value stays still and a hidden tab records the truth without replaying.
  function syncNumbers() {
    var hosts = document.querySelectorAll("[data-w]");
    if (hosts.length !== numStates.length) {
      // A row, lane or panel appeared or went: pair nothing this frame.
      cancelNumberRuns();
      numStates = [];
      for (var fresh = 0; fresh < hosts.length; fresh += 1) {
        numStates.push({ el: hosts[fresh], text: hosts[fresh].textContent, history: [] });
      }
      return;
    }
    var states = [];
    for (var index = 0; index < hosts.length; index += 1) {
      var el = hosts[index];
      var text = el.textContent;
      var previous = numStates[index];
      if (numRuns[index] && previous) {
        if (previous.el === el) {
          // The node was not replaced, so its text is the effect's own paint,
          // not a reading.
          states.push(previous);
          continue;
        }
        // A frame arrived mid-effect: carry the effect into the new node and
        // hold this reading until it ends. The state keeps the newest value,
        // so a burst coalesces to it; nothing else is queued.
        states.push({ el: el, text: text, history: previous.history });
        numAdopt(index, el);
        continue;
      }
      var state = { el: el, text: text, history: previous ? previous.history : [] };
      if (previous && previous.text !== text) {
        state.history = [previous.text].concat(previous.history).slice(0, NUM_ECHOES);
        playNumber(index, el, previous.text, text, state.history);
      }
      states.push(state);
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
    if (numMode === "none") cancelNumberRuns();
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

  function closeConfig(returnTo) {
    if (!config || config.hidden) return;
    config.hidden = true;
    if (scrim) scrim.hidden = true;
    setExpanded(false);
    if (!returnTo) return;
    var trigger = returnTo.isConnected ? returnTo : visibleToggle();
    if (trigger) trigger.focus();
  }

  function openConfig(trigger) {
    if (!config) return;
    lastTrigger = trigger;
    config.hidden = false;
    if (scrim) scrim.hidden = false;
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
      if (event.key !== "Tab") return;
      // ACC-VIEW-104-CONFIG-DIM: Tab and Shift+Tab cycle inside the open menu.
      var items = [];
      var nodes = config.querySelectorAll("button, select, input, textarea, a[href], [tabindex]");
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
      var outside = !config.contains(active);
      if (event.shiftKey && (outside || active === first)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (outside || active === last)) {
        event.preventDefault();
        first.focus();
      }
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
    var mark = p.collapsed ? "\u25b8\u2002" : "\u25be\u2002";
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
        // ACC-VIEW-103-TREE-DEPTH: a node at or below the display level starts
        // folded; a triangle click overrides that for its pid until the level
        // changes. A query opens the ancestors of a match without touching the
        // stored set.
        var stored = Object.prototype.hasOwnProperty.call(procView.toggled, p.pid) ? procView.toggled[p.pid] : p.depth >= procView.level;
        p.collapsed = parent && stored && !(query && p.under);
        p.shown = false;
      }
      if (view !== "tree" && p.shown) order.push(p);
    });
    if (view === "tree") {
      // btop's prefix (`btop_shared.cpp:188`, `btop_collect.cpp:2754`): one
      // three-character slot per ancestor level, ' \u2502 ' while that ancestor has
      // a following sibling and '   ' under the last one, then a leaf's
      // ' \u251c\u2500 ', ' \u2514\u2500 ' or the first root's ' \u250c\u2500 '. A node with children puts
      // its triangle in the branch slot instead, followed by U+2002.
      var walk = function (siblings, bars) {
        var shown = sortProc(siblings).filter(function (p) {
          return p.match || p.under;
        });
        var prefix = bars.map(function (more) {
          return more ? " \u2502 " : "   ";
        }).join("");
        shown.forEach(function (p, at) {
          var last = at === shown.length - 1;
          p.shown = true;
          order.push(p);
          glyphs[p.pid] = prefix + (p.kids.length > 0 ? " " : last ? " \u2514\u2500 " : p.depth === 0 && at === 0 ? " \u250c\u2500 " : " \u251c\u2500 ");
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

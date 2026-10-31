// Theme, mode and panel layout bootstrap. Runs from <head> so the chosen
// palette and layout apply before the body paints. Storage access is guarded:
// a page that cannot read localStorage still follows the system colour scheme
// and shows the default layout.
(function () {
  "use strict";

  var MODE_KEY = "btop-web-mode";
  var THEME_KEY = "btop-web-theme";
  var FONT_KEY = "btop-web-font";
  var LIVE_STATUS_KEY = "btop-web-live-status";
  var root = document.documentElement;

  function read(key) {
    try {
      return window.localStorage.getItem(key);
    } catch (error) {
      return null;
    }
  }

  function write(key, value) {
    try {
      window.localStorage.setItem(key, value);
    } catch (error) {
      // Storage is optional; the page works without it.
    }
  }

  function systemMode() {
    if (window.matchMedia && window.matchMedia("(prefers-color-scheme: light)").matches) {
      return "light";
    }
    return "dark";
  }

  // ACC-THEME-69-MODE-AUTO: the stored choice is dark, light or auto; no
  // stored or an unknown value is auto, which follows the system scheme.
  function modeFor(value) {
    return value === "auto" ? systemMode() : value;
  }

  var theme = read(THEME_KEY) || root.getAttribute("data-theme") || "default";
  // ACC-THEME-155-FONT-TOGGLE: the Default GalaxaTron pair is gone. A browser
  // that stored its id falls back to Default before the first paint, and the
  // picker check in ready() covers any other id this build does not ship.
  if (theme === "default-galaxatron") {
    theme = "default";
    write(THEME_KEY, theme);
  }
  var choice = read(MODE_KEY);
  if (choice !== "dark" && choice !== "light") {
    choice = "auto";
  }
  var mode = modeFor(choice);
  // ACC-THEME-155-FONT-TOGGLE: the font is its own per-browser choice; System
  // is the default and anything unrecognised falls back to it.
  var font = read(FONT_KEY);
  if (font !== "galaxatron") {
    font = "system";
  }
  // ACC-VIEW-200-LIVE-STATUS: on, dimmed or off, set here in the head so an off
  // status never paints; on is the default and anything unrecognised is on.
  var liveStatus = read(LIVE_STATUS_KEY);
  if (liveStatus !== "dimmed" && liveStatus !== "off") {
    liveStatus = "on";
  }

  root.setAttribute("data-theme", theme);
  root.setAttribute("data-mode", mode);
  root.setAttribute("data-font", font);
  root.setAttribute("data-live-status", liveStatus);

  // DEF-LAYOUT-90-LOAD-SHIFT: the stored face is prefetched from the head, so
  // the swap in app.css finds it before the first paint and no text re-lays out
  // after it. Only the GalaxaTron choice requests the file; the system stack
  // needs none.
  if (font === "galaxatron") {
    var preload = document.createElement("link");
    preload.rel = "preload";
    preload.as = "font";
    preload.type = "font/ttf";
    preload.crossOrigin = "anonymous";
    preload.href = "assets/fonts/GalaxaLabsTronNarrow-VF.ttf";
    document.head.appendChild(preload);
  }

  // ACC-VIEW-40-GADGETS: hidden panels, panel sizes and panel order, stored
  // beside the theme as
  // {"v":1,"off":[ids],"size":{id:"s"|"l"},"order":[ids in page order]}. The
  // page's data-panel ids and the server's PANELS registry match this map; a
  // panel's band follows its place in order, three panels in bands 1-3 and one
  // in band 4. ACC-LAYOUT-157 sets the default order, ACC-LAYOUT-158 stores a
  // drag swap in it, and ACC-LAYOUT-159 clears it with the rest.
  var LAYOUT_KEY = "btop-web-layout";
  var PANELS = {
    "cpu-stat": 1,
    "mem-stat": 1,
    "proc-stat": 1,
    cores: 2,
    "cpu-gauge": 2,
    "mem-gauge": 2,
    gpu: 3,
    net: 3,
    storage: 3,
    proc: 4
  };
  var DEFAULT_ORDER = [
    "cpu-stat",
    "mem-stat",
    "proc-stat",
    "cores",
    "cpu-gauge",
    "mem-gauge",
    "gpu",
    "net",
    "storage",
    "proc"
  ];
  var BAND_SIZES = [3, 3, 3, 1];
  var FACTORS = { s: 0.75, m: 1, l: 1.5 };

  function defaultLayout() {
    return { v: 1, off: [], size: {}, order: DEFAULT_ORDER.slice() };
  }

  function readOrder(stored) {
    var order = [];
    if (Array.isArray(stored)) {
      stored.forEach(function (id) {
        if (Object.prototype.hasOwnProperty.call(PANELS, id) && order.indexOf(id) === -1) {
          order.push(id);
        }
      });
    }
    DEFAULT_ORDER.forEach(function (id) {
      if (order.indexOf(id) === -1) order.push(id);
    });
    return order;
  }

  function readLayout() {
    try {
      var stored = JSON.parse(read(LAYOUT_KEY));
      if (!stored || stored.v !== 1) return defaultLayout();
      var layout = defaultLayout();
      if (Array.isArray(stored.off)) {
        layout.off = stored.off.filter(function (id) {
          return Object.prototype.hasOwnProperty.call(PANELS, id);
        });
      }
      if (stored.size && typeof stored.size === "object") {
        Object.keys(stored.size).forEach(function (id) {
          if (Object.prototype.hasOwnProperty.call(PANELS, id) && (stored.size[id] === "s" || stored.size[id] === "l")) {
            layout.size[id] = stored.size[id];
          }
        });
      }
      layout.order = readOrder(stored.order);
      return layout;
    } catch (error) {
      return defaultLayout();
    }
  }

  function writeLayout(layout) {
    write(LAYOUT_KEY, JSON.stringify(layout));
  }

  function bandOf(id, order) {
    var index = order.indexOf(id);
    if (index === -1) return PANELS[id];
    var seen = 0;
    for (var band = 0; band < BAND_SIZES.length; band += 1) {
      seen += BAND_SIZES[band];
      if (index < seen) return band + 1;
    }
    return PANELS[id];
  }

  // ACC-LAYOUT-158: move each panel into its slot. The slot carries the width
  // weight (app.css), so a panel keeps its content, size factor and controls
  // and takes the width of the place it lands in. Runs only once the body is
  // parsed; theme.js also runs from <head>, where there are no bands yet.
  function applyOrder(order) {
    var bands = document.querySelectorAll(".band");
    if (bands.length === 0) return;
    var counts = [];
    var index;
    for (index = 0; index < bands.length; index += 1) counts.push(bands[index].children.length);
    var position = 0;
    for (index = 0; index < bands.length; index += 1) {
      for (var slot = 0; slot < counts[index] && position < order.length; slot += 1) {
        var panel = document.querySelector('[data-panel="' + order[position] + '"]');
        position += 1;
        if (!panel) continue;
        if (panel.getAttribute("data-slot") !== String(slot + 1)) {
          panel.setAttribute("data-slot", String(slot + 1));
        }
        panel.setAttribute("data-band", String(index + 1));
        if (bands[index].children[slot] !== panel) {
          bands[index].insertBefore(panel, bands[index].children[slot] || null);
        }
      }
    }
  }

  // A band's height factor is the largest size factor among its shown panels.
  function applyLayout(layout) {
    var sizes = [];
    var bands = { 1: 0, 2: 0, 3: 0, 4: 0 };
    var order = readOrder(layout.order);
    Object.keys(PANELS).forEach(function (id) {
      var band = bandOf(id, order);
      var size = layout.size[id] || "m";
      if (size !== "m") sizes.push(id + ":" + size);
      if (layout.off.indexOf(id) === -1) bands[band] = Math.max(bands[band], FACTORS[size]);
    });
    root.setAttribute("data-off", layout.off.join(" "));
    root.setAttribute("data-size", sizes.join(" "));
    Object.keys(bands).forEach(function (band) {
      root.style.setProperty("--bf-" + band, String(bands[band] || 1));
    });
    applyOrder(order);
  }

  function resetLayout() {
    try {
      window.localStorage.removeItem(LAYOUT_KEY);
    } catch (error) {
      // Storage is optional; the default applies to this page view.
    }
    var layout = defaultLayout();
    applyLayout(layout);
    return layout;
  }

  var initialLayout = readLayout();
  applyLayout(initialLayout);
  window.btopLayout = {
    panels: PANELS,
    read: readLayout,
    write: writeLayout,
    apply: applyLayout,
    reset: resetLayout
  };

  function sync() {
    var picker = document.getElementById("theme-picker");
    if (picker) {
      picker.value = theme;
    }
    var buttons = document.querySelectorAll('[data-group="mode"] .opt');
    for (var index = 0; index < buttons.length; index += 1) {
      buttons[index].classList.toggle("on", buttons[index].getAttribute("data-value") === choice);
    }
    var fontButtons = document.querySelectorAll('[data-group="font"] .opt');
    for (var fontIndex = 0; fontIndex < fontButtons.length; fontIndex += 1) {
      var fontOn = fontButtons[fontIndex].getAttribute("data-value") === font;
      fontButtons[fontIndex].classList.toggle("on", fontOn);
      fontButtons[fontIndex].setAttribute("aria-checked", String(fontOn));
    }
    var liveButtons = document.querySelectorAll('[data-group="live-status"] .opt');
    for (var liveIndex = 0; liveIndex < liveButtons.length; liveIndex += 1) {
      var liveOn = liveButtons[liveIndex].getAttribute("data-value") === liveStatus;
      liveButtons[liveIndex].classList.toggle("on", liveOn);
      liveButtons[liveIndex].setAttribute("aria-checked", String(liveOn));
    }
  }

  function ready() {
    // ACC-LAYOUT-158: the stored arrangement lands before the first frame.
    applyOrder(initialLayout.order);
    var picker = document.getElementById("theme-picker");
    if (picker) {
      var known = false;
      for (var index = 0; index < picker.options.length; index += 1) {
        if (picker.options[index].value === theme) {
          known = true;
          break;
        }
      }
      if (!known && picker.options.length > 0) {
        theme = picker.options[0].value;
        root.setAttribute("data-theme", theme);
        write(THEME_KEY, theme);
      }
      picker.value = theme;
      picker.addEventListener("change", function () {
        theme = picker.value;
        root.setAttribute("data-theme", theme);
        write(THEME_KEY, theme);
      });
    }

    var modeGroup = document.querySelector('[data-group="mode"]');
    if (modeGroup) {
      modeGroup.addEventListener("click", function (event) {
        var button = event.target.closest(".opt");
        if (!button || !modeGroup.contains(button)) {
          return;
        }
        choice = button.getAttribute("data-value");
        mode = modeFor(choice);
        root.setAttribute("data-mode", mode);
        write(MODE_KEY, choice);
        sync();
      });
    }

    // ACC-THEME-155-FONT-TOGGLE: the FONT chips set the root attribute the
    // display-face token is keyed on and store the choice for this browser.
    var fontGroup = document.querySelector('[data-group="font"]');
    if (fontGroup) {
      fontGroup.addEventListener("click", function (event) {
        var button = event.target.closest(".opt");
        if (!button || !fontGroup.contains(button)) {
          return;
        }
        font = button.getAttribute("data-value");
        root.setAttribute("data-font", font);
        write(FONT_KEY, font);
        sync();
      });
    }

    var liveGroup = document.querySelector('[data-group="live-status"]');
    if (liveGroup) {
      liveGroup.addEventListener("click", function (event) {
        var button = event.target.closest(".opt");
        if (!button || !liveGroup.contains(button)) {
          return;
        }
        liveStatus = button.getAttribute("data-value");
        root.setAttribute("data-live-status", liveStatus);
        write(LIVE_STATUS_KEY, liveStatus);
        sync();
      });
    }

    if (window.matchMedia) {
      var media = window.matchMedia("(prefers-color-scheme: light)");
      var follow = function () {
        if (choice !== "auto") {
          return;
        }
        mode = media.matches ? "light" : "dark";
        root.setAttribute("data-mode", mode);
        sync();
      };
      if (media.addEventListener) {
        media.addEventListener("change", follow);
      }
    }

    sync();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", ready);
  } else {
    ready();
  }
})();

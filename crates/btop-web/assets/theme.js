// Theme, mode and panel layout bootstrap. Runs from <head> so the chosen
// palette and layout apply before the body paints. Storage access is guarded:
// a page that cannot read localStorage still follows the system colour scheme
// and shows the default layout.
(function () {
  "use strict";

  var MODE_KEY = "btop-web-mode";
  var THEME_KEY = "btop-web-theme";
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
  var choice = read(MODE_KEY);
  if (choice !== "dark" && choice !== "light") {
    choice = "auto";
  }
  var mode = modeFor(choice);

  root.setAttribute("data-theme", theme);
  root.setAttribute("data-mode", mode);

  // ACC-VIEW-40-GADGETS: hidden panels and panel sizes, stored beside the
  // theme as {"v":1,"off":[ids],"size":{id:"s"|"l"}}. Each id maps to its band;
  // the page's data-panel ids and the server's PANELS registry match this map.
  var LAYOUT_KEY = "btop-web-layout";
  var PANELS = {
    "cpu-stat": 1,
    "mem-stat": 1,
    "proc-stat": 1,
    net: 2,
    "cpu-gauge": 2,
    "mem-gauge": 2,
    cores: 3,
    gpu: 3,
    storage: 3,
    proc: 4
  };
  var FACTORS = { s: 0.75, m: 1, l: 1.5 };

  function defaultLayout() {
    return { v: 1, off: [], size: {} };
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
      return layout;
    } catch (error) {
      return defaultLayout();
    }
  }

  function writeLayout(layout) {
    write(LAYOUT_KEY, JSON.stringify(layout));
  }

  // A band's height factor is the largest size factor among its shown panels.
  function applyLayout(layout) {
    var sizes = [];
    var bands = { 1: 0, 2: 0, 3: 0, 4: 0 };
    Object.keys(PANELS).forEach(function (id) {
      var size = layout.size[id] || "m";
      if (size !== "m") sizes.push(id + ":" + size);
      if (layout.off.indexOf(id) === -1) bands[PANELS[id]] = Math.max(bands[PANELS[id]], FACTORS[size]);
    });
    root.setAttribute("data-off", layout.off.join(" "));
    root.setAttribute("data-size", sizes.join(" "));
    Object.keys(bands).forEach(function (band) {
      root.style.setProperty("--bf-" + band, String(bands[band] || 1));
    });
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

  applyLayout(readLayout());
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
  }

  function ready() {
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

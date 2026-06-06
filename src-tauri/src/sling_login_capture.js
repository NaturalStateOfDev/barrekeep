// Injected before any Sling JS runs. Two responsibilities:
//
//  1. AUTOFILL — if `window.__BK_CREDS` was set by a Rust preamble,
//     fill Sling's login form with the saved email + password as soon
//     as those inputs appear. Captcha and the submit click are left
//     to the user (lowest chance of tripping Sling's bot heuristics).
//
//  2. CAPTURE — monkey-patch fetch + XHR and, when the page makes an
//     authenticated request to api.getsling.com, trigger a same-origin
//     navigation to a sentinel URL that the Rust on_navigation hook
//     intercepts.
//
// Why a navigation rather than a Tauri event emit: Tauri 2 does not
// expose the IPC bridge on external/remote URLs by default.
//
// Filter: request URL host == api.getsling.com, path starts with /v1/,
// Authorization header is non-empty and >= 20 chars.

(() => {
  // -------- 1. AUTOFILL --------
  const creds = (window.__BK_CREDS && typeof window.__BK_CREDS === "object")
    ? window.__BK_CREDS
    : null;

  if (creds && (creds.email || creds.password)) {
    const EMAIL_SELECTORS = [
      'input[type="email"]',
      'input[name="email"]',
      'input[autocomplete="username"]',
      'input[autocomplete="email"]',
      'input[name="username"]',
      'input[id*="email" i]',
    ];
    const PASS_SELECTORS = [
      'input[type="password"]',
      'input[name="password"]',
      'input[autocomplete="current-password"]',
      'input[id*="password" i]',
    ];

    function findInput(selectors) {
      for (const sel of selectors) {
        const el = document.querySelector(sel);
        if (el && !el.disabled && !el.readOnly) return el;
      }
      return null;
    }

    // Setting .value alone doesn't trigger React/Vue controlled-input
    // updates; we have to invoke the native setter then fire input+change.
    function setReactInputValue(el, value) {
      const proto = Object.getPrototypeOf(el);
      const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
      if (setter) setter.call(el, value);
      else el.value = value;
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    }

    let filledEmail = !creds.email;
    let filledPass = !creds.password;

    function tryFill() {
      if (filledEmail && filledPass) return true;
      if (!filledEmail) {
        const el = findInput(EMAIL_SELECTORS);
        if (el) {
          setReactInputValue(el, String(creds.email));
          filledEmail = true;
        }
      }
      if (!filledPass) {
        const el = findInput(PASS_SELECTORS);
        if (el) {
          setReactInputValue(el, String(creds.password));
          filledPass = true;
        }
      }
      return filledEmail && filledPass;
    }

    if (!tryFill()) {
      const obs = new MutationObserver(() => {
        if (tryFill()) obs.disconnect();
      });
      // Start observing once a body exists.
      const start = () => obs.observe(document.documentElement, {
        childList: true, subtree: true,
      });
      if (document.documentElement) start();
      else document.addEventListener("DOMContentLoaded", start, { once: true });
      // Stop observing after 30s regardless.
      setTimeout(() => obs.disconnect(), 30000);
    }
  }

  // -------- 2. CAPTURE --------
  let captured = false;
  const AUTH_HOST = "api.getsling.com";
  const AUTH_PATH = "/v1/";
  const MIN_LEN = 20;
  const CAPTURE_URL = "https://app.getsling.com/__bk_capture";

  function tryCapture(url, authHeader) {
    if (captured) return;
    try {
      const u = new URL(url, window.location.origin);
      if (u.host !== AUTH_HOST) return;
      if (!u.pathname.startsWith(AUTH_PATH)) return;
      if (!authHeader || String(authHeader).length < MIN_LEN) return;
      captured = true;
      let target = CAPTURE_URL + "?t=" + encodeURIComponent(String(authHeader));
      // Opportunistically grab the org id from an org-scoped /v1/{org}/… URL
      // so the app can prefill studio config. Absent on non-org endpoints.
      const orgMatch = u.pathname.match(/^\/v1\/(\d+)(?:\/|$)/);
      if (orgMatch) target += "&o=" + orgMatch[1];
      window.location.replace(target);
    } catch (_) { /* swallow */ }
  }

  // Patch fetch
  const _fetch = window.fetch;
  window.fetch = function (input, init) {
    const url = typeof input === "string" ? input : (input && input.url) || "";
    let auth = null;
    if (init && init.headers) {
      const h = init.headers;
      if (typeof h.get === "function") auth = h.get("Authorization") || h.get("authorization");
      else auth = h.Authorization || h.authorization || null;
    } else if (input && typeof input !== "string" && input.headers) {
      const h = input.headers;
      if (typeof h.get === "function") auth = h.get("Authorization") || h.get("authorization");
    }
    if (auth) tryCapture(url, auth);
    return _fetch.apply(this, arguments);
  };

  // Patch XHR
  const _open = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (method, url) {
    this.__bk_url = url;
    return _open.apply(this, arguments);
  };
  const _setHeader = XMLHttpRequest.prototype.setRequestHeader;
  XMLHttpRequest.prototype.setRequestHeader = function (name, value) {
    if (String(name).toLowerCase() === "authorization") {
      tryCapture(this.__bk_url || window.location.href, value);
    }
    return _setHeader.call(this, name, value);
  };

  // 90s idle banner
  setTimeout(() => {
    if (captured) return;
    try {
      const b = document.createElement("div");
      b.style.cssText =
        "position:fixed;top:0;left:0;right:0;background:hsl(36 60% 92%);" +
        "color:hsl(36 70% 26%);padding:0.5rem 0.75rem;" +
        "font:13px/1.4 sans-serif;border-bottom:1px solid hsl(36 60% 70%);" +
        "z-index:99999";
      b.textContent =
        "Still waiting for sign-in. If you're already signed in, " +
        "try clicking around the calendar or refresh once.";
      if (document.body) document.body.appendChild(b);
    } catch (_) { /* swallow */ }
  }, 90000);
})();

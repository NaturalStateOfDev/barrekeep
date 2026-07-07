// Tests for src-tauri/src/sling_login_capture.js — the script injected into
// the Sling login webview. Loaded here as text and run against faked
// window/document/XHR globals (vitest runs in node, no jsdom needed).
import src from "../../src-tauri/src/sling_login_capture.js?raw";
import { describe, expect, it, vi } from "vitest";

const TOKEN = "0123456789abcdef0123456789abcdef"; // >= 20 chars

class FakeXHR {
  __listeners: Record<string, Array<() => void>> = {};
  __responseHeaders: Record<string, string> = {};
  addEventListener(ev: string, fn: () => void) {
    (this.__listeners[ev] ??= []).push(fn);
  }
  open(_method: string, _url: string) {}
  setRequestHeader(_name: string, _value: string) {}
  getResponseHeader(name: string): string | null {
    return this.__responseHeaders[name.toLowerCase()] ?? null;
  }
  send() {}
  fireLoad() {
    for (const fn of this.__listeners["load"] ?? []) fn.call(this);
  }
}

interface Harness {
  window: {
    fetch: (input: unknown, init?: unknown) => Promise<unknown>;
    location: { origin: string; href: string; replace: ReturnType<typeof vi.fn> };
    __BK_CREDS?: unknown;
  };
  XHR: typeof FakeXHR;
  fetchMock: ReturnType<typeof vi.fn>;
}

/** Load a fresh copy of the capture script against faked globals. */
function loadScript(fetchResult?: unknown): Harness {
  const fetchMock = vi.fn(() =>
    Promise.resolve(fetchResult ?? { url: "", headers: { get: () => null } }),
  );
  const window: Harness["window"] = {
    fetch: fetchMock,
    location: {
      origin: "https://app.getsling.com",
      href: "https://app.getsling.com/",
      replace: vi.fn(),
    },
  };
  // Fresh XHR class per load so prototype patches don't leak between tests.
  const XHR = class extends FakeXHR {};
  const document = {
    documentElement: {},
    body: null,
    addEventListener() {},
    createElement: () => ({ style: {} as Record<string, string> }),
  };
  const fn = new Function(
    "window",
    "document",
    "XMLHttpRequest",
    "MutationObserver",
    "setTimeout",
    src,
  );
  fn(
    window,
    document,
    XHR,
    class {
      observe() {}
      disconnect() {}
    },
    () => 0, // timers are no-ops in tests
  );
  return { window, XHR, fetchMock };
}

function capturedUrl(h: Harness): string | undefined {
  return h.window.location.replace.mock.calls[0]?.[0];
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("request-header capture (existing behavior)", () => {
  it("captures from a fetch with an Authorization header", () => {
    const h = loadScript();
    h.window.fetch("https://api.getsling.com/v1/account", {
      headers: { Authorization: TOKEN },
    });
    expect(capturedUrl(h)).toBe(
      `https://app.getsling.com/__bk_capture?t=${encodeURIComponent(TOKEN)}`,
    );
  });

  it("captures from an XHR setRequestHeader and extracts the org id", () => {
    const h = loadScript();
    const xhr = new h.XHR();
    xhr.open("GET", "https://api.getsling.com/v1/8675309/shifts");
    xhr.setRequestHeader("Authorization", TOKEN);
    expect(capturedUrl(h)).toContain(`t=${encodeURIComponent(TOKEN)}`);
    expect(capturedUrl(h)).toContain("&o=8675309");
  });

  it("ignores non-api hosts and short tokens", () => {
    const h = loadScript();
    h.window.fetch("https://analytics.example.com/v1/x", {
      headers: { Authorization: TOKEN },
    });
    h.window.fetch("https://api.getsling.com/v1/account", {
      headers: { Authorization: "short" },
    });
    expect(h.window.location.replace).not.toHaveBeenCalled();
  });
});

describe("response-header capture (login POST carries the token)", () => {
  it("captures the authorization header from a fetch response", async () => {
    const h = loadScript({
      url: "https://api.getsling.com/v1/account/login",
      headers: {
        get: (n: string) => (n.toLowerCase() === "authorization" ? TOKEN : null),
      },
    });
    // Login POST itself has no Authorization request header.
    h.window.fetch("https://api.getsling.com/v1/account/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
    });
    await flush();
    expect(capturedUrl(h)).toBe(
      `https://app.getsling.com/__bk_capture?t=${encodeURIComponent(TOKEN)}`,
    );
  });

  it("captures the authorization header from an XHR response", () => {
    const h = loadScript();
    const xhr = new h.XHR();
    xhr.open("POST", "https://api.getsling.com/v1/account/login");
    xhr.__responseHeaders["authorization"] = TOKEN;
    xhr.fireLoad();
    expect(capturedUrl(h)).toBe(
      `https://app.getsling.com/__bk_capture?t=${encodeURIComponent(TOKEN)}`,
    );
  });

  it("ignores response headers from non-api hosts", async () => {
    const h = loadScript({
      url: "https://cdn.example.com/asset.js",
      headers: { get: () => TOKEN },
    });
    h.window.fetch("https://cdn.example.com/asset.js");
    await flush();
    expect(h.window.location.replace).not.toHaveBeenCalled();
  });

  it("does not double-capture when both request and response match", async () => {
    const h = loadScript({
      url: "https://api.getsling.com/v1/account",
      headers: { get: () => TOKEN },
    });
    h.window.fetch("https://api.getsling.com/v1/account", {
      headers: { Authorization: TOKEN },
    });
    await flush();
    expect(h.window.location.replace).toHaveBeenCalledTimes(1);
  });
});

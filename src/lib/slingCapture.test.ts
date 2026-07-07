// Tests for src-tauri/src/sling_login_capture.js — the script injected into
// the Sling login webview. Loaded here as text and run against faked
// window/document/XHR globals (vitest runs in node, no jsdom needed).
import src from "../../src-tauri/src/sling_login_capture.js?raw";
import { describe, expect, it, vi } from "vitest";

const TOKEN = "0123456789abcdef0123456789abcdef"; // >= 20 chars

class FakeXHR {
  open(_method: string, _url: string) {}
  setRequestHeader(_name: string, _value: string) {}
  send() {}
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
function loadScript(): Harness {
  const fetchMock = vi.fn(() => Promise.resolve({ url: "", headers: { get: () => null } }));
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

// Capture keys off the Authorization REQUEST header. On an already-signed-in
// session the SPA's first /v1/ call carries it, so this fires ~instantly;
// reading the response side is impossible cross-origin (see the script's
// header comment), which is why there is no response-capture path to test.
describe("request-header capture", () => {
  it("captures from a fetch with an Authorization header", () => {
    const h = loadScript();
    h.window.fetch("https://api.getsling.com/v1/account", {
      headers: { Authorization: TOKEN },
    });
    expect(capturedUrl(h)).toBe(
      `https://app.getsling.com/__bk_capture?t=${encodeURIComponent(TOKEN)}`,
    );
  });

  it("captures from a fetch whose init.headers is a Headers instance", () => {
    const h = loadScript();
    const headers = new Map([["authorization", TOKEN]]);
    h.window.fetch("https://api.getsling.com/v1/account", {
      headers: { get: (k: string) => headers.get(k.toLowerCase()) ?? null },
    });
    expect(capturedUrl(h)).toContain(`t=${encodeURIComponent(TOKEN)}`);
  });

  it("captures from an XHR setRequestHeader and extracts the org id", () => {
    const h = loadScript();
    const xhr = new h.XHR() as FakeXHR & {
      open: (m: string, u: string) => void;
      setRequestHeader: (n: string, v: string) => void;
    };
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

  it("only captures once", () => {
    const h = loadScript();
    h.window.fetch("https://api.getsling.com/v1/account", { headers: { Authorization: TOKEN } });
    h.window.fetch("https://api.getsling.com/v1/other", { headers: { Authorization: TOKEN } });
    expect(h.window.location.replace).toHaveBeenCalledTimes(1);
  });
});

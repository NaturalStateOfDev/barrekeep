import { describe, it, expect } from "vitest";
import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { parseTimeFormat, useTimeFormat } from "./timeFormat";

describe("parseTimeFormat", () => {
  it("defaults to 12h for anything but an explicit 24h", () => {
    expect(parseTimeFormat("24h")).toBe("24h");
    expect(parseTimeFormat("12h")).toBe("12h");
    expect(parseTimeFormat(null)).toBe("12h");
    expect(parseTimeFormat(undefined)).toBe("12h");
    expect(parseTimeFormat("garbage")).toBe("12h");
  });
});

describe("useTimeFormat", () => {
  it("formats in 12-hour time outside a provider (the default)", () => {
    function Probe() {
      const tf = useTimeFormat();
      return createElement("span", null, `${tf.fmt}|${tf.time("17:30")}|${tf.range("09:45", "10:35")}`);
    }
    expect(renderToString(createElement(Probe))).toContain("12h|5:30 PM|9:45 – 10:35 AM");
  });
});

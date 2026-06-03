import { describe, it, expect } from "vitest";
import { chipFor } from "./formatChips";

describe("chipFor", () => {
  it("maps Classic to its slate token", () => {
    expect(chipFor("Classic")).toEqual({ label: "Classic", token: "--chip-slate" });
  });

  it("maps Empower to its terracotta token", () => {
    expect(chipFor("Empower")).toEqual({ label: "Empower", token: "--chip-terracotta" });
  });

  it("maps Define to its sage token", () => {
    expect(chipFor("Define")).toEqual({ label: "Define", token: "--chip-sage" });
  });

  it("maps Reform to its ochre token", () => {
    expect(chipFor("Reform")).toEqual({ label: "Reform", token: "--chip-ochre" });
  });

  it("maps Foundations to its plum token", () => {
    expect(chipFor("Foundations")).toEqual({ label: "Foundations", token: "--chip-plum" });
  });

  it("maps Focus to its accent token", () => {
    expect(chipFor("Focus")).toEqual({ label: "Focus", token: "--chip-accent" });
  });

  it("falls back to the raw class name with the default token", () => {
    expect(chipFor("Mystery Class")).toEqual({ label: "Mystery Class", token: "--chip-default" });
  });
});

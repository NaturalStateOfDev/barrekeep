import { describe, it, expect } from "vitest";
import { avatarTint, AVATAR_TINTS } from "./avatar";

describe("avatarTint", () => {
  it("is deterministic for the same name", () => {
    expect(avatarTint("Alex Braun")).toBe(avatarTint("Alex Braun"));
  });

  it("always returns a color from the harmonized palette", () => {
    for (const name of ["Alex Braun", "Kayla Moore", "Casey Diaz", "Jordan Lee", "Priya Shah", "Morgan Ellis", "X"]) {
      expect(AVATAR_TINTS).toContain(avatarTint(name));
    }
  });

  it("spreads distinct names across more than one tint", () => {
    const tints = new Set(
      ["Alex Braun", "Kayla Moore", "Casey Diaz", "Jordan Lee", "Priya Shah", "Morgan Ellis"].map(avatarTint),
    );
    expect(tints.size).toBeGreaterThan(1);
  });

  it("handles null/empty names without throwing", () => {
    expect(AVATAR_TINTS).toContain(avatarTint(null));
    expect(AVATAR_TINTS).toContain(avatarTint(""));
  });
});

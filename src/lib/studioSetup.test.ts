import { describe, it, expect } from "vitest";
import { buildPickerModel, isStudioComplete, isStudioNotConfigured, studioSummary } from "./studioSetup";
import type { DiscoveredStudio } from "../types";

const found: DiscoveredStudio = {
  org_id: 10,
  acting_user_id: 20,
  acting_user_name: "Lead Teacher",
  org_name: "Barre Co",
  locations: [
    { id: 30, name: "Downtown" },
    { id: 31, name: "Uptown" },
  ],
};
const unset = { org_id: 0, acting_user_id: 0, home_location_id: 0 };

describe("buildPickerModel", () => {
  it("uses names and preselects sole candidates only", () => {
    const m = buildPickerModel(found, unset);
    expect(m.orgs).toEqual([{ id: 10, label: "Barre Co (10)" }]);
    expect(m.users[0].label).toBe("Lead Teacher (20)");
    expect(m.locations.map((l) => l.label)).toEqual(["Downtown", "Uptown"]);
    expect(m.selected).toEqual({ org_id: 10, acting_user_id: 20, home_location_id: 0 });
  });

  it("preselects current values and keeps ones Sling didn't report", () => {
    const m = buildPickerModel(found, { org_id: 10, acting_user_id: 99, home_location_id: 77 });
    expect(m.selected).toEqual({ org_id: 10, acting_user_id: 99, home_location_id: 77 });
    expect(m.users.map((u) => u.id)).toEqual([20, 99]);
    expect(m.locations.map((l) => l.id)).toEqual([30, 31, 77]);
    expect(m.locations[2].label).toMatch(/Currently configured location/);
  });

  it("falls back to ids without names", () => {
    const m = buildPickerModel({ ...found, org_name: "", acting_user_name: "" }, unset);
    expect(m.orgs[0].label).toBe("Organization 10");
    expect(m.users[0].label).toBe("User 20");
  });
});

describe("helpers", () => {
  it("recognizes the backend's not-configured error", () => {
    expect(isStudioNotConfigured("Studio not configured — use “Set up studio” …")).toBe(true);
    expect(isStudioNotConfigured("sling-401")).toBe(false);
  });

  it("isStudioComplete requires every id", () => {
    expect(isStudioComplete(unset)).toBe(false);
    expect(isStudioComplete({ org_id: 1, acting_user_id: 0, home_location_id: 3 })).toBe(false);
    expect(isStudioComplete({ org_id: 1, acting_user_id: 2, home_location_id: 3 })).toBe(true);
  });

  it("studioSummary names org and location", () => {
    expect(studioSummary(found, { org_id: 10, acting_user_id: 20, home_location_id: 31 })).toBe("Barre Co · Uptown");
    expect(studioSummary(found, { org_id: 11, acting_user_id: 20, home_location_id: 99 })).toBe("org 11 · location 99");
  });
});

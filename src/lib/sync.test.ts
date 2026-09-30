import { describe, expect, it } from "vitest";
import { groupSyncActions, hasWork, pushLabel, slingChangeCount, updateLabel } from "./sync";
import type { SyncAction, SyncPreview } from "../types";

const view = (teacher: string, start = "09:00", end = "10:00", cls = "Classic") => ({
  date: "2026-11-03",
  start,
  end,
  class_name: cls,
  teacher_name: teacher,
});

const action = (kind: SyncAction["kind"], extra: Partial<SyncAction> = {}): SyncAction => ({
  kind,
  proposal_shift_id: 1,
  sling_shift_id: null,
  before: null,
  after: view("Alex"),
  reason: "",
  from_draft: null,
  skip_outcome: null,
  ...extra,
});

const preview = (actions: SyncAction[]): SyncPreview => ({
  mode: "push",
  proposal_id: 1,
  draft_name: "Draft 1",
  target_month: "2026-11",
  actions,
  unchanged: 0,
  cleanup: false,
  cleanup_offers: [],
  plan_key: "",
});

describe("sync helpers", () => {
  it("groups actions by kind", () => {
    const g = groupSyncActions([
      action("create"),
      action("update"),
      action("create"),
      action("skip"),
      action("skip", { skip_outcome: "skipped_missing" }),
      action("baseline"),
    ]);
    expect([g.create.length, g.update.length, g.skip.length, g.delete.length]).toEqual([2, 1, 1, 0]);
    expect([g.deletedInSling.length, g.baseline.length]).toEqual([1, 1]);
  });

  it("counts only Sling-changing actions", () => {
    const p = preview([action("create"), action("cleanup"), action("adopt"), action("skip")]);
    expect(slingChangeCount(p)).toBe(2);
  });

  it("has work for bookkeeping-only plans, not for unrecorded skips", () => {
    expect(hasWork(preview([action("skip")]))).toBe(false);
    expect(hasWork(preview([action("skip", { skip_outcome: "skipped_missing" })]))).toBe(true);
    expect(hasWork(preview([action("adopt")]))).toBe(true);
    expect(hasWork(preview([]))).toBe(false);
  });

  it("describes what an update changes", () => {
    expect(updateLabel(action("update", { before: view("Alex"), after: view("Kay") }))).toBe("Alex → Kay");
    expect(
      updateLabel(action("update", { before: view("Alex"), after: view("Alex", "09:30", "10:30", "Define") })),
    ).toBe("9:00a–10:00a → 9:30a–10:30a, Classic → Define");
  });

  it("labels the push button by whether the draft is already in Sling", () => {
    expect(pushLabel({ sling_shift_count: 0 })).toBe("Push to Sling");
    expect(pushLabel({ sling_shift_count: 12 })).toBe("Sync to Sling");
  });
});

import { describe, it, expect } from "vitest";
import {
  codifyInstruction,
  confirmLabel,
  diffLines,
  ruleDiffLabel,
  ruleLines,
  unexpectedSlots,
} from "./rules";
import type { CandidateValidation, SlotChange } from "../types";

const teacher = (uid: unknown) => (String(uid) === "501" ? "Alex" : `teacher ${uid}`);

describe("ruleLines", () => {
  it("labels list, map and scalar rules with teacher names", () => {
    const lines = ruleLines(
      {
        teacher_class_blocklist: [{ sling_user_id: 501, class_name: "Reform", reason: "swaps" }],
        variety_penalty_multiplier: { "501": 2 },
        variety_penalty_per_class: 0.5,
        sat_time_shifts: { "08:00": "08:30" },
      },
      teacher,
    );
    expect(lines).toEqual([
      "Alex — never Reform (swaps)",
      "Alex — variety penalty ×2",
      "Variety penalty per class: 0.5",
      "Saturday 8:00 AM class moves to 8:30 AM",
    ]);
  });

  it("keeps rule times as HH:MM in 24-hour mode", () => {
    expect(ruleLines({ sat_time_shifts: { "08:00": "08:30" } }, teacher, "24h")).toEqual([
      "Saturday 08:00 class moves to 08:30",
    ]);
  });

  it("handles an empty rule set", () => {
    expect(ruleLines({}, teacher)).toEqual([]);
  });
});

describe("ruleDiffLabel", () => {
  it("shows before → after for changed entries and the old value for removed ones", () => {
    expect(
      ruleDiffLabel(
        { rule_key: "variety_penalty_per_class", identity: "", kind: "changed", before: 0.3, after: 0.5 },
        teacher,
      ),
    ).toBe("Variety penalty per class: 0.3 → Variety penalty per class: 0.5");
    expect(
      ruleDiffLabel(
        {
          rule_key: "teacher_slot_blocklist",
          identity: "501 · Wed · 05:45",
          kind: "removed",
          before: { sling_user_id: 501, weekday: "Wed", time: "05:45" },
          after: null,
        },
        teacher,
      ),
    ).toBe("Alex — never Wed 5:45 AM");
  });
});

describe("codifyInstruction", () => {
  it("asks for a rule proposal and no shift edits, quoting the suggestion", () => {
    const text = codifyInstruction({ summary: "Casey never on Reform", rationale: "3 swaps" });
    expect(text).toContain("ruleset_proposal");
    expect(text).toContain("NOT propose any shift edits");
    expect(text).toContain("Suggestion: Casey never on Reform");
    expect(text).toContain("Rationale: 3 swaps");
  });
});

function validation(over: Partial<CandidateValidation>): CandidateValidation {
  return {
    status: "needs_confirm",
    error: null,
    reasons: [],
    month: "2026-08",
    slot_count: 20,
    candidate_slot_count: 21,
    changed_count: 0,
    added_count: 0,
    removed_count: 0,
    unexpected_count: 0,
    changed_pct: 0,
    changes: [],
    ...over,
  };
}

function slot(kind: SlotChange["kind"], expected = false): SlotChange {
  return {
    date: "2026-08-04", weekday: "Tue", start: "07:00", kind,
    class_before: kind === "added" ? null : "Classic",
    class_after: kind === "removed" ? null : "Classic",
    teacher_before: kind === "added" ? null : "Alex",
    teacher_after: kind === "removed" ? null : "Kay",
    expected,
  };
}

describe("confirmLabel / unexpectedSlots", () => {
  it("names the unexplained slot count, ignoring time-shift moves", () => {
    const v = validation({
      changes: [slot("added"), slot("added"), slot("removed"), slot("added", true), slot("changed")],
    });
    const u = unexpectedSlots(v);
    expect([u.added.length, u.removed.length]).toEqual([2, 1]);
    expect(confirmLabel(v)).toBe("I intend to add 2 / remove 1 class slots");
  });

  it("covers slot and threshold confirmations together", () => {
    const v = validation({ changes: [slot("removed")], changed_count: 8, changed_pct: 0.4 });
    expect(confirmLabel(v)).toBe("I intend to remove 1 class slot and accept the 8 changed assignments");
    expect(confirmLabel(validation({ changed_count: 8, changed_pct: 0.4 }))).toBe(
      "I've reviewed and accept the 8 changed assignments",
    );
  });
});

describe("diffLines", () => {
  it("classifies unified diff lines", () => {
    const kinds = diffLines("--- a\n+++ b\n@@ -1,2 +1,2 @@\n x\n-y\n+z\n").map((l) => l.kind);
    expect(kinds).toEqual(["meta", "meta", "hunk", "ctx", "del", "add"]);
  });
});

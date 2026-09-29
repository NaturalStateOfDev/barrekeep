import { describe, it, expect } from "vitest";
import { codifyInstruction, diffLines, ruleDiffLabel, ruleLines } from "./rules";

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
    ).toBe("Alex — never Wed 05:45");
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

describe("diffLines", () => {
  it("classifies unified diff lines", () => {
    const kinds = diffLines("--- a\n+++ b\n@@ -1,2 +1,2 @@\n x\n-y\n+z\n").map((l) => l.kind);
    expect(kinds).toEqual(["meta", "meta", "hunk", "ctx", "del", "add"]);
  });
});

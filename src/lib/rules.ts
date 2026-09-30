// Human-readable labels for algorithm rules (rules-as-data, see
// src-tauri/src/algorithm.rs) and their diffs, plus unified-diff parsing
// for the script diff view.

import type { CandidateValidation, ReviewSuggestion, RuleDiffEntry, SlotChange } from "../types";

/** The editor instruction that turns a Claude-review suggestion into a rule
 *  proposal (flows into the same validate → diff → adopt card). */
export function codifyInstruction(s: Pick<ReviewSuggestion, "summary" | "rationale">): string {
  return (
    "Codify this review suggestion as a standing algorithm rule. Return a ruleset_proposal " +
    "if the rule keys can express it (otherwise needs_code_change explaining why), and do " +
    "NOT propose any shift edits.\n" +
    `Suggestion: ${s.summary}\nRationale: ${s.rationale}`
  );
}

export type TeacherName = (uid: unknown) => string;

/** Label one rule entry of `key` (a list item, a map value, or a scalar). */
export function ruleEntryLabel(
  key: string,
  identity: string,
  value: any,
  teacher: TeacherName,
): string {
  switch (key) {
    case "teacher_class_blocklist":
      return `${teacher(value?.sling_user_id)} — never ${value?.class_name}${value?.reason ? ` (${value.reason})` : ""}`;
    case "teacher_slot_blocklist":
      return `${teacher(value?.sling_user_id)} — never ${value?.weekday} ${value?.time}${value?.reason ? ` (${value.reason})` : ""}`;
    case "priority_slots":
      return `${teacher(value?.sling_user_id)} — preferred for ${value?.weekday} ${value?.time}`;
    case "slot_class_overrides":
      return `${value?.weekday} ${value?.time} is always ${value?.class_name}`;
    case "variety_penalty_multiplier":
      return `${teacher(identity)} — variety penalty ×${value}`;
    case "variety_penalty_per_class":
      return `Variety penalty per class: ${value}`;
    case "sat_time_shifts":
      return `Saturday ${identity} class moves to ${value}`;
    case "sun_time_shifts":
      return `Sunday ${identity} class moves to ${value}`;
    case "slot_continuity_bonus":
      return `Keep teachers on the same weekday+time: bonus ${value} per repeat`;
    default:
      return `${key}${identity ? ` ${identity}` : ""}: ${JSON.stringify(value)}`;
  }
}

/** Every standing rule in a rule set, one line each. */
export function ruleLines(rules: Record<string, unknown>, teacher: TeacherName): string[] {
  const out: string[] = [];
  for (const [key, value] of Object.entries(rules ?? {})) {
    if (Array.isArray(value)) {
      for (const item of value) out.push(ruleEntryLabel(key, "", item, teacher));
    } else if (value && typeof value === "object") {
      for (const [id, v] of Object.entries(value as Record<string, unknown>))
        out.push(ruleEntryLabel(key, id, v, teacher));
    } else if (value != null) {
      out.push(ruleEntryLabel(key, "", value, teacher));
    }
  }
  return out;
}

/** One line describing a rules-diff entry. */
export function ruleDiffLabel(e: RuleDiffEntry, teacher: TeacherName): string {
  if (e.kind === "changed") {
    return `${ruleEntryLabel(e.rule_key, e.identity, e.before, teacher)} → ${ruleEntryLabel(e.rule_key, e.identity, e.after, teacher)}`;
  }
  return ruleEntryLabel(e.rule_key, e.identity, e.kind === "removed" ? e.before : e.after, teacher);
}

/** Slots that appear/disappear with no time-shift rule change explaining them. */
export function unexpectedSlots(v: CandidateValidation): { added: SlotChange[]; removed: SlotChange[] } {
  return {
    added: v.changes.filter((c) => c.kind === "added" && !c.expected),
    removed: v.changes.filter((c) => c.kind === "removed" && !c.expected),
  };
}

/** Text of the explicit "adopt anyway" confirm, naming exactly what is
 *  being accepted. */
export function confirmLabel(v: CandidateValidation, threshold = 0.25): string {
  const { added, removed } = unexpectedSlots(v);
  const parts: string[] = [];
  if (added.length || removed.length) {
    const what = [
      added.length ? `add ${added.length}` : null,
      removed.length ? `remove ${removed.length}` : null,
    ]
      .filter(Boolean)
      .join(" / ");
    const n = added.length + removed.length;
    parts.push(`I intend to ${what} class slot${n === 1 ? "" : "s"}`);
  }
  if (v.changed_pct > threshold) {
    parts.push(
      `${parts.length ? "accept" : "I've reviewed and accept"} the ${v.changed_count} changed assignments`,
    );
  }
  return parts.join(" and ");
}

export type DiffLineKind = "add" | "del" | "hunk" | "meta" | "ctx";

/** Classify unified-diff lines for colouring. */
export function diffLines(diff: string): { kind: DiffLineKind; text: string }[] {
  return diff
    .split("\n")
    .filter((l, i, all) => !(l === "" && i === all.length - 1))
    .map((text) => {
      if (text.startsWith("+++") || text.startsWith("---")) return { kind: "meta" as const, text };
      if (text.startsWith("@@")) return { kind: "hunk" as const, text };
      if (text.startsWith("+")) return { kind: "add" as const, text };
      if (text.startsWith("-")) return { kind: "del" as const, text };
      return { kind: "ctx" as const, text };
    });
}

// Browser-only preview data. Loaded exclusively from main.tsx when running
// `npm run dev` outside the Tauri shell (import.meta.env.DEV guard), so none
// of this ships in a production build. Mirrors the placeholder demo roster
// from the design-system UI kit / seed.rs — real data comes from Sling.

import { mockIPC } from "@tauri-apps/api/mocks";
import { consistencyStats } from "./drafts";
import { blocksOnDate, monthDates } from "./availability";
import { emptyWeek } from "./studioHours";
import type {
  AlgorithmVersion,
  BackupEntry,
  DraftSlotDiff,
  Teacher,
  Position,
  ProposalSummary,
  ProposalShiftRow,
  EditRow,
  AvailabilityBlock,
  AvailabilityWindow,
  DayHours,
  DayRange,
  MonthAvailability,
  ExternalShiftRow,
  DraftConflict,
  ShiftView,
  SyncAction,
  SyncPreview,
} from "../types";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const TEACHERS: Teacher[] = [
  { sling_user_id: 1930001, display_name: "Alex Braun", weekly_target: 4, weekly_max: 5, is_lead: true, ranking_weight: 3, variety_multiplier: 1, active: true, notes: null, locations: "Downtown" },
  { sling_user_id: 1930002, display_name: "Kayla Moore", weekly_target: 4, weekly_max: 5, is_lead: false, ranking_weight: 2, variety_multiplier: 1, active: true, notes: null, locations: "Downtown" },
  { sling_user_id: 1930003, display_name: "Casey Diaz", weekly_target: 3, weekly_max: 4, is_lead: false, ranking_weight: 2, variety_multiplier: 1, active: true, notes: null, locations: "Downtown" },
  { sling_user_id: 1930004, display_name: "Jordan Lee", weekly_target: 3, weekly_max: 4, is_lead: false, ranking_weight: 1, variety_multiplier: 1, active: true, notes: null, locations: "Downtown" },
  { sling_user_id: 1930005, display_name: "Priya Shah", weekly_target: 5, weekly_max: 6, is_lead: false, ranking_weight: 2, variety_multiplier: 1, active: true, notes: null, locations: "Downtown" },
  { sling_user_id: 1930006, display_name: "Morgan Ellis", weekly_target: 2, weekly_max: 3, is_lead: false, ranking_weight: 1, variety_multiplier: 1, active: true, notes: null, locations: "Uptown" },
];

const POSITIONS: Position[] = [
  { sling_position_id: 101, class_name: "Classic", duration_minutes: 50, is_special: false, active: true },
  { sling_position_id: 102, class_name: "Empower", duration_minutes: 45, is_special: false, active: true },
  { sling_position_id: 103, class_name: "Define", duration_minutes: 50, is_special: false, active: true },
  { sling_position_id: 104, class_name: "Reform", duration_minutes: 50, is_special: false, active: true },
  { sling_position_id: 105, class_name: "Foundations", duration_minutes: 45, is_special: true, active: true },
  { sling_position_id: 106, class_name: "Focus", duration_minutes: 30, is_special: true, active: true },
  { sling_position_id: 107, class_name: "Sales Rep", duration_minutes: 0, is_special: false, active: false },
];

const QUALIFIED: Record<number, string[]> = {
  1930001: ["Classic", "Empower", "Define", "Reform", "Foundations", "Focus"],
  1930002: ["Classic", "Empower", "Reform", "Focus"],
  1930003: ["Classic", "Define", "Foundations"],
  1930004: ["Empower", "Define", "Focus"],
  1930005: ["Classic", "Empower", "Define", "Reform", "Focus"],
  1930006: ["Classic", "Define", "Foundations"],
};

const positionByName = new Map(POSITIONS.map((p) => [p.class_name, p]));

function addMinutes(hhmm: string, minutes: number): string {
  const [h, m] = hhmm.split(":").map(Number);
  const total = h * 60 + m + minutes;
  return `${String(Math.floor(total / 60) % 24).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
}

let nextShiftId = 1000;

/** "consistent" keeps each weekday+time with one teacher all month (the
 *  slot_continuity_bonus what-if); "rotate" is the default rotation. */
function buildShifts(ym: string, mode: "rotate" | "consistent" = "rotate"): ProposalShiftRow[] {
  const [y, m] = ym.split("-").map(Number);
  const daysInMonth = new Date(Date.UTC(y, m, 0)).getUTCDate();
  const weekdayTemplate = [
    { time: "05:45", format: "Classic" },
    { time: "09:00", format: "Empower" },
    { time: "10:15", format: "Define" },
    { time: "17:30", format: "Reform" },
    { time: "18:45", format: "Focus" },
  ];
  const satTemplate = [
    { time: "08:00", format: "Classic" },
    { time: "09:15", format: "Foundations" },
  ];
  const out: ProposalShiftRow[] = [];
  for (let d = 1; d <= daysInMonth; d++) {
    const iso = `${ym}-${String(d).padStart(2, "0")}`;
    const dow = new Date(iso + "T12:00:00Z").getUTCDay();
    if (dow === 0) continue; // no Sunday classes
    const tmpl = dow === 6 ? satTemplate : weekdayTemplate;
    tmpl.forEach((slot, i) => {
      const pos = positionByName.get(slot.format)!;
      const teacher = TEACHERS[((mode === "consistent" ? dow : d) + i) % TEACHERS.length];
      const unassigned = d === 12 && slot.time === "09:00";
      const notQualified = d === 12 && slot.time === "17:30"; // Casey on Reform
      const dropped = d === 5 && slot.time === "18:45";
      const assigned = notQualified ? TEACHERS[2] : teacher;
      out.push({
        id: nextShiftId++,
        shift_date: iso,
        start_time: slot.time,
        end_time: addMinutes(slot.time, pos.duration_minutes),
        class_name: slot.format,
        sling_position_id: pos.sling_position_id,
        teacher_name: unassigned || dropped ? null : assigned.display_name,
        sling_user_id: unassigned || dropped ? null : assigned.sling_user_id,
        generation_reason: "rotation",
        flag: notQualified ? "qualification" : null,
        is_coteach: false,
        coteach_label: null,
        is_dropped: dropped,
      });
    });
  }
  return out;
}

interface MockProposal {
  summary: ProposalSummary;
  shifts: ProposalShiftRow[];
}

function mockSummary(
  over: Pick<ProposalSummary, "id" | "target_month" | "algorithm_version" | "generated_at" | "is_current" | "name"> &
    Partial<ProposalSummary>,
): ProposalSummary {
  return {
    shift_count: 0,
    dropped_count: 0,
    edit_count: 0,
    archived: false,
    parent_proposal_id: null,
    created_from: "generate",
    is_push_candidate: false,
    pushed: false,
    sling_shift_count: 0,
    ...over,
  };
}

const PROPOSALS: MockProposal[] = [
  {
    summary: mockSummary({ id: 8, target_month: "2026-08", algorithm_version: "v10", generated_at: "2026-07-04 11:02:40", is_current: true, name: "Consistent days" }),
    shifts: buildShifts("2026-08", "consistent"),
  },
  {
    summary: mockSummary({ id: 7, target_month: "2026-08", algorithm_version: "v3", generated_at: "2026-07-03 09:14:02", is_current: false, name: "Draft 1", dropped_count: 1 }),
    shifts: buildShifts("2026-08"),
  },
  {
    summary: mockSummary({ id: 6, target_month: "2026-07", algorithm_version: "v3", generated_at: "2026-06-24 08:02:11", is_current: true, name: "Draft 1", edit_count: 5, pushed: true }),
    shifts: buildShifts("2026-07"),
  },
  {
    summary: mockSummary({ id: 5, target_month: "2026-06", algorithm_version: "v2", generated_at: "2026-05-26 10:41:37", is_current: true, name: "Draft 1", dropped_count: 2, edit_count: 2, pushed: true }),
    shifts: buildShifts("2026-06"),
  },
];
for (const p of PROPOSALS) p.summary.shift_count = p.shifts.filter((s) => !s.is_dropped).length;

/** Mirrors month_push_candidate: month -> the draft Push sends. */
const PUSH_DRAFT = new Map<string, number>([
  ["2026-08", 7],
  ["2026-07", 6],
  ["2026-06", 5],
]);

function summaryOf(p: MockProposal): ProposalSummary {
  return {
    ...p.summary,
    is_push_candidate: PUSH_DRAFT.get(p.summary.target_month) === p.summary.id,
    sling_shift_count: SLING.get(p.summary.id)?.size ?? 0,
  };
}

// ---- Mock Sling tracking (mirrors push_sync.rs) ----
// proposal id -> proposal_shift id -> the shift as last pushed (+ Sling status).
type MockSlingShift = {
  row: ProposalShiftRow;
  sling_id: number;
  status: "planning" | "published" | "deleted";
  /** Pushed before sync tracking (no snapshot yet). */
  legacy?: boolean;
};
const SLING = new Map<number, Map<number, MockSlingShift>>();
let nextSlingId = 5_000_000;
/** Drafts whose month data is newer than their last check (get_proposal.is_stale). */
const STALE = new Set<number>([6]);
/** Conflicts a check "finds": proposal id -> (date|start|user) keys with a kind. */
const CONFLICT_SEEDS = new Map<number, Map<string, DraftConflict["kind"]>>();

function seedSling(id: number, publishedFirst = false) {
  const p = PROPOSALS.find((x) => x.summary.id === id);
  if (!p) return;
  const m = new Map<number, MockSlingShift>();
  // Everything but the last 3 assigned shifts is "in Sling" (so a sync has creates).
  const rows = p.shifts.filter((s) => !s.is_dropped && s.sling_user_id != null);
  rows.slice(0, Math.max(0, rows.length - 3)).forEach((r, i) =>
    m.set(r.id, {
      row: { ...r },
      sling_id: nextSlingId++,
      // Exercise the preview: #0 published, #1 deleted in Sling, #2 a legacy push.
      status: publishedFirst && i === 0 ? "published" : publishedFirst && i === 1 ? "deleted" : "planning",
      legacy: publishedFirst && i === 2,
    }),
  );
  SLING.set(id, m);
  p.summary.pushed = true;
}
seedSling(7, true); // August "Draft 1" = push draft, pushed, first shift published since
seedSling(6);

const view = (r: ProposalShiftRow): ShiftView => ({
  date: r.shift_date,
  start: r.start_time,
  end: r.end_time,
  class_name: r.class_name,
  teacher_name: r.teacher_name ?? "?",
});
const sameShift = (a: ProposalShiftRow, b: ProposalShiftRow) =>
  a.shift_date === b.shift_date &&
  a.start_time === b.start_time &&
  a.end_time === b.end_time &&
  a.sling_user_id === b.sling_user_id &&
  a.sling_position_id === b.sling_position_id;
const act = (kind: SyncAction["kind"], ps: number, x: Partial<SyncAction>): SyncAction => ({
  kind,
  proposal_shift_id: ps,
  sling_shift_id: null,
  before: null,
  after: null,
  reason: "",
  from_draft: null,
  skip_outcome: null,
  ...x,
});
const KIND_ORDER: SyncAction["kind"][] = ["baseline", "adopt", "skip", "cleanup", "delete", "update", "create"];

function mockSyncPlan(id: number, mode: "push" | "remove", cleanup: boolean): SyncPreview {
  const p = findProposal(id);
  const mine = SLING.get(id) ?? new Map<number, MockSlingShift>();
  const actions: SyncAction[] = [];
  let unchanged = 0;
  const others = PROPOSALS.filter(
    (x) => x.summary.target_month === p.summary.target_month && x.summary.id !== id && (SLING.get(x.summary.id)?.size ?? 0) > 0,
  );
  const adopted = new Set<number>();
  if (mode === "remove") {
    for (const [ps, t] of mine) {
      actions.push(
        t.status === "published"
          ? act("skip", ps, { sling_shift_id: t.sling_id, before: view(t.row), reason: "is published in Sling — the app only changes planning shifts", skip_outcome: "skipped_conflict" })
          : act("delete", ps, { sling_shift_id: t.sling_id, before: view(t.row), reason: "removing this draft's shifts from Sling" }),
      );
    }
  } else {
    for (const s of p.shifts) {
      const t = mine.get(s.id);
      const live = !s.is_dropped && s.sling_user_id != null;
      if (t && live && t.status === "deleted")
        actions.push(act("skip", s.id, { sling_shift_id: t.sling_id, before: view(t.row), after: view(s), reason: "deleted in Sling since last push — will be re-created on the next push unless you remove it from the draft", skip_outcome: "skipped_missing" }));
      else if (t && live && sameShift(t.row, s) && t.legacy && t.status === "planning")
        actions.push(act("baseline", s.id, { sling_shift_id: t.sling_id, before: view(s), after: view(s), reason: "pushed before sync tracking; matches the draft — tracked from now on" }));
      else if (t && live && sameShift(t.row, s)) unchanged++;
      else if (t && t.legacy)
        actions.push(act("skip", s.id, { sling_shift_id: t.sling_id, before: view(t.row), after: live ? view(s) : null, reason: "pushed before sync tracking; differs from draft — fix in Sling or remove manually", skip_outcome: "skipped_conflict" }));
      else if (t && t.status === "published")
        actions.push(act("skip", s.id, { sling_shift_id: t.sling_id, before: view(t.row), after: live ? view(s) : null, reason: "is published in Sling — the app only changes planning shifts", skip_outcome: "skipped_conflict" }));
      else if (t && live) actions.push(act("update", s.id, { sling_shift_id: t.sling_id, before: view(t.row), after: view(s), reason: "changed in the draft since the last push" }));
      else if (t) actions.push(act("delete", s.id, { sling_shift_id: t.sling_id, before: view(t.row), reason: "no longer in the draft" }));
      else if (live) {
        let from: MockProposal | undefined;
        let hit: MockSlingShift | undefined;
        for (const o of others) {
          for (const x of SLING.get(o.summary.id)!.values())
            if (!adopted.has(x.sling_id) && x.status === "planning" && sameShift(x.row, s)) {
              hit = x;
              from = o;
              break;
            }
          if (hit) break;
        }
        if (hit && from) {
          adopted.add(hit.sling_id);
          actions.push(act("adopt", s.id, { sling_shift_id: hit.sling_id, before: view(s), after: view(s), from_draft: from.summary.name, reason: "already in Sling from another draft — tracked for this draft now" }));
        } else actions.push(act("create", s.id, { after: view(s), reason: "not in Sling yet" }));
      }
    }
    if (cleanup)
      for (const o of others)
        for (const [ps, x] of SLING.get(o.summary.id)!)
          if (!adopted.has(x.sling_id))
            actions.push(
              x.status === "published"
                ? act("skip", ps, { sling_shift_id: x.sling_id, before: view(x.row), from_draft: o.summary.name, reason: "is published in Sling — the app only changes planning shifts", skip_outcome: "skipped_conflict" })
                : act("cleanup", ps, { sling_shift_id: x.sling_id, before: view(x.row), from_draft: o.summary.name, reason: "pushed from another draft of this month" }),
            );
  }
  actions.sort((a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind));
  const cleanup_offers = mode === "push"
    ? others.map((o) => {
        const rows = [...SLING.get(o.summary.id)!.values()].filter((x) => !adopted.has(x.sling_id));
        return {
          proposal_id: o.summary.id,
          draft_name: o.summary.name,
          removable: rows.filter((x) => x.status === "planning").length,
          blocked: rows.filter((x) => x.status !== "planning").length,
        };
      }).filter((o) => o.removable + o.blocked > 0)
    : [];
  return {
    mode,
    proposal_id: id,
    draft_name: p.summary.name,
    target_month: p.summary.target_month,
    actions,
    unchanged,
    cleanup,
    cleanup_offers,
    plan_key: JSON.stringify(actions.map((a) => [a.kind, a.proposal_shift_id, a.sling_shift_id])),
  };
}

async function mockSyncExecute(id: number, mode: "push" | "remove", cleanup: boolean, planKey: string) {
  const plan = mockSyncPlan(id, mode, cleanup);
  if (plan.plan_key !== planKey) throw new Error("Sling or the draft changed since the preview — review the changes again before pushing.");
  await sleep(1200);
  const p = findProposal(id);
  const mine = SLING.get(id) ?? new Map<number, MockSlingShift>();
  const sum = { push_id: 1, created: 0, updated: 0, deleted: 0, adopted: 0, skipped: 0, failed: 0, aborted: false, backup_warning: null as string | null };
  for (const a of plan.actions) {
    const row = p.shifts.find((s) => s.id === a.proposal_shift_id);
    if (a.kind === "create" && row) { mine.set(row.id, { row: { ...row }, sling_id: nextSlingId++, status: "planning" }); sum.created++; }
    else if (a.kind === "update" && row) { mine.set(row.id, { row: { ...row }, sling_id: nextSlingId++, status: "planning" }); sum.updated++; }
    else if (a.kind === "delete") { mine.delete(a.proposal_shift_id); sum.deleted++; }
    else if (a.kind === "cleanup") { for (const m of SLING.values()) for (const [k, x] of m) if (x.sling_id === a.sling_shift_id) m.delete(k); sum.deleted++; }
    else if (a.kind === "baseline") { const t = mine.get(a.proposal_shift_id); if (t) t.legacy = false; sum.adopted++; }
    else if (a.kind === "skip" && a.skip_outcome === "skipped_missing") { mine.delete(a.proposal_shift_id); sum.skipped++; }
    else if (a.kind === "adopt" && row) {
      for (const m of SLING.values()) for (const [k, x] of m) if (x.sling_id === a.sling_shift_id) m.delete(k);
      mine.set(row.id, { row: { ...row }, sling_id: a.sling_shift_id!, status: "planning" }); sum.adopted++;
    } else sum.skipped++;
  }
  SLING.set(id, mine);
  if (mode === "push") p.summary.pushed = true;
  return sum;
}

function mockCheckConflicts(id: number): DraftConflict[] {
  const p = findProposal(id);
  STALE.delete(id);
  const key = (s: ProposalShiftRow) => `${s.shift_date}|${s.start_time}|${s.sling_user_id}`;
  let seeds = CONFLICT_SEEDS.get(id);
  if (!seeds) {
    // First check "discovers" new blocked time / leave on two assigned slots.
    const rows = p.shifts.filter((s) => !s.is_dropped && s.sling_user_id != null);
    seeds = new Map([[key(rows[3]), "blocked" as const], [key(rows[8]), "leave" as const]]);
    CONFLICT_SEEDS.set(id, seeds);
  }
  const out: DraftConflict[] = [];
  for (const s of p.shifts) {
    const kind = !s.is_dropped ? seeds.get(key(s)) : undefined;
    if (!kind) continue;
    out.push({
      proposal_shift_id: s.id,
      shift_date: s.shift_date,
      start_time: s.start_time,
      end_time: s.end_time,
      class_name: s.class_name,
      sling_user_id: s.sling_user_id,
      teacher_name: s.teacher_name,
      kind,
      message:
        kind === "blocked"
          ? `${s.teacher_name} is marked unavailable (${s.start_time}–${s.end_time}) — overlaps ${s.start_time} ${s.class_name}`
          : `${s.teacher_name} is on leave (all day) — overlaps ${s.start_time} ${s.class_name}`,
    });
  }
  return out;
}

function ensurePushDraft(id: number) {
  const p = findProposal(id);
  const push = PUSH_DRAFT.get(p.summary.target_month);
  if (push !== id) {
    const other = push != null ? PROPOSALS.find((x) => x.summary.id === push) : undefined;
    throw new Error(
      `"${p.summary.name}" is not the push draft for ${p.summary.target_month}` +
        (other ? ` — "${other.summary.name}" is` : "") +
        `. Mark this draft as the push draft ("Use for push") first.`,
    );
  }
}

/** Rough stand-in for drafts::diff_impl (pairs rows per date+time in order). */
function mockDiff(a: MockProposal, b: MockProposal) {
  const key = (s: ProposalShiftRow) => `${s.shift_date}|${s.start_time}`;
  const label = (s: ProposalShiftRow) =>
    s.is_dropped ? "Dropped" : s.coteach_label ?? s.teacher_name ?? "Unassigned";
  const group = (rows: ProposalShiftRow[]) => {
    const m = new Map<string, ProposalShiftRow[]>();
    for (const s of rows) m.set(key(s), [...(m.get(key(s)) ?? []), s]);
    return m;
  };
  const ga = group(a.shifts);
  const gb = group(b.shifts);
  const keys = [...new Set([...ga.keys(), ...gb.keys()])].sort();
  const changes: DraftSlotDiff[] = [];
  for (const k of keys) {
    const ra = ga.get(k) ?? [];
    const rb = gb.get(k) ?? [];
    const [date, start] = k.split("|");
    const weekday = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][new Date(date + "T12:00:00Z").getUTCDay()];
    for (let i = 0; i < Math.max(ra.length, rb.length); i++) {
      const x = ra[i];
      const y = rb[i];
      if (x && y) {
        const fmt = x.class_name !== y.class_name;
        const tch = label(x) !== label(y);
        if (!fmt && !tch) continue;
        changes.push({ date, weekday, start, class_a: x.class_name, class_b: y.class_name, teacher_a: label(x), teacher_b: label(y), kind: fmt && tch ? "format_teacher" : fmt ? "format" : "teacher" });
      } else if (x) {
        changes.push({ date, weekday, start, class_a: x.class_name, class_b: null, teacher_a: label(x), teacher_b: null, kind: "only_a" });
      } else if (y) {
        changes.push({ date, weekday, start, class_a: null, class_b: y.class_name, teacher_a: null, teacher_b: label(y), kind: "only_b" });
      }
    }
  }
  const sa = consistencyStats(a.shifts);
  const sb = consistencyStats(b.shifts);
  const totals = (m: typeof sa) => {
    const classes = [...m.values()].reduce((n, s) => n + s.classes, 0);
    const distinct_slots = [...m.values()].reduce((n, s) => n + s.distinct_slots, 0);
    return { classes, distinct_slots, classes_per_slot: distinct_slots ? classes / distinct_slots : 0 };
  };
  const uids = [...new Set([...sa.keys(), ...sb.keys()])];
  return {
    target_month: a.summary.target_month,
    a_id: a.summary.id,
    b_id: b.summary.id,
    a_name: a.summary.name,
    b_name: b.summary.name,
    changes,
    teachers: uids
      .map((uid) => ({
        sling_user_id: uid,
        name: TEACHERS.find((t) => t.sling_user_id === uid)?.display_name ?? `teacher ${uid}`,
        a: sa.get(uid) ?? null,
        b: sb.get(uid) ?? null,
      }))
      .sort((x, y) => x.name.localeCompare(y.name)),
    totals_a: totals(sa),
    totals_b: totals(sb),
  };
}

const EDITS: EditRow[] = [];
let nextEditId = 1;
let nextProposalId = 9;
let nextRunId = 42;

// One sample per block source (all are time the teacher is NOT available).
const BLOCKS: AvailabilityBlock[] = [
  // leave — approved time off (calendar feed)
  { sling_user_id: 1930004, source: "leave", starts_at: "2026-08-20T08:00:00", ends_at: "2026-08-20T12:00:00" },
  { sling_user_id: 1930006, source: "leave", starts_at: "2026-08-24T00:00:00", ends_at: "2026-08-26T23:59:59" },
  // availability — one-off unavailability (calendar feed)
  { sling_user_id: 1930002, source: "availability", starts_at: "2026-08-13T17:00:00", ends_at: "2026-08-13T19:30:00" },
  // availability_set — a recurring set: Priya, every Tuesday 9–10:15
  ...["04", "11", "18", "25"].map((d) => ({
    sling_user_id: 1930005,
    source: "availability_set",
    starts_at: `2026-08-${d}T09:00:00`,
    ends_at: `2026-08-${d}T10:15:00`,
  })),
  // availability_set_pending — a recurring set not yet approved in Sling:
  // Casey, Saturdays all day
  ...["01", "08", "15", "22", "29"].map((d) => ({
    sling_user_id: 1930003,
    source: "availability_set_pending",
    starts_at: `2026-08-${d}T00:00:00`,
    ends_at: `2026-08-${d}T23:59:59`,
  })),
];

// Studio hours as saved in Settings (empty = not set).
let STUDIO_HOURS: DayHours[] = [];

/** What the mock schedule implies: Mon–Fri 05:45–19:15, Sat 08:00–10:00. */
function impliedHours(): DayHours[] {
  return emptyWeek().map((d) =>
    d.weekday <= 4
      ? { ...d, closed: false, open: "05:45", close: "19:15" }
      : d.weekday === 5
        ? { ...d, closed: false, open: "08:00", close: "10:00" }
        : d,
  );
}

/** Browser-preview twin of availability.rs: day spans minus blocks. */
function mockMonthAvailability(month: string): MonthAvailability {
  const hours = STUDIO_HOURS.length > 0 ? STUDIO_HOURS : impliedHours();
  const day_ranges: DayRange[] = [];
  const windows: AvailabilityWindow[] = [];
  for (const date of monthDates(month)) {
    const weekday = (new Date(`${date}T12:00:00Z`).getUTCDay() + 6) % 7;
    const h = hours.find((x) => x.weekday === weekday);
    if (!h || h.closed || !h.open || !h.close) continue;
    day_ranges.push({ date, open: h.open, close: h.close, widened: false });
    for (const t of TEACHERS.filter((x) => x.active)) {
      let cursor = h.open;
      for (const b of blocksOnDate(BLOCKS, t.sling_user_id, date)) {
        const from = b.start < h.open ? h.open : b.start;
        const to = b.end > h.close ? h.close : b.end;
        if (to <= from) continue;
        if (from > cursor) windows.push({ sling_user_id: t.sling_user_id, date, start: cursor, end: from });
        if (to > cursor) cursor = to;
      }
      if (cursor < h.close) windows.push({ sling_user_id: t.sling_user_id, date, start: cursor, end: h.close });
    }
  }
  const set_issues =
    month === "2026-08"
      ? [{
          sling_user_id: 1930006,
          teacher_name: "Morgan Ellis",
          name: "First Monday",
          interval_raw: '"monthly on the first Monday"',
          problem: 'interval "monthly on the first Monday" not understood',
        }]
      : [];
  return {
    target_month: month,
    windows,
    day_ranges,
    hours_set: STUDIO_HOURS.length > 0,
    set_count: 3,
    pending_set_count: 1,
    set_issues,
    warnings: set_issues.length
      ? ["1 availability set from Sling couldn't be interpreted — schedule may miss unavailability; see raw pull file"]
      : [],
  };
}

let EXTERNAL: ExternalShiftRow[] = [
  { sling_shift_id: 990001, shift_date: "2026-08-22", start_time: "05:45", end_time: "06:35", sling_user_id: 1930002, sling_position_id: 101, status: "published" },
];

let hasSlingToken = true;
let hasAnthropicKey = true;
let hasSlingCredentials = false;
let studioConfig = { org_id: 41822, acting_user_id: 1930221, home_location_id: 901 };
const APP_SETTINGS = new Map<string, string>();
const MOCK_DISCOVERED = {
  org_id: 41822,
  acting_user_id: 1930221,
  acting_user_name: "Lead teacher",
  org_name: "Demo Barre Co.",
  locations: [
    { id: 901, name: "Downtown Studio" },
    { id: 902, name: "Uptown Studio" },
  ],
};

const ALGO_VERSIONS: AlgorithmVersion[] = [
  {
    version: 10,
    description: "v10 — Casey off Reform; Saturday opener 8:15",
    rules: {
      teacher_class_blocklist: [
        { sling_user_id: 1930003, class_name: "Reform", reason: "recurring manual swaps" },
      ],
      sat_time_shifts: { "08:00": "08:15" },
    },
    script_file: null,
    created_by: "claude",
    adopted_at: "2026-07-01 10:00:00",
    last_used_month: "2026-08",
    script_archived: false,
    script_missing: false,
    baseline_sha256: null,
    baseline_outdated: false,
    is_active: true,
  },
];
/** Mirrors app_settings.active_algorithm_version (9 = shipped baseline). */
let activeAlgoVersion = 10;

const MOCK_SCRIPT = [
  "# propose.py (dev mock excerpt)",
  "def try_assign(slot_start, slot_end_, week_key_str, cls, wd, st):",
  "    tiers = get_candidates(cls, wd, st)",
  "    for tier in tiers:",
  "        ...",
  "    return None, None",
].join("\n");
const MOCK_DRAFT_SCRIPT = MOCK_SCRIPT.replace(
  "    for tier in tiers:",
  "    if back_to_back_evening(wd, st):\n        return None, None\n    for tier in tiers:",
);

function mockDiffRules(active: Record<string, any>, cand: Record<string, any>) {
  const out: { rule_key: string; identity: string; kind: string; before: unknown; after: unknown }[] = [];
  const keys = new Set([...Object.keys(active), ...Object.keys(cand)]);
  for (const k of keys) {
    const a = JSON.stringify(active[k] ?? null);
    const c = JSON.stringify(cand[k] ?? null);
    if (a === c) continue;
    const kind = active[k] == null ? "added" : cand[k] == null ? "removed" : "changed";
    out.push({ rule_key: k, identity: "", kind, before: active[k] ?? null, after: cand[k] ?? null });
  }
  return out;
}

const REVIEWS = [
  {
    id: 1,
    model: "claude-sonnet-5",
    input_tokens: 4210,
    output_tokens: 680,
    cost_usd: 0.018,
    duration_ms: 3200,
    ran_at: "2026-07-03 09:20:44",
    overall_assessment:
      "Solid coverage. Two structural notes: Priya is consistently at cap while Morgan is under target, and Tuesday evenings lean heavily on newer teachers. Consider rebalancing before publishing.",
    suggestions: [
      { type: "add_rule", confidence: "high", summary: "Cap Priya at 5 classes/week, not 6", rationale: "Priya has hit her max three weeks running; distributing to Morgan evens the load." },
      { type: "tweak_parameter", confidence: "medium", summary: "Raise Morgan's weekly target to 3", rationale: "Morgan is reliably under target and qualified for Classic and Define." },
      { type: "fyi", confidence: "low", summary: "Tuesday 5:30p Reform has thin qualified coverage", rationale: "Only two teachers are qualified for Reform on Tuesday evenings." },
    ],
  },
];

function findProposal(id: number): MockProposal {
  const p = PROPOSALS.find((x) => x.summary.id === id);
  if (!p) throw new Error(`no proposal ${id}`);
  return p;
}

const DEV_BACKUPS: BackupEntry[] = [
  { name: "scheduler-20260929-081502-startup.duckdb", path: "backups/scheduler-20260929-081502-startup.duckdb", size_bytes: 3_407_872, created_at: "2026-09-29 08:15:02", reason: "startup" },
  { name: "scheduler-20260928-164410-prepush.duckdb", path: "backups/scheduler-20260928-164410-prepush.duckdb", size_bytes: 3_395_584, created_at: "2026-09-28 16:44:10", reason: "prepush" },
  { name: "scheduler-20260928-090133-startup.duckdb", path: "backups/scheduler-20260928-090133-startup.duckdb", size_bytes: 3_383_296, created_at: "2026-09-28 09:01:33", reason: "startup" },
];

export function installDevMock() {
  // eslint-disable-next-line no-console
  console.info("[barrekeep] Tauri shell not detected — using dev preview data.");
  mockIPC(async (cmd, payload) => {
    const args = (payload ?? {}) as Record<string, any>;
    switch (cmd) {
      // ---- Tauri plumbing ----
      case "plugin:event|listen":
        return 1;
      case "plugin:event|unlisten":
        return null;
      case "plugin:app|version":
        return "0.1.4";

      // ---- Meta / secrets ----
      case "db_info":
        return { path: "data/scheduler.duckdb", schema_version: 9, teacher_count: TEACHERS.length, position_count: POSITIONS.length };
      case "check_python":
        await sleep(300);
        return { found: true, version: "3.12.4", command: "py -3", path: "C:\\Python312\\python.exe", error: null, min_version: "3.11" };
      case "list_backups":
        return { dir: "C:\\Users\\you\\AppData\\Local\\com.barrekeep.app\\backups", keep: 14, backups: DEV_BACKUPS, last_error: null };
      case "backup_now": {
        await sleep(400);
        const d = new Date();
        const pad = (n: number) => String(n).padStart(2, "0");
        const stamp = `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
        const entry = {
          name: `scheduler-${stamp}-manual.duckdb`,
          path: `backups/scheduler-${stamp}-manual.duckdb`,
          size_bytes: 3_407_872,
          created_at: `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`,
          reason: "manual",
        };
        DEV_BACKUPS.unshift(entry);
        DEV_BACKUPS.splice(14);
        return entry;
      }
      case "open_backups_folder":
        return null;
      case "has_sling_token":
        return hasSlingToken;
      case "set_sling_token":
        hasSlingToken = Boolean(args.value);
        return null;
      case "has_anthropic_key":
        return hasAnthropicKey;
      case "set_anthropic_key":
        hasAnthropicKey = Boolean(args.value);
        return null;
      case "has_sling_credentials":
        return hasSlingCredentials;
      case "get_app_setting":
        return APP_SETTINGS.get(args.key) ?? null;
      case "set_app_setting":
        APP_SETTINGS.set(args.key, args.value);
        return null;
      case "set_sling_credentials":
        hasSlingCredentials = Boolean(args.email);
        return null;
      case "get_studio_config":
        return studioConfig;
      case "set_studio_config":
        studioConfig = { org_id: args.orgId, acting_user_id: args.actingUserId, home_location_id: args.homeLocationId };
        return null;
      case "discover_studio_config":
        await sleep(400);
        return MOCK_DISCOVERED;
      case "auto_detect_studio_config": {
        // Mirrors studio_setup::decide in src-tauri/src/studio_setup.rs.
        await sleep(400);
        const d = MOCK_DISCOVERED;
        const c = studioConfig;
        const complete = c.org_id > 0 && c.acting_user_id > 0 && c.home_location_id > 0;
        if (!complete) {
          if (d.locations.length === 1) {
            studioConfig = { org_id: d.org_id, acting_user_id: d.acting_user_id, home_location_id: d.locations[0].id };
            return { decision: "autosaved", discovered: d, current: studioConfig, reasons: [] };
          }
          return { decision: "ask", discovered: d, current: c, reasons: [] };
        }
        const reasons: string[] = [];
        if (c.org_id !== d.org_id) reasons.push(`The Sling login belongs to a different organization than the configured one (org ${c.org_id}).`);
        if (c.acting_user_id !== d.acting_user_id) reasons.push(`The logged-in Sling user isn't the configured acting user (${c.acting_user_id}).`);
        if (!d.locations.some((l) => l.id === c.home_location_id)) reasons.push(`The logged-in Sling user can't see the configured home location (${c.home_location_id}).`);
        return { decision: reasons.length ? "mismatch" : "ok", discovered: d, current: c, reasons };
      }
      case "open_sling_login_window":
        return null;

      // ---- Roster ----
      case "list_teachers":
        return TEACHERS;
      case "update_teacher_settings": {
        const t = TEACHERS.find((x) => x.sling_user_id === args.slingUserId);
        if (t) {
          t.weekly_target = args.weeklyTarget;
          t.weekly_max = args.weeklyMax;
        }
        return null;
      }
      case "list_positions":
        return POSITIONS;
      case "set_position_active": {
        const p = POSITIONS.find((x) => x.sling_position_id === args.slingPositionId);
        if (p) p.active = args.active;
        return null;
      }
      case "refresh_roster_from_sling":
        await sleep(700);
        return { teachers_active: 6, teachers_deactivated: 0, positions_active: 6, positions_deactivated: 1, qualifications: 27 };
      case "list_qualified_pairs":
        return Object.entries(QUALIFIED).flatMap(([uid, formats]) =>
          formats.map((f) => `${uid}:${positionByName.get(f)!.sling_position_id}`),
        );

      // ---- Proposals ----
      case "list_proposals":
        return PROPOSALS.map(summaryOf).sort((a, b) => b.id - a.id);
      case "get_proposal": {
        const p = findProposal(args.proposalId);
        return {
          summary: summaryOf(p),
          shifts: p.shifts,
          is_stale: STALE.has(p.summary.id),
          last_pulled_at: "2026-07-01T08:00:00",
          last_checked_at: CONFLICT_SEEDS.has(p.summary.id) ? "2026-07-05T12:00:00" : null,
        };
      }
      case "generate_proposal": {
        await sleep(900);
        const id = nextProposalId++;
        const shifts = buildShifts(args.targetMonth);
        const inMonth = PROPOSALS.filter((x) => x.summary.target_month === args.targetMonth);
        for (const other of inMonth) other.summary.is_current = false;
        PROPOSALS.unshift({
          summary: mockSummary({
            id,
            target_month: args.targetMonth,
            algorithm_version: "v3",
            generated_at: "2026-07-05 12:00:00",
            is_current: true,
            name: args.name?.trim() || `Draft ${inMonth.length + 1}`,
            shift_count: shifts.filter((s) => !s.is_dropped).length,
            dropped_count: shifts.filter((s) => s.is_dropped).length,
          }),
          shifts,
        });
        // The push draft is only set when the month has none.
        if (!PUSH_DRAFT.has(args.targetMonth)) PUSH_DRAFT.set(args.targetMonth, id);
        return { proposal_id: id, target_month: args.targetMonth, algorithm_version: "v3", shift_count: shifts.length, dropped_count: 1, stderr_tail: "" };
      }
      case "duplicate_proposal": {
        await sleep(300);
        const src = findProposal(args.proposalId);
        const id = nextProposalId++;
        PROPOSALS.unshift({
          summary: {
            ...src.summary,
            id,
            name: args.name?.trim() || `Copy of ${src.summary.name}`,
            is_current: false,
            archived: false,
            parent_proposal_id: src.summary.id,
            created_from: "duplicate",
            edit_count: 0,
            pushed: false,
          },
          shifts: src.shifts.map((s) => ({ ...s, id: nextShiftId++ })),
        });
        return id;
      }
      case "rename_proposal": {
        const name = String(args.name ?? "").trim();
        if (!name) throw new Error("draft name can't be empty");
        findProposal(args.proposalId).summary.name = name;
        return null;
      }
      case "archive_proposal":
      case "unarchive_proposal": {
        const p = findProposal(args.proposalId);
        if (cmd === "archive_proposal" && PUSH_DRAFT.get(p.summary.target_month) === p.summary.id)
          throw new Error("This is the month's push draft — mark another draft as the push draft before archiving it.");
        p.summary.archived = cmd === "archive_proposal";
        return null;
      }
      case "set_push_candidate": {
        const p = findProposal(args.proposalId);
        if (p.summary.archived) throw new Error("An archived draft can't be the push draft — unarchive it first.");
        PUSH_DRAFT.set(args.targetMonth, args.proposalId);
        return null;
      }
      case "diff_proposals": {
        await sleep(200);
        return mockDiff(findProposal(args.a), findProposal(args.b));
      }
      case "edit_proposal_shift_teacher": {
        for (const p of PROPOSALS) {
          const s = p.shifts.find((x) => x.id === args.proposalShiftId);
          if (!s) continue;
          const t = TEACHERS.find((x) => x.sling_user_id === args.newUserId) ?? null;
          EDITS.push({
            id: nextEditId++,
            proposal_shift_id: s.id,
            shift_date: s.shift_date,
            start_time: s.start_time,
            class_name: s.class_name,
            field: "teacher",
            old_value: s.sling_user_id != null ? String(s.sling_user_id) : null,
            new_value: t ? String(t.sling_user_id) : null,
            old_teacher_name: s.teacher_name,
            new_teacher_name: t?.display_name ?? null,
            old_class_name: null,
            new_class_name: null,
            reason: args.reason ?? null,
            edited_at: "2026-07-05 12:00:00",
            reverted: false,
          });
          s.sling_user_id = t?.sling_user_id ?? null;
          s.teacher_name = t?.display_name ?? null;
          p.summary.edit_count += 1;
        }
        return null;
      }
      case "edit_proposal_shift_position": {
        for (const p of PROPOSALS) {
          const s = p.shifts.find((x) => x.id === args.proposalShiftId);
          if (!s) continue;
          const pos = POSITIONS.find((x) => x.sling_position_id === args.newPositionId);
          if (!pos) throw new Error(`position ${args.newPositionId} not found`);
          const oldPos = POSITIONS.find((x) => x.sling_position_id === s.sling_position_id);
          EDITS.push({
            id: nextEditId++,
            proposal_shift_id: s.id,
            shift_date: s.shift_date,
            start_time: s.start_time,
            class_name: pos.class_name,
            field: "sling_position_id",
            old_value: String(s.sling_position_id),
            new_value: String(pos.sling_position_id),
            old_teacher_name: null,
            new_teacher_name: null,
            old_class_name: oldPos?.class_name ?? null,
            new_class_name: pos.class_name,
            reason: args.reason ?? null,
            edited_at: "2026-07-06 12:00:00",
            reverted: false,
          });
          s.sling_position_id = pos.sling_position_id;
          s.class_name = pos.class_name;
          s.end_time = addMinutes(s.start_time, pos.duration_minutes);
          p.summary.edit_count += 1;
        }
        return null;
      }
      case "list_edits_for_proposal":
        return EDITS;

      // ---- Sling I/O ----
      case "pull_month_from_sling":
        await sleep(900);
        if (!hasSlingToken) throw new Error("sling-401: token expired");
        return {
          target_month: args.targetMonth,
          pulled_at: "2026-07-05T12:00:00",
          user_count: 6,
          qual_count: 27,
          availability_count: 12,
          unavailability_count: 10,
          leave_count: 2,
          leave_day_count: 4,
          set_block_count: 9,
          pending_block_count: 5,
          external_shift_count: 1,
          history_shift_count: 42,
          warnings: ["1 availability set from Sling couldn't be interpreted — schedule may miss unavailability; see raw pull file"],
          raw_pull_file: "raw_pulls/20260705T120000Z-2026-08.json",
        };
      case "list_availability_blocks":
        return BLOCKS;
      case "get_month_availability":
        return mockMonthAvailability(args.targetMonth);
      case "get_studio_hours":
        return STUDIO_HOURS.length > 0 ? { set: true, days: STUDIO_HOURS } : { set: false, days: impliedHours() };
      case "set_studio_hours":
        STUDIO_HOURS = args.days;
        return null;
      case "suggest_studio_hours_from_schedule":
        return impliedHours();
      case "raw_pulls_info":
        return { dir: "raw_pulls", count: 3, keep: 20, latest: "20260705T120000Z-2026-08.json" };
      case "open_raw_pulls_folder":
        return null;
      case "list_external_shifts_for_month":
        return args.targetMonth === "2026-08" ? EXTERNAL : [];
      case "import_external_shift": {
        const ext = EXTERNAL.find((x) => x.sling_shift_id === args.slingShiftId);
        if (ext) {
          const p = findProposal(args.proposalId);
          const t = TEACHERS.find((x) => x.sling_user_id === ext.sling_user_id) ?? null;
          p.shifts.push({
            id: nextShiftId++,
            shift_date: ext.shift_date,
            start_time: ext.start_time,
            end_time: ext.end_time,
            class_name: POSITIONS.find((x) => x.sling_position_id === ext.sling_position_id)?.class_name ?? "?",
            sling_position_id: ext.sling_position_id,
            teacher_name: t?.display_name ?? null,
            sling_user_id: ext.sling_user_id,
            generation_reason: "imported from Sling",
            flag: null,
            is_coteach: false,
            coteach_label: null,
            is_dropped: false,
          });
          EXTERNAL = EXTERNAL.filter((x) => x.sling_shift_id !== args.slingShiftId);
        }
        return null;
      }
      case "refresh_availability_from_sling":
        await sleep(900);
        if (!hasSlingToken) throw new Error("sling-401: token expired");
        for (const x of PROPOSALS) STALE.add(x.summary.id);
        return {
          months: [...new Set(PROPOSALS.map((x) => x.summary.target_month))].sort().map((m) => ({
            target_month: m,
            availability_count: 12,
            unavailability_count: 10,
            leave_count: 2,
            leave_day_count: 4,
            external_shift_count: 1,
          })),
          roster: { teachers_active: 6, teachers_deactivated: 0, positions_active: 6, positions_deactivated: 1, qualifications: 27 },
          refreshed_at: "2026-07-05T12:00:00Z",
          warnings: [],
          raw_pull_file: "raw_pulls/20260705T120000Z-refresh-2026-08_2026-09.json",
        };
      case "check_draft_conflicts":
        return mockCheckConflicts(args.proposalId);
      case "push_sync_preview":
        ensurePushDraft(args.proposalId);
        await sleep(500);
        return mockSyncPlan(args.proposalId, "push", args.cleanup);
      case "push_sync_execute":
        ensurePushDraft(args.proposalId);
        return mockSyncExecute(args.proposalId, "push", args.cleanup, args.planKey);
      case "remove_draft_from_sling_preview":
        await sleep(400);
        return mockSyncPlan(args.proposalId, "remove", false);
      case "remove_draft_from_sling_execute":
        return mockSyncExecute(args.proposalId, "remove", false, args.planKey);

      // ---- Claude review ----
      case "review_proposal":
        await sleep(1200);
        return { run_id: 1, suggestions: REVIEWS[0].suggestions, overall_assessment: REVIEWS[0].overall_assessment, model: REVIEWS[0].model, input_tokens: 4210, output_tokens: 680, cache_read_input_tokens: 0, cost_usd: 0.018, duration_ms: 3200 };
      case "list_reviews_for_proposal":
        return REVIEWS;

      // ---- Claude proposal editor ----
      case "claude_edit_proposal": {
        await sleep(1200);
        const p = findProposal(args.proposalId);
        const assigned = p.shifts.filter((s) => !s.is_dropped && s.sling_user_id != null);
        const a = assigned[4] ?? assigned[0];
        const b = assigned[9] ?? assigned[1];
        const other = TEACHERS.find((t) => t.sling_user_id !== a.sling_user_id)!;
        return {
          run_id: nextRunId++,
          summary:
            "Rebalanced two slots per the instruction. Casey keeps getting swapped off Reform — proposed a standing rule.",
          edits: [
            {
              proposal_shift_id: a.id,
              action: "reassign",
              new_user_id: other.sling_user_id,
              new_class_name: null,
              rationale: `${other.display_name} is under target this week`,
              valid: true,
              validation_note: null,
            },
            {
              proposal_shift_id: b.id,
              action: "change_format",
              new_user_id: null,
              new_class_name: b.class_name === "Classic" ? "Empower" : "Classic",
              rationale: "thin coverage for the original format",
              valid: true,
              validation_note: null,
            },
            {
              proposal_shift_id: 999999,
              action: "unassign",
              new_user_id: null,
              new_class_name: null,
              rationale: "a hallucinated slot, for the invalid-row UI",
              valid: false,
              validation_note: "that slot is not in this proposal",
            },
          ],
          ruleset_proposal: {
            description: "v-next — Casey never teaches Reform",
            rules: {
              teacher_class_blocklist: [
                { sling_user_id: 1930003, class_name: "Reform", reason: "swapped off 3 months running" },
              ],
              sat_time_shifts: { "08:00": "08:15" },
            },
          },
          needs_code_change: null,
          model: APP_SETTINGS.get("claude_model") ?? "claude-opus-5-5",
          cost_usd: 0.11,
          duration_ms: 4200,
        };
      }
      case "claude_draft_code_change":
        await sleep(1500);
        return {
          run_id: 43,
          description: "v-next — never assign back-to-back evening classes",
          script: MOCK_DRAFT_SCRIPT,
          diff:
            "--- active/propose.py\n+++ draft/propose.py\n@@ -2,4 +2,6 @@\n" +
            " def try_assign(slot_start, slot_end_, week_key_str, cls, wd, st):\n" +
            "     tiers = get_candidates(cls, wd, st)\n" +
            "+    if back_to_back_evening(wd, st):\n+        return None, None\n" +
            "     for tier in tiers:\n         ...\n",
          edit_count: 1,
          rules: ALGO_VERSIONS.find((v) => v.version === activeAlgoVersion)?.rules ?? {},
          model: APP_SETTINGS.get("claude_model") ?? "claude-opus-5-5",
          cost_usd: 0.09,
          duration_ms: 9100,
        };
      case "preview_algorithm_candidate": {
        await sleep(900);
        const active = ALGO_VERSIONS.find((v) => v.version === activeAlgoVersion);
        const rulesDiff = mockDiffRules((active?.rules as Record<string, any>) ?? {}, args.rules ?? {});
        const isCode = args.scriptContent != null;
        const changes = [
          { date: "2026-08-04", weekday: "Tue", start: "17:30", kind: "changed", class_before: "Reform", class_after: "Reform", teacher_before: "Casey Diaz", teacher_after: "Priya Shah", expected: false },
          { date: "2026-08-11", weekday: "Tue", start: "17:30", kind: "changed", class_before: "Reform", class_after: "Reform", teacher_before: "Casey Diaz", teacher_after: "Kayla Moore", expected: false },
        ];
        return {
          active_version: activeAlgoVersion,
          rules_diff: rulesDiff,
          script_diff: isCode
            ? "--- active/propose.py\n+++ candidate/propose.py\n@@ -3,2 +3,4 @@\n     tiers = get_candidates(cls, wd, st)\n+    if back_to_back_evening(wd, st):\n+        return None, None\n     for tier in tiers:\n"
            : null,
          validation: {
            // Code drafts demo the "adopt anyway" confirm path.
            status: isCode ? "needs_confirm" : "pass",
            error: null,
            reasons: isCode ? ["31 of 114 assignments change (27% — more than 25%)"] : [],
            month: "2026-08",
            slot_count: 114,
            candidate_slot_count: 114,
            changed_count: isCode ? 31 : changes.length,
            added_count: 0,
            removed_count: 0,
            unexpected_count: 0,
            changed_pct: isCode ? 0.27 : changes.length / 114,
            changes,
          },
        };
      }
      case "list_algorithm_versions":
        return [...ALGO_VERSIONS]
          .map((v) => ({ ...v, is_active: v.version === activeAlgoVersion }))
          .sort((x, y) => y.version - x.version);
      case "adopt_algorithm_version": {
        const version = Math.max(9, ...ALGO_VERSIONS.map((v) => v.version)) + 1;
        const active = ALGO_VERSIONS.find((v) => v.version === activeAlgoVersion);
        ALGO_VERSIONS.push({
          version,
          description: args.description,
          rules: args.rules ?? {},
          // Rules-only adoptions keep the active version's script.
          script_file: args.scriptContent ? `propose_v${version}.py` : active?.script_file ?? null,
          created_by: args.claudeRunId != null ? "claude" : "user",
          adopted_at: "2026-07-06 12:00:00",
          last_used_month: null,
          script_archived: false,
          script_missing: false,
          baseline_sha256: null,
          baseline_outdated: false,
          is_active: true,
        });
        activeAlgoVersion = version;
        return version;
      }
      case "set_active_algorithm_version": {
        if (args.version !== 9 && !ALGO_VERSIONS.some((v) => v.version === args.version))
          throw new Error(`version v${args.version} does not exist`);
        activeAlgoVersion = args.version;
        return null;
      }
      case "delete_algorithm_script": {
        const v = ALGO_VERSIONS.find((x) => x.version === args.version);
        if (!v) throw new Error(`version v${args.version} not found`);
        if (v.version === activeAlgoVersion)
          throw new Error("cannot delete the active version's script");
        v.script_missing = true;
        return null;
      }

      default:
        throw new Error(`devMock: unhandled command ${cmd}`);
    }
  });
}

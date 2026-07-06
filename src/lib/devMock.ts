// Browser-only preview data. Loaded exclusively from main.tsx when running
// `npm run dev` outside the Tauri shell (import.meta.env.DEV guard), so none
// of this ships in a production build. Mirrors the placeholder demo roster
// from the design-system UI kit / seed.rs — real data comes from Sling.

import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  Teacher,
  Position,
  ProposalSummary,
  ProposalShiftRow,
  EditRow,
  AvailabilityBlock,
  ExternalShiftRow,
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

function buildShifts(ym: string): ProposalShiftRow[] {
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
      const teacher = TEACHERS[(d + i) % TEACHERS.length];
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

const PROPOSALS: MockProposal[] = [
  {
    summary: { id: 7, target_month: "2026-08", algorithm_version: "v3", generated_at: "2026-07-03 09:14:02", is_current: true, shift_count: 0, dropped_count: 1, edit_count: 0 },
    shifts: buildShifts("2026-08"),
  },
  {
    summary: { id: 6, target_month: "2026-07", algorithm_version: "v3", generated_at: "2026-06-24 08:02:11", is_current: true, shift_count: 0, dropped_count: 0, edit_count: 5 },
    shifts: buildShifts("2026-07"),
  },
  {
    summary: { id: 5, target_month: "2026-06", algorithm_version: "v2", generated_at: "2026-05-26 10:41:37", is_current: true, shift_count: 0, dropped_count: 2, edit_count: 2 },
    shifts: buildShifts("2026-06"),
  },
];
for (const p of PROPOSALS) p.summary.shift_count = p.shifts.filter((s) => !s.is_dropped).length;

const EDITS: EditRow[] = [];
let nextEditId = 1;
let nextProposalId = 8;

const BLOCKS: AvailabilityBlock[] = [
  { sling_user_id: 1930004, source: "leave", starts_at: "2026-08-20T08:00:00", ends_at: "2026-08-20T12:00:00" },
];

let EXTERNAL: ExternalShiftRow[] = [
  { sling_shift_id: 990001, shift_date: "2026-08-22", start_time: "05:45", end_time: "06:35", sling_user_id: 1930002, sling_position_id: 101, status: "published" },
];

let hasSlingToken = true;
let hasAnthropicKey = true;
let hasSlingCredentials = false;
let studioConfig = { org_id: 41822, acting_user_id: 1930221, home_location_id: 901 };
const APP_SETTINGS = new Map<string, string>();

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
        return {
          org_id: 41822,
          acting_user_id: 1930221,
          acting_user_name: "Lead teacher",
          locations: [
            { id: 901, name: "Downtown Studio" },
            { id: 902, name: "Uptown Studio" },
          ],
        };
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
        return PROPOSALS.map((p) => p.summary).sort((a, b) => b.id - a.id);
      case "get_proposal": {
        const p = findProposal(args.proposalId);
        return { summary: p.summary, shifts: p.shifts, is_stale: p.summary.id === 6, last_pulled_at: "2026-07-01T08:00:00" };
      }
      case "generate_proposal": {
        await sleep(900);
        const id = nextProposalId++;
        const shifts = buildShifts(args.targetMonth);
        for (const other of PROPOSALS.filter((x) => x.summary.target_month === args.targetMonth)) {
          other.summary.is_current = false;
        }
        PROPOSALS.unshift({
          summary: {
            id,
            target_month: args.targetMonth,
            algorithm_version: "v3",
            generated_at: "2026-07-05 12:00:00",
            is_current: true,
            shift_count: shifts.filter((s) => !s.is_dropped).length,
            dropped_count: shifts.filter((s) => s.is_dropped).length,
            edit_count: 0,
          },
          shifts,
        });
        return { proposal_id: id, target_month: args.targetMonth, algorithm_version: "v3", shift_count: shifts.length, dropped_count: 1, stderr_tail: "" };
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
      case "list_edits_for_proposal":
        return EDITS;

      // ---- Sling I/O ----
      case "pull_month_from_sling":
        await sleep(900);
        if (!hasSlingToken) throw new Error("sling-401: token expired");
        return { target_month: args.targetMonth, pulled_at: "2026-07-05T12:00:00", user_count: 6, qual_count: 27, availability_count: 3, external_shift_count: 1, history_shift_count: 42 };
      case "list_availability_blocks":
        return BLOCKS;
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
      case "push_proposal_dry_run": {
        await sleep(500);
        const p = findProposal(args.proposalId);
        const items = p.shifts.filter((s) => !s.is_dropped && s.teacher_name);
        return {
          total: items.length,
          skipped_count: 2,
          to_create: items.slice(0, 40).map((s) => ({ date: s.shift_date, start: s.start_time, end: s.end_time, class_name: s.class_name, teacher_name: s.teacher_name! })),
        };
      }
      case "push_proposal_execute":
        await sleep(1800);
        return { push_id: 1, created: 38, failed: 0, skipped: 2 };

      // ---- Claude review ----
      case "review_proposal":
        await sleep(1200);
        return { run_id: 1, suggestions: REVIEWS[0].suggestions, overall_assessment: REVIEWS[0].overall_assessment, model: REVIEWS[0].model, input_tokens: 4210, output_tokens: 680, cache_read_input_tokens: 0, cost_usd: 0.018, duration_ms: 3200 };
      case "list_reviews_for_proposal":
        return REVIEWS;

      default:
        throw new Error(`devMock: unhandled command ${cmd}`);
    }
  });
}

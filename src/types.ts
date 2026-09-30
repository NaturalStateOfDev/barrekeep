// Type definitions mirroring the DuckDB schema in docs/data-model.md.
// Update both when the schema changes (see .claude/skills/schema-change/).

export interface Teacher {
  sling_user_id: number;
  display_name: string;
  weekly_target: number;
  weekly_max: number;
  is_lead: boolean;
  ranking_weight: number;
  variety_multiplier: number;
  active: boolean;
  notes: string | null;
  locations: string | null;
}

export interface StudioConfig {
  org_id: number;
  acting_user_id: number;
  home_location_id: number;
}

export interface Position {
  sling_position_id: number;
  class_name: string;
  duration_minutes: number;
  is_special: boolean;
  active: boolean;
}

export interface DbInfo {
  path: string;
  schema_version: number;
  teacher_count: number;
  position_count: number;
}

export interface PythonStatus {
  found: boolean;
  version: string | null;
  /** How it's invoked, e.g. "py -3". */
  command: string | null;
  /** sys.executable of the resolved interpreter. */
  path: string | null;
  error: string | null;
  min_version: string;
}

export interface BackupEntry {
  name: string;
  path: string;
  size_bytes: number;
  /** Local time, "YYYY-MM-DD HH:MM:SS". */
  created_at: string;
  reason: string;
}

export interface BackupsInfo {
  dir: string;
  keep: number;
  backups: BackupEntry[];
  /** Most recent backup failure this session, if the last attempt failed. */
  last_error: string | null;
}

export interface GenerateResult {
  proposal_id: number;
  target_month: string;
  algorithm_version: string;
  shift_count: number;
  dropped_count: number;
  stderr_tail: string;
}

export interface ProposalSummary {
  id: number;
  target_month: string;
  algorithm_version: string;
  generated_at: string;
  /** Newest generated draft of the month (legacy; see is_push_candidate). */
  is_current: boolean;
  shift_count: number;
  dropped_count: number;
  edit_count: number;
  /** Draft metadata (migration 0012). */
  name: string;
  archived: boolean;
  parent_proposal_id: number | null;
  created_from: "generate" | "duplicate" | string;
  /** The month's push draft — the only draft Push sends to Sling. */
  is_push_candidate: boolean;
  /** At least one push to Sling is on record for this draft. */
  pushed: boolean;
  /** Live Sling shifts this draft owns (push tracking, migration 0013). */
  sling_shift_count: number;
}

export interface EditRow {
  id: number;
  proposal_shift_id: number;
  shift_date: string;
  start_time: string;
  class_name: string;
  field: string;
  old_value: string | null;
  new_value: string | null;
  old_teacher_name: string | null;
  new_teacher_name: string | null;
  old_class_name: string | null;
  new_class_name: string | null;
  reason: string | null;
  edited_at: string;
  reverted: boolean;
}

export type SuggestionKind = "add_rule" | "tweak_parameter" | "fyi";

export interface ReviewSuggestion {
  type: SuggestionKind;
  summary: string;
  rationale: string;
  confidence: "high" | "medium" | "low";
}

export interface ReviewResult {
  run_id: number;
  suggestions: ReviewSuggestion[];
  overall_assessment: string;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cache_read_input_tokens: number;
  cost_usd: number;
  duration_ms: number;
}

export interface ReviewRunSummary {
  id: number;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  duration_ms: number;
  ran_at: string;
  suggestions: ReviewSuggestion[];
  overall_assessment: string;
}

export interface ProposalShiftRow {
  id: number;
  shift_date: string;
  start_time: string;
  end_time: string;
  class_name: string;
  sling_position_id: number;
  teacher_name: string | null;
  sling_user_id: number | null;
  generation_reason: string;
  flag: string | null;
  is_coteach: boolean;
  coteach_label: string | null;
  is_dropped: boolean;
}

export interface ProposalDetail {
  summary: ProposalSummary;
  shifts: ProposalShiftRow[];
  /** The month's latest pull/refresh is newer than this draft's generation
   *  AND its last conflict check. */
  is_stale: boolean;
  last_pulled_at: string | null;
  /** Last time check_draft_conflicts validated this draft. */
  last_checked_at: string | null;
}

export interface PullResult {
  target_month: string;
  pulled_at: string;
  user_count: number;
  qual_count: number;
  availability_count: number;
  external_shift_count: number;
  history_shift_count: number;
}

export interface AvailabilityBlock {
  sling_user_id: number;
  source: string; // 'leave' | 'availability'
  starts_at: string; // ISO timestamp
  ends_at: string;
}

export type DraftDiffKind = "teacher" | "format" | "format_teacher" | "only_a" | "only_b";

export interface DraftSlotDiff {
  date: string;
  weekday: string;
  start: string;
  class_a: string | null;
  class_b: string | null;
  teacher_a: string | null;
  teacher_b: string | null;
  kind: DraftDiffKind;
}

export interface TeacherStats {
  classes: number;
  /** Distinct weekday+start-time slots taught this month (lower = steadier). */
  distinct_slots: number;
  /** Most common slot, e.g. "Tue 08:45". */
  top_slot: string;
  top_slot_count: number;
  top_slot_share: number;
}

export interface TeacherConsistency {
  sling_user_id: number;
  name: string;
  a: TeacherStats | null;
  b: TeacherStats | null;
}

export interface DraftTotals {
  classes: number;
  distinct_slots: number;
  classes_per_slot: number;
}

export interface ProposalDiff {
  target_month: string;
  a_id: number;
  b_id: number;
  a_name: string;
  b_name: string;
  changes: DraftSlotDiff[];
  teachers: TeacherConsistency[];
  totals_a: DraftTotals;
  totals_b: DraftTotals;
}

export type SyncActionKind = "baseline" | "adopt" | "skip" | "cleanup" | "delete" | "update" | "create";

export interface ShiftView {
  date: string;
  start: string;
  end: string;
  class_name: string;
  teacher_name: string;
}

export interface SyncAction {
  kind: SyncActionKind;
  proposal_shift_id: number;
  sling_shift_id: number | null;
  /** What's in Sling now (update/delete/cleanup) or was last pushed. */
  before: ShiftView | null;
  /** What the draft wants (create/update/adopt). */
  after: ShiftView | null;
  reason: string;
  /** For cleanup/adopt: the other draft that owned the shift. */
  from_draft: string | null;
  /** For skips: the push_results outcome recorded on execute, if any. */
  skip_outcome: string | null;
}

export interface CleanupOffer {
  proposal_id: number;
  draft_name: string;
  /** Planning, unmodified shifts that can be removed. */
  removable: number;
  /** Shifts that will be left alone (published / edited in Sling / legacy). */
  blocked: number;
}

export interface SyncPreview {
  mode: "push" | "remove";
  proposal_id: number;
  draft_name: string;
  target_month: string;
  actions: SyncAction[];
  unchanged: number;
  cleanup: boolean;
  cleanup_offers: CleanupOffer[];
  /** Hand back to execute; it refuses if the plan changed since. */
  plan_key: string;
}

export interface SyncSummary {
  push_id: number;
  created: number;
  updated: number;
  deleted: number;
  adopted: number;
  skipped: number;
  failed: number;
  aborted: boolean;
  /** Set when the pre-sync database backup failed (the sync still ran). */
  backup_warning: string | null;
}

export interface SyncProgress {
  total: number;
  done: number;
  created: number;
  updated: number;
  deleted: number;
  failed: number;
  last_label: string;
  last_outcome: string;
}

export type DraftConflictKind =
  | "blocked"
  | "leave"
  | "teacher_inactive"
  | "not_qualified"
  | "over_cap"
  | "unassigned";

export interface DraftConflict {
  proposal_shift_id: number;
  shift_date: string;
  start_time: string;
  end_time: string;
  class_name: string;
  sling_user_id: number | null;
  teacher_name: string | null;
  kind: DraftConflictKind;
  message: string;
}

export interface MonthRefresh {
  target_month: string;
  availability_count: number;
  external_shift_count: number;
}

export interface AvailabilityRefreshResult {
  months: MonthRefresh[];
  roster: RosterSyncSummary;
  refreshed_at: string;
}

export interface RosterSyncSummary {
  teachers_active: number;
  teachers_deactivated: number;
  positions_active: number;
  positions_deactivated: number;
  qualifications: number;
}

export interface DiscoveredLocation {
  id: number;
  name: string;
}

export interface DiscoveredStudio {
  org_id: number;
  acting_user_id: number;
  acting_user_name: string;
  locations: DiscoveredLocation[];
}

export interface ExternalShiftRow {
  sling_shift_id: number;
  shift_date: string;
  start_time: string;
  end_time: string;
  sling_user_id: number | null;
  sling_position_id: number;
  status: string;
}

// ---- Claude proposal editor (spec 2026-07-06) ----

export type EditAction = "reassign" | "unassign" | "change_format";

export interface ProposedEdit {
  proposal_shift_id: number;
  action: EditAction;
  new_user_id?: number | null;
  new_class_name?: string | null;
  rationale: string;
  valid: boolean;
  validation_note?: string | null;
}

export interface RulesetProposal {
  description: string;
  rules: Record<string, unknown>;
}

export interface ClaudeEditResult {
  run_id: number;
  summary: string;
  edits: ProposedEdit[];
  ruleset_proposal: RulesetProposal | null;
  needs_code_change: { rationale: string } | null;
  model: string;
  cost_usd: number;
  duration_ms: number;
}

export interface AlgorithmVersion {
  version: number;
  description: string;
  rules: Record<string, unknown>;
  script_file: string | null;
  created_by: string;
  adopted_at: string;
  last_used_month: string | null;
  script_archived: boolean;
  script_missing: boolean;
  /** sha256 of the shipped propose.py this version was adopted on. */
  baseline_sha256: string | null;
  /** Custom script built on a different shipped baseline than installed now. */
  baseline_outdated: boolean;
  is_active: boolean;
}

export interface CodeDraft {
  run_id: number;
  description: string;
  /** Full resulting script (active script + Claude's edits). */
  script: string;
  /** Unified diff, active script → draft. */
  diff: string;
  edit_count: number;
  /** Active rules at draft time, carried into the code version. */
  rules: Record<string, unknown>;
  model: string;
  cost_usd: number;
  duration_ms: number;
}

export interface RuleDiffEntry {
  rule_key: string;
  identity: string;
  kind: "added" | "removed" | "changed";
  before: unknown;
  after: unknown;
}

export interface SlotChange {
  date: string;
  weekday: string;
  start: string;
  kind: "changed" | "added" | "removed";
  class_before: string | null;
  class_after: string | null;
  teacher_before: string | null;
  teacher_after: string | null;
  expected: boolean;
}

export type ValidationStatus = "pass" | "needs_confirm" | "error";

export interface CandidateValidation {
  status: ValidationStatus;
  error: string | null;
  reasons: string[];
  month: string;
  slot_count: number;
  candidate_slot_count: number;
  changed_count: number;
  added_count: number;
  removed_count: number;
  unexpected_count: number;
  changed_pct: number;
  changes: SlotChange[];
}

export interface CandidatePreview {
  active_version: number;
  rules_diff: RuleDiffEntry[];
  script_diff: string | null;
  validation: CandidateValidation;
}


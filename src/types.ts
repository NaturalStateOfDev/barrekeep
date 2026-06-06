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

export interface SlingCandidate {
  sling_user_id: number;
  display_name: string;
  active: boolean;
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
  is_current: boolean;
  shift_count: number;
  dropped_count: number;
  edit_count: number;
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
  is_stale: boolean;
  last_pulled_at: string | null;
}

export interface NewUserSummary {
  sling_user_id: number;
  display_name: string;
  active: boolean;
  locations: string | null;
}

export interface PullResult {
  target_month: string;
  pulled_at: string;
  user_count: number;
  qual_count: number;
  availability_count: number;
  external_shift_count: number;
  history_shift_count: number;
  new_users: NewUserSummary[];
}

export interface AvailabilityBlock {
  sling_user_id: number;
  source: string; // 'leave' | 'availability'
  starts_at: string; // ISO timestamp
  ends_at: string;
}

export interface PushPreviewItem {
  date: string;
  start: string;
  end: string;
  class_name: string;
  teacher_name: string;
}

export interface PushPreview {
  total: number;
  skipped_count: number;
  to_create: PushPreviewItem[];
}

export interface PushSummary {
  push_id: number;
  created: number;
  failed: number;
  skipped: number;
}

export interface PushProgress {
  total: number;
  done: number;
  created: number;
  failed: number;
  skipped: number;
  last_label: string;
  last_outcome: string;
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

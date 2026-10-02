// Thin wrapper around Tauri IPC. One function per Rust command.
// Keep call signatures here in sync with src-tauri/src/commands.rs.

import { invoke } from "@tauri-apps/api/core";
import { notifySlingTokenSet } from "./studioSetup";
import type {
  AlgorithmVersion,
  CandidatePreview,
  ClaudeEditResult,
  CodeDraft,
  Teacher,
  StudioConfig,
  Position,
  DbInfo,
  GenerateResult,
  ProposalSummary,
  ProposalDetail,
  EditRow,
  ReviewResult,
  ReviewRunSummary,
  PullResult,
  AvailabilityBlock,
  ExternalShiftRow,
  SyncPreview,
  SyncSummary,
  DraftConflict,
  AvailabilityRefreshResult,
  DiscoveredStudio,
  StudioDetectOutcome,
  RosterSyncSummary,
  PythonStatus,
  BackupsInfo,
  BackupEntry,
  ProposalDiff,
  MonthAvailability,
  StudioHours,
  DayHours,
  RawPullsInfo,
} from "../types";

export const api = {
  dbInfo: () => invoke<DbInfo>("db_info"),
  checkPython: () => invoke<PythonStatus>("check_python"),
  listBackups: () => invoke<BackupsInfo>("list_backups"),
  backupNow: () => invoke<BackupEntry>("backup_now"),
  openBackupsFolder: () => invoke<void>("open_backups_folder"),
  listTeachers: () => invoke<Teacher[]>("list_teachers"),
  updateTeacherSettings: (slingUserId: number, weeklyTarget: number, weeklyMax: number) =>
    invoke<void>("update_teacher_settings", { slingUserId, weeklyTarget, weeklyMax }),
  listPositions: () => invoke<Position[]>("list_positions"),
  setPositionActive: (slingPositionId: number, active: boolean) =>
    invoke<void>("set_position_active", { slingPositionId, active }),
  refreshRosterFromSling: () => invoke<RosterSyncSummary>("refresh_roster_from_sling"),
  listQualifiedPairs: () => invoke<string[]>("list_qualified_pairs"),
  generateProposal: (targetMonth: string, name?: string) =>
    invoke<GenerateResult>("generate_proposal", { targetMonth, name: name ?? null }),
  duplicateProposal: (proposalId: number, name?: string) =>
    invoke<number>("duplicate_proposal", { proposalId, name: name ?? null }),
  renameProposal: (proposalId: number, name: string) =>
    invoke<void>("rename_proposal", { proposalId, name }),
  archiveProposal: (proposalId: number) => invoke<void>("archive_proposal", { proposalId }),
  unarchiveProposal: (proposalId: number) => invoke<void>("unarchive_proposal", { proposalId }),
  setPushCandidate: (targetMonth: string, proposalId: number) =>
    invoke<void>("set_push_candidate", { targetMonth, proposalId }),
  diffProposals: (a: number, b: number) => invoke<ProposalDiff>("diff_proposals", { a, b }),
  listProposals: () => invoke<ProposalSummary[]>("list_proposals"),
  getProposal: (proposalId: number) =>
    invoke<ProposalDetail>("get_proposal", { proposalId }),
  editProposalShiftTeacher: (
    proposalShiftId: number,
    newUserId: number | null,
    reason: string | null,
  ) =>
    invoke<void>("edit_proposal_shift_teacher", {
      proposalShiftId,
      newUserId,
      reason,
    }),
  editProposalShiftPosition: (
    proposalShiftId: number,
    newPositionId: number,
    reason: string | null,
  ) =>
    invoke<void>("edit_proposal_shift_position", {
      proposalShiftId,
      newPositionId,
      reason,
    }),
  listEditsForProposal: (proposalId: number) =>
    invoke<EditRow[]>("list_edits_for_proposal", { proposalId }),
  setAnthropicKey: (value: string) =>
    invoke<void>("set_anthropic_key", { value }),
  hasAnthropicKey: () => invoke<boolean>("has_anthropic_key"),
  getAppSetting: (key: string) =>
    invoke<string | null>("get_app_setting", { key }),
  setAppSetting: (key: string, value: string) =>
    invoke<void>("set_app_setting", { key, value }),
  setSlingToken: async (value: string) => {
    await invoke<void>("set_sling_token", { value });
    // A pasted token is a login too: let StudioSetup re-run detection.
    if (value.trim()) notifySlingTokenSet();
  },
  hasSlingToken: () => invoke<boolean>("has_sling_token"),
  setSlingCredentials: (email: string, password: string) =>
    invoke<void>("set_sling_credentials", { email, password }),
  hasSlingCredentials: () => invoke<boolean>("has_sling_credentials"),
  getStudioConfig: () => invoke<StudioConfig>("get_studio_config"),
  setStudioConfig: (orgId: number, actingUserId: number, homeLocationId: number) =>
    invoke<void>("set_studio_config", { orgId, actingUserId, homeLocationId }),
  discoverStudioConfig: () => invoke<DiscoveredStudio>("discover_studio_config"),
  autoDetectStudioConfig: () => invoke<StudioDetectOutcome>("auto_detect_studio_config"),
  openSlingLoginWindow: () => invoke<void>("open_sling_login_window"),
  reviewProposal: (proposalId: number) =>
    invoke<ReviewResult>("review_proposal", { proposalId }),
  listReviewsForProposal: (proposalId: number) =>
    invoke<ReviewRunSummary[]>("list_reviews_for_proposal", { proposalId }),
  pullMonthFromSling: (targetMonth: string) =>
    invoke<PullResult>("pull_month_from_sling", { targetMonth }),
  importExternalShift: (slingShiftId: number, proposalId: number) =>
    invoke<void>("import_external_shift", { slingShiftId, proposalId }),
  listAvailabilityBlocks: (targetMonth: string) =>
    invoke<AvailabilityBlock[]>("list_availability_blocks", { targetMonth }),
  listExternalShiftsForMonth: (targetMonth: string) =>
    invoke<ExternalShiftRow[]>("list_external_shifts_for_month", { targetMonth }),
  /** The month's computed availability: per-teacher available windows, each
   *  date's schedulable span, and any uninterpreted Sling availability sets.
   *  Recomputed (and stored) on every call. */
  getMonthAvailability: (targetMonth: string) =>
    invoke<MonthAvailability>("get_month_availability", { targetMonth }),
  getStudioHours: () => invoke<StudioHours>("get_studio_hours"),
  /** Save studio hours (an empty list clears them) and recompute windows. */
  setStudioHours: (days: DayHours[]) => invoke<void>("set_studio_hours", { days }),
  suggestStudioHours: () => invoke<DayHours[]>("suggest_studio_hours_from_schedule"),
  rawPullsInfo: () => invoke<RawPullsInfo>("raw_pulls_info"),
  openRawPullsFolder: () => invoke<void>("open_raw_pulls_folder"),
  /** Re-pull unavailability/leave + roster for the current and future months
   *  without regenerating any draft. */
  refreshAvailabilityFromSling: () =>
    invoke<AvailabilityRefreshResult>("refresh_availability_from_sling"),
  /** Re-validate a draft against the latest pulled data (marks it checked). */
  checkDraftConflicts: (proposalId: number) =>
    invoke<DraftConflict[]>("check_draft_conflicts", { proposalId }),
  /** Incremental push plan for the month's push draft. */
  pushSyncPreview: (proposalId: number, cleanup: boolean) =>
    invoke<SyncPreview>("push_sync_preview", { proposalId, cleanup }),
  pushSyncExecute: (proposalId: number, cleanup: boolean, planKey: string) =>
    invoke<SyncSummary>("push_sync_execute", { proposalId, cleanup, planKey }),
  /** Plan removing a draft's (planning, unmodified) shifts from Sling. */
  removeDraftFromSlingPreview: (proposalId: number) =>
    invoke<SyncPreview>("remove_draft_from_sling_preview", { proposalId }),
  removeDraftFromSlingExecute: (proposalId: number, planKey: string) =>
    invoke<SyncSummary>("remove_draft_from_sling_execute", { proposalId, planKey }),
  /** groupRunId: the first run of a prompt sent to several drafts — links
   *  this run to it (claude_run_targets). */
  claudeEditProposal: (proposalId: number, instruction: string, groupRunId?: number) =>
    invoke<ClaudeEditResult>("claude_edit_proposal", {
      proposalId,
      instruction,
      groupRunId: groupRunId ?? null,
    }),
  claudeDraftCodeChange: (
    proposalId: number,
    instruction: string,
    rationale: string,
  ) =>
    invoke<CodeDraft>("claude_draft_code_change", {
      proposalId,
      instruction,
      rationale,
    }),
  previewAlgorithmCandidate: (rules: Record<string, unknown>, scriptContent?: string) =>
    invoke<CandidatePreview>("preview_algorithm_candidate", {
      rules,
      scriptContent: scriptContent ?? null,
    }),
  setActiveAlgorithmVersion: (version: number) =>
    invoke<void>("set_active_algorithm_version", { version }),
  listAlgorithmVersions: () =>
    invoke<AlgorithmVersion[]>("list_algorithm_versions"),
  adoptAlgorithmVersion: (
    description: string,
    rules: Record<string, unknown>,
    scriptContent?: string,
    claudeRunId?: number,
  ) =>
    invoke<number>("adopt_algorithm_version", {
      description,
      rules,
      scriptContent: scriptContent ?? null,
      claudeRunId: claudeRunId ?? null,
    }),
  deleteAlgorithmScript: (version: number) =>
    invoke<void>("delete_algorithm_script", { version }),
};

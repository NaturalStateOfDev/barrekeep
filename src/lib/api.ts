// Thin wrapper around Tauri IPC. One function per Rust command.
// Keep call signatures here in sync with src-tauri/src/commands.rs.

import { invoke } from "@tauri-apps/api/core";
import type {
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
  PushPreview,
  PushSummary,
  DiscoveredStudio,
  RosterSyncSummary,
} from "../types";

export const api = {
  dbInfo: () => invoke<DbInfo>("db_info"),
  listTeachers: () => invoke<Teacher[]>("list_teachers"),
  updateTeacherSettings: (slingUserId: number, weeklyTarget: number, weeklyMax: number) =>
    invoke<void>("update_teacher_settings", { slingUserId, weeklyTarget, weeklyMax }),
  listPositions: () => invoke<Position[]>("list_positions"),
  setPositionActive: (slingPositionId: number, active: boolean) =>
    invoke<void>("set_position_active", { slingPositionId, active }),
  refreshRosterFromSling: () => invoke<RosterSyncSummary>("refresh_roster_from_sling"),
  listQualifiedPairs: () => invoke<string[]>("list_qualified_pairs"),
  generateProposal: (targetMonth: string) =>
    invoke<GenerateResult>("generate_proposal", { targetMonth }),
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
  setSlingToken: (value: string) => invoke<void>("set_sling_token", { value }),
  hasSlingToken: () => invoke<boolean>("has_sling_token"),
  setSlingCredentials: (email: string, password: string) =>
    invoke<void>("set_sling_credentials", { email, password }),
  hasSlingCredentials: () => invoke<boolean>("has_sling_credentials"),
  getStudioConfig: () => invoke<StudioConfig>("get_studio_config"),
  setStudioConfig: (orgId: number, actingUserId: number, homeLocationId: number) =>
    invoke<void>("set_studio_config", { orgId, actingUserId, homeLocationId }),
  discoverStudioConfig: () => invoke<DiscoveredStudio>("discover_studio_config"),
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
  pushProposalDryRun: (proposalId: number) =>
    invoke<PushPreview>("push_proposal_dry_run", { proposalId }),
  pushProposalExecute: (proposalId: number) =>
    invoke<PushSummary>("push_proposal_execute", { proposalId }),
};

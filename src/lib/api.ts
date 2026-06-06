// Thin wrapper around Tauri IPC. One function per Rust command.
// Keep call signatures here in sync with src-tauri/src/commands.rs.

import { invoke } from "@tauri-apps/api/core";
import type {
  Teacher,
  SlingCandidate,
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
} from "../types";

export const api = {
  dbInfo: () => invoke<DbInfo>("db_info"),
  listTeachers: () => invoke<Teacher[]>("list_teachers"),
  listSlingCandidates: () => invoke<SlingCandidate[]>("list_sling_candidates"),
  updateTeacherSettings: (slingUserId: number, weeklyTarget: number, weeklyMax: number) =>
    invoke<void>("update_teacher_settings", { slingUserId, weeklyTarget, weeklyMax }),
  listPositions: () => invoke<Position[]>("list_positions"),
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
  listEditsForProposal: (proposalId: number) =>
    invoke<EditRow[]>("list_edits_for_proposal", { proposalId }),
  setAnthropicKey: (value: string) =>
    invoke<void>("set_anthropic_key", { value }),
  hasAnthropicKey: () => invoke<boolean>("has_anthropic_key"),
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
  addTeacherFromPull: (input: { sling_user_id: number; display_name: string;
    weekly_target: number; weekly_max: number; is_lead: boolean; }) =>
    invoke<void>("add_teacher_from_pull", { input }),
  pushProposalDryRun: (proposalId: number) =>
    invoke<PushPreview>("push_proposal_dry_run", { proposalId }),
  pushProposalExecute: (proposalId: number) =>
    invoke<PushSummary>("push_proposal_execute", { proposalId }),
};

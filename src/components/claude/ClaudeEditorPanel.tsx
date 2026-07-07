import { useState } from "react";
import { Code2, Sparkles, Wand2 } from "lucide-react";
import { api } from "../../lib/api";
import { LoadingBlock } from "../ui/LoadingBlock";
import { Field } from "../ui/Field";
import { EditChecklist } from "./EditChecklist";
import { VersionProposalCard } from "./VersionProposalCard";
import type {
  ClaudeEditResult,
  CodeDraft,
  DraftValidation,
  Position,
  ProposalDetail,
  Teacher,
} from "../../types";

interface Props {
  detail: ProposalDetail;
  positions: Position[];
  teachers: Teacher[];
  hasKey: boolean;
  onProposalChanged: () => void;
  onVersionAdopted: () => void;
}

const SHORTCUT_INSTRUCTION = "Resolve the open conflicts in this proposal.";

/** The instruction box + result surface: edit checklist, version proposal,
 *  and the two-step code-draft flow. */
export function ClaudeEditorPanel({
  detail,
  positions,
  teachers,
  hasKey,
  onProposalChanged,
  onVersionAdopted,
}: Props) {
  const [instruction, setInstruction] = useState("");
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<ClaudeEditResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Code-draft flow
  const [drafting, setDrafting] = useState(false);
  const [draft, setDraft] = useState<CodeDraft | null>(null);
  const [validating, setValidating] = useState(false);
  const [validation, setValidation] = useState<DraftValidation | null>(null);
  const [showScript, setShowScript] = useState(false);
  // Code versions carry the ACTIVE rules forward — adopting code must not
  // silently drop the standing rule set.
  const [carriedRules, setCarriedRules] = useState<Record<string, unknown>>({});

  const send = async (text: string) => {
    if (!text.trim()) return;
    setRunning(true);
    setError(null);
    setResult(null);
    setDraft(null);
    setValidation(null);
    try {
      setResult(await api.claudeEditProposal(detail.summary.id, text.trim()));
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  };

  const onDraftCode = async () => {
    if (!result?.needs_code_change) return;
    setDrafting(true);
    setError(null);
    try {
      const versions = await api.listAlgorithmVersions();
      setCarriedRules((versions[0]?.rules as Record<string, unknown>) ?? {});
      setDraft(
        await api.claudeDraftCodeChange(
          detail.summary.id,
          instruction.trim() || SHORTCUT_INSTRUCTION,
          result.needs_code_change.rationale,
        ),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setDrafting(false);
    }
  };

  const onValidate = async () => {
    if (!draft) return;
    setValidating(true);
    setError(null);
    try {
      setValidation(await api.validateCodeDraft(draft.script));
    } catch (e) {
      setError(String(e));
    } finally {
      setValidating(false);
    }
  };

  if (!hasKey) {
    return (
      <div className="card">
        <strong>Ask Claude</strong>
        <div className="muted" style={{ marginTop: 8 }}>
          Set your Anthropic API key in Settings to use the editor.
        </div>
      </div>
    );
  }

  return (
    <div className="card">
      <strong>Ask Claude</strong>
      <Field
        label="Ask Claude to adjust this proposal"
        style={{ marginTop: 10 }}
        hint="Edits are proposed first — nothing changes until you apply it."
      >
        <textarea
          rows={2}
          value={instruction}
          placeholder='e.g. "Give Morgan more Saturday classes" or "make the Tuesday 5:30 a Classic"'
          onChange={(e) => setInstruction(e.target.value)}
          disabled={running}
        />
      </Field>
      <div className="row">
        <button
          className="btn-primary"
          onClick={() => send(instruction)}
          disabled={running || !instruction.trim()}
        >
          <Sparkles size={15} /> {running ? "Asking…" : "Send"}
        </button>
        <button
          className="btn-ghost"
          disabled={running}
          onClick={() => {
            setInstruction(SHORTCUT_INSTRUCTION);
            send(SHORTCUT_INSTRUCTION);
          }}
        >
          <Wand2 size={15} /> Resolve open conflicts
        </button>
      </div>

      {running && <LoadingBlock label="Asking Claude…" />}
      {error && <div className="error">{error}</div>}

      {result && !running && (
        <div style={{ marginTop: 14 }}>
          <p style={{ margin: 0 }}>{result.summary}</p>
          <div className="muted" style={{ fontSize: 12, marginTop: 4 }}>
            {result.model} · ${result.cost_usd.toFixed(4)} ·{" "}
            {(result.duration_ms / 1000).toFixed(1)}s
          </div>

          {result.edits.length > 0 && (
            <EditChecklist
              edits={result.edits}
              detail={detail}
              positions={positions}
              teachers={teachers}
              onProposalChanged={onProposalChanged}
            />
          )}

          {result.ruleset_proposal && (
            <VersionProposalCard
              proposal={result.ruleset_proposal}
              runId={result.run_id}
              onAdopted={onVersionAdopted}
            />
          )}

          {result.needs_code_change && !draft && (
            <div className="suggestion" style={{ marginTop: 14 }}>
              <div className="row" style={{ marginBottom: 6 }}>
                <Code2 size={16} style={{ color: "var(--color-info)" }} />
                <strong>This needs a code change</strong>
              </div>
              <div className="muted" style={{ fontSize: 13 }}>
                {result.needs_code_change.rationale}
              </div>
              <div className="row" style={{ marginTop: 10 }}>
                <button className="btn-primary" onClick={onDraftCode} disabled={drafting}>
                  {drafting ? "Drafting…" : "Draft code change"}
                </button>
              </div>
            </div>
          )}

          {draft && (
            <div className="suggestion" style={{ marginTop: 14 }}>
              <div className="row" style={{ marginBottom: 6 }}>
                <Code2 size={16} style={{ color: "var(--color-info)" }} />
                <strong>Code draft</strong>
              </div>
              <div>{draft.description}</div>
              <div className="muted" style={{ fontSize: 12, marginTop: 4 }}>
                {draft.model} · ${draft.cost_usd.toFixed(4)}
              </div>
              <div className="row" style={{ marginTop: 10 }}>
                <button className="btn-ghost btn-sm" onClick={onValidate} disabled={validating}>
                  {validating ? "Running…" : "Validate against the last month"}
                </button>
                <button className="btn-ghost btn-sm" onClick={() => setShowScript(!showScript)}>
                  {showScript ? "Hide script" : "View script"}
                </button>
              </div>
              {validation && (
                <div style={{ marginTop: 8 }}>
                  {validation.ok ? (
                    <div className="ok" style={{ marginTop: 0 }}>
                      Ran clean on {validation.month}: {validation.shift_count} shifts,{" "}
                      {validation.changed_assignments} assignments differ, +
                      {validation.added_slots}/−{validation.removed_slots} slots.
                    </div>
                  ) : (
                    <div className="error" style={{ marginTop: 0 }}>{validation.error}</div>
                  )}
                </div>
              )}
              {showScript && <pre className="bk-code-scroll">{draft.script}</pre>}
              <VersionProposalCard
                proposal={{ description: draft.description, rules: carriedRules }}
                runId={draft.run_id}
                scriptContent={draft.script}
                adoptDisabled={!validation?.ok}
                adoptDisabledNote="Validate the draft against the last month first"
                onAdopted={onVersionAdopted}
              />
            </div>
          )}
        </div>
      )}
    </div>
  );
}

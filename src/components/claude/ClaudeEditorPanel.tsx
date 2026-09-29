import { useRef, useState } from "react";
import { Code2, Sparkles, Wand2 } from "lucide-react";
import { api } from "../../lib/api";
import { LoadingBlock } from "../ui/LoadingBlock";
import { Field } from "../ui/Field";
import { EditChecklist } from "./EditChecklist";
import { VersionProposalCard } from "./VersionProposalCard";
import type {
  ClaudeEditResult,
  CodeDraft,
  Position,
  ProposalDetail,
  Teacher,
} from "../../types";

interface Props {
  detail: ProposalDetail;
  positions: Position[];
  teachers: Teacher[];
  hasKey: boolean;
  /** Past month: no edits (the Claude tab mirrors the calendar's read-only). */
  readonly: boolean;
  onProposalChanged: () => void;
  onVersionAdopted: () => void;
}

const SHORTCUT_INSTRUCTION = "Resolve the open conflicts in this proposal.";

/** The instruction box + result surface: edit checklist, version proposal,
 *  and the code-draft flow (draft → diff + validate → adopt). */
export function ClaudeEditorPanel({
  detail,
  positions,
  teachers,
  hasKey,
  readonly,
  onProposalChanged,
  onVersionAdopted,
}: Props) {
  const [instruction, setInstruction] = useState("");
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<ClaudeEditResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [drafting, setDrafting] = useState(false);
  const [draft, setDraft] = useState<CodeDraft | null>(null);
  // Guards against double-submit between the click and the re-render that
  // disables the buttons.
  const busy = useRef(false);

  const send = async (text: string) => {
    if (!text.trim() || readonly || busy.current) return;
    busy.current = true;
    setRunning(true);
    setError(null);
    setResult(null);
    setDraft(null);
    try {
      setResult(await api.claudeEditProposal(detail.summary.id, text.trim()));
    } catch (e) {
      setError(String(e));
    } finally {
      busy.current = false;
      setRunning(false);
    }
  };

  const onDraftCode = async () => {
    if (!result?.needs_code_change || busy.current) return;
    busy.current = true;
    setDrafting(true);
    setError(null);
    try {
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
      busy.current = false;
      setDrafting(false);
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

  const working = running || drafting;

  return (
    <div className="card">
      <strong>Ask Claude</strong>
      {readonly ? (
        <div className="muted" style={{ marginTop: 8 }}>
          Past month — read only. Claude can't edit this proposal; open the current or an
          upcoming month to ask for changes.
        </div>
      ) : (
        <>
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
              disabled={working}
            />
          </Field>
          <div className="row">
            <button
              className="btn-primary"
              onClick={() => send(instruction)}
              disabled={working || !instruction.trim()}
            >
              <Sparkles size={15} /> {running ? "Asking…" : "Send"}
            </button>
            <button
              className="btn-ghost"
              disabled={working}
              onClick={() => {
                setInstruction(SHORTCUT_INSTRUCTION);
                send(SHORTCUT_INSTRUCTION);
              }}
            >
              <Wand2 size={15} /> Resolve open conflicts
            </button>
          </div>
        </>
      )}

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
              readonly={readonly}
              onProposalChanged={onProposalChanged}
            />
          )}

          {result.ruleset_proposal && (
            <VersionProposalCard
              proposal={result.ruleset_proposal}
              runId={result.run_id}
              teachers={teachers}
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
                <button className="btn-primary" onClick={onDraftCode} disabled={working}>
                  {drafting ? "Drafting…" : "Draft code change"}
                </button>
              </div>
              {drafting && <LoadingBlock label="Claude is drafting edits to the active script…" />}
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
                {draft.edit_count} edit{draft.edit_count === 1 ? "" : "s"} to the active script ·{" "}
                {draft.model} · ${draft.cost_usd.toFixed(4)} ·{" "}
                {(draft.duration_ms / 1000).toFixed(1)}s
              </div>
              <VersionProposalCard
                proposal={{ description: draft.description, rules: draft.rules }}
                runId={draft.run_id}
                scriptContent={draft.script}
                teachers={teachers}
                onAdopted={onVersionAdopted}
              />
            </div>
          )}
        </div>
      )}
    </div>
  );
}

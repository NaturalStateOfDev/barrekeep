import { useEffect, useRef, useState } from "react";
import { Code2, Sparkles, Wand2 } from "lucide-react";
import { api } from "../../lib/api";
import { LoadingBlock } from "../ui/LoadingBlock";
import { Field } from "../ui/Field";
import { EditChecklist } from "./EditChecklist";
import { VersionProposalCard } from "./VersionProposalCard";
import { estimateCostLabel, whatIfName } from "../../lib/drafts";
import type {
  ClaudeEditResult,
  CodeDraft,
  Position,
  ProposalDetail,
  ProposalSummary,
  Teacher,
} from "../../types";

interface Props {
  detail: ProposalDetail;
  positions: Position[];
  teachers: Teacher[];
  hasKey: boolean;
  /** Past month: no edits (the Claude tab mirrors the calendar's read-only). */
  readonly: boolean;
  /** The month's (non-archived) drafts a prompt can target. */
  monthDrafts: ProposalSummary[];
  onProposalChanged: () => void;
  /** Another draft changed or was created (refresh the draft list). */
  onDraftsChanged: () => void;
  onOpenDraft: (id: number) => void;
  onVersionAdopted: () => void;
}

/** One draft's answer to a prompt. */
interface DraftRun {
  draftId: number;
  name: string;
  /** True when this draft was created by "Duplicate first". */
  copied: boolean;
  result?: ClaudeEditResult;
  detail?: ProposalDetail;
  error?: string;
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
  monthDrafts,
  onProposalChanged,
  onDraftsChanged,
  onOpenDraft,
  onVersionAdopted,
}: Props) {
  const [instruction, setInstruction] = useState("");
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<string | null>(null);
  const [runs, setRuns] = useState<DraftRun[]>([]);
  const [targets, setTargets] = useState<Set<number>>(() => new Set([detail.summary.id]));
  const [duplicateFirst, setDuplicateFirst] = useState(false);
  const [lastCostPerDraft, setLastCostPerDraft] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [drafting, setDrafting] = useState(false);
  const [draft, setDraft] = useState<CodeDraft | null>(null);
  // Guards against double-submit between the click and the re-render that
  // disables the buttons.
  const busy = useRef(false);

  // Switching drafts retargets the prompt at the draft now on screen.
  useEffect(() => {
    setTargets(new Set([detail.summary.id]));
  }, [detail.summary.id]);

  // The run whose rule / code-change suggestion is shown (rules are
  // algorithm-wide, so one card is enough): the first draft that answered.
  const primaryRun = runs.find((r) => r.result);
  const result: ClaudeEditResult | null = primaryRun?.result ?? null;
  const primaryDraftId = primaryRun?.draftId ?? detail.summary.id;

  // Targets in display order: the viewed draft first, then the month's list order.
  const orderedTargets = [
    ...(targets.has(detail.summary.id) ? [detail.summary.id] : []),
    ...monthDrafts.map((d) => d.id).filter((id) => id !== detail.summary.id && targets.has(id)),
  ];
  const nameOf = (id: number) =>
    id === detail.summary.id
      ? detail.summary.name
      : monthDrafts.find((d) => d.id === id)?.name ?? `Draft #${id}`;

  const send = async (text: string) => {
    if (!text.trim() || readonly || busy.current || orderedTargets.length === 0) return;
    busy.current = true;
    setRunning(true);
    setError(null);
    setRuns([]);
    setDraft(null);
    // One Claude call per draft, sequentially: shift ids differ per draft,
    // so each draft needs its own edit list. All runs are linked to the
    // first one (claude_run_targets).
    const out: DraftRun[] = [];
    let groupRunId: number | undefined;
    let created = false;
    try {
      for (const [i, sourceId] of orderedTargets.entries()) {
        let draftId = sourceId;
        let name = nameOf(sourceId);
        setProgress(`${i + 1}/${orderedTargets.length}: ${name}`);
        try {
          if (duplicateFirst) {
            name = whatIfName(name);
            draftId = await api.duplicateProposal(sourceId, name);
            created = true;
          }
          const r = await api.claudeEditProposal(draftId, text.trim(), groupRunId);
          groupRunId ??= r.run_id;
          const d = draftId === detail.summary.id ? detail : await api.getProposal(draftId);
          out.push({ draftId, name, copied: draftId !== sourceId, result: r, detail: d });
        } catch (e) {
          out.push({ draftId, name, copied: draftId !== sourceId, error: String(e) });
        }
        setRuns([...out]);
      }
      const costs = out.flatMap((r) => (r.result ? [r.result.cost_usd] : []));
      if (costs.length > 0) setLastCostPerDraft(costs.reduce((a, b) => a + b, 0) / costs.length);
    } finally {
      if (created) onDraftsChanged();
      busy.current = false;
      setRunning(false);
      setProgress(null);
    }
  };

  /** Edits applied on some draft: refresh that draft (or the screen's). */
  const onRunDraftChanged = async (draftId: number) => {
    if (draftId === detail.summary.id) {
      onProposalChanged();
      return;
    }
    onDraftsChanged();
    try {
      const d = await api.getProposal(draftId);
      setRuns((prev) => prev.map((r) => (r.draftId === draftId ? { ...r, detail: d } : r)));
    } catch {
      // The list refresh above still reflects the change.
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
          primaryDraftId,
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
              placeholder='e.g. "Give Morgan more Saturday classes" or "keep each teacher on the same weekday and time all month"'
              onChange={(e) => setInstruction(e.target.value)}
              disabled={working}
            />
          </Field>
          {monthDrafts.length > 1 && (
            <div className="bk-draft-targets">
              <span className="muted">Apply to:</span>
              {monthDrafts.map((d) => (
                <label key={d.id} className="bk-draft-target">
                  <input
                    type="checkbox"
                    style={{ accentColor: "var(--accent)" }}
                    checked={targets.has(d.id)}
                    disabled={working}
                    onChange={(e) =>
                      setTargets((prev) => {
                        const next = new Set(prev);
                        if (e.target.checked) next.add(d.id);
                        else next.delete(d.id);
                        return next;
                      })
                    }
                  />
                  {d.name}
                  {d.id === detail.summary.id && <span className="muted"> (viewing)</span>}
                </label>
              ))}
            </div>
          )}
          <label className="bk-draft-target" style={{ marginBottom: 6 }}>
            <input
              type="checkbox"
              style={{ accentColor: "var(--accent)" }}
              checked={duplicateFirst}
              disabled={working}
              onChange={(e) => setDuplicateFirst(e.target.checked)}
            />
            Duplicate first, then apply (a what-if copy; the original stays as it is)
          </label>
          <div className="muted" style={{ fontSize: 12, marginBottom: 8 }}>
            {estimateCostLabel(lastCostPerDraft, orderedTargets.length)}
          </div>
          <div className="row">
            <button
              className="btn-primary"
              onClick={() => send(instruction)}
              disabled={working || !instruction.trim() || orderedTargets.length === 0}
            >
              <Sparkles size={15} /> {running ? "Asking…" : "Send"}
            </button>
            <button
              className="btn-ghost"
              disabled={working || orderedTargets.length === 0}
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

      {running && (
        <LoadingBlock
          label={
            orderedTargets.length > 1 || duplicateFirst
              ? `Asking Claude… (${progress ?? ""})`
              : "Asking Claude…"
          }
        />
      )}
      {error && <div className="error">{error}</div>}

      {!running &&
        runs.map((r) => (
          <div
            key={r.draftId}
            className={runs.length > 1 || r.copied ? "bk-draft-run" : undefined}
            style={{ marginTop: 14 }}
          >
            {(runs.length > 1 || r.draftId !== detail.summary.id) && (
              <div className="row" style={{ marginBottom: 6 }}>
                <strong>{r.name}</strong>
                {r.copied && <span className="muted" style={{ fontSize: 12 }}>new copy</span>}
                {r.draftId !== detail.summary.id && (
                  <button
                    className="btn-ghost btn-sm"
                    style={{ marginLeft: "auto" }}
                    onClick={() => onOpenDraft(r.draftId)}
                  >
                    Open draft
                  </button>
                )}
              </div>
            )}
            {r.error && <div className="error">{r.error}</div>}
            {r.result && (
              <>
                <p style={{ margin: 0 }}>{r.result.summary}</p>
                <div className="muted" style={{ fontSize: 12, marginTop: 4 }}>
                  {r.result.model} · ${r.result.cost_usd.toFixed(4)} ·{" "}
                  {(r.result.duration_ms / 1000).toFixed(1)}s
                </div>
                {r.result.edits.length > 0 && r.detail && (
                  <EditChecklist
                    key={`${r.draftId}-${r.result.run_id}`}
                    edits={r.result.edits}
                    detail={r.draftId === detail.summary.id ? detail : r.detail}
                    positions={positions}
                    teachers={teachers}
                    readonly={readonly}
                    onProposalChanged={() => onRunDraftChanged(r.draftId)}
                  />
                )}
              </>
            )}
          </div>
        ))}

      {result && !running && (
        <div style={{ marginTop: 14 }}>

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

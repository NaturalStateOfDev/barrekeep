import { useEffect, useMemo, useRef, useState } from "react";
import { GitBranchPlus } from "lucide-react";
import { api } from "../../lib/api";
import { confirmLabel, diffLines, ruleDiffLabel, unexpectedSlots } from "../../lib/rules";
import { LoadingBlock } from "../ui/LoadingBlock";
import type {
  CandidatePreview,
  RulesetProposal,
  SlotChange,
  Teacher,
} from "../../types";

interface Props {
  proposal: RulesetProposal;
  runId: number;
  /** Also pass a script for code-draft adoptions. */
  scriptContent?: string;
  teachers: Teacher[];
  onAdopted: (version: number) => void;
}

function changeLabel(c: SlotChange): string {
  const when = `${c.weekday} ${c.date.slice(5)} ${c.start}`;
  if (c.kind === "added") return `${when} · new slot: ${c.class_after} — ${c.teacher_after}`;
  if (c.kind === "removed") return `${when} · slot gone: ${c.class_before} — ${c.teacher_before}`;
  const cls =
    c.class_before === c.class_after ? c.class_before : `${c.class_before} → ${c.class_after}`;
  const who =
    c.teacher_before === c.teacher_after
      ? c.teacher_before
      : `${c.teacher_before} → ${c.teacher_after}`;
  return `${when} · ${cls}: ${who}`;
}

/** A proposed algorithm version (rules or code). Before Adopt it shows the
 *  rules diff and script diff against the ACTIVE version and re-runs the
 *  most recent month with both ("reproduce last month"); Adopt stays
 *  disabled on errors, and needs an explicit confirm (naming what is being
 *  accepted) when too many assignments change or slots appear/disappear
 *  with no time-shift rule to explain them. */
export function VersionProposalCard({ proposal, runId, scriptContent, teachers, onAdopted }: Props) {
  const [nextVersion, setNextVersion] = useState<number | null>(null);
  const [preview, setPreview] = useState<CandidatePreview | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [confirmAnyway, setConfirmAnyway] = useState(false);
  const [adopting, setAdopting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [adoptedAs, setAdoptedAs] = useState<number | null>(null);
  const [showDiff, setShowDiff] = useState(true);
  const [showChanges, setShowChanges] = useState(false);
  const inFlight = useRef(false);

  const teacherName = useMemo(() => {
    const byId = new Map(teachers.map((t) => [String(t.sling_user_id), t.display_name]));
    return (uid: unknown) => byId.get(String(uid)) ?? `teacher ${uid}`;
  }, [teachers]);

  const runPreview = async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setPreviewing(true);
    setError(null);
    setConfirmAnyway(false);
    try {
      setPreview(await api.previewAlgorithmCandidate(proposal.rules, scriptContent));
    } catch (e) {
      setError(String(e));
    } finally {
      inFlight.current = false;
      setPreviewing(false);
    }
  };

  // Keyed on content, not object identity: parents may rebuild the
  // proposal object on every render.
  const candidateKey = JSON.stringify(proposal.rules) + "\u0000" + (scriptContent ?? "");
  useEffect(() => {
    api
      .listAlgorithmVersions()
      .then((vs) => setNextVersion(Math.max(9, ...vs.map((v) => v.version)) + 1))
      .catch(() => setNextVersion(null));
    runPreview();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [candidateKey]);

  const status = preview?.validation.status;
  const canAdopt =
    !!preview && (status === "pass" || (status === "needs_confirm" && confirmAnyway));

  const onAdopt = async () => {
    if (!canAdopt || inFlight.current) return;
    inFlight.current = true;
    setAdopting(true);
    setError(null);
    try {
      const v = await api.adoptAlgorithmVersion(
        proposal.description,
        proposal.rules,
        scriptContent,
        runId,
      );
      setAdoptedAs(v);
      onAdopted(v);
    } catch (e) {
      setError(String(e));
    } finally {
      inFlight.current = false;
      setAdopting(false);
    }
  };

  const removed = preview?.rules_diff.filter((e) => e.kind === "removed") ?? [];
  const v = preview?.validation;
  const unexpected = v ? unexpectedSlots(v) : { added: [], removed: [] };

  return (
    <div className="suggestion" style={{ marginTop: 14 }}>
      <div className="row" style={{ marginBottom: 6 }}>
        <GitBranchPlus size={16} style={{ color: "var(--accent)" }} />
        <strong>{scriptContent ? "Proposed code version" : "Proposed rule version"}</strong>
        {preview && (
          <span className="muted" style={{ fontSize: 12 }}>
            compared with the active v{preview.active_version}
          </span>
        )}
      </div>
      <div>{proposal.description}</div>

      {/* Rules diff vs the active version */}
      {preview && (preview.rules_diff.length > 0 || !scriptContent) && (
        <div style={{ marginTop: 10 }}>
          <div className="bk-candidate-label">Rule changes</div>
          {preview.rules_diff.length === 0 ? (
            <div className="muted" style={{ fontSize: 13 }}>
              Same rules as the active version.
            </div>
          ) : (
            <ul className="bk-change-list">
              {preview.rules_diff.map((e, i) => (
                <li
                  key={i}
                  className={
                    e.kind === "removed"
                      ? "bk-change-removed"
                      : e.kind === "added"
                        ? "bk-change-added"
                        : ""
                  }
                >
                  <strong>{e.kind === "added" ? "+ " : e.kind === "removed" ? "− " : "~ "}</strong>
                  {ruleDiffLabel(e, teacherName)}
                </li>
              ))}
            </ul>
          )}
          {removed.length > 0 && (
            <div className="bk-warn">
              This proposal drops {removed.length} standing rule{removed.length === 1 ? "" : "s"}.
              Claude sends the whole rule set, so a missing entry deletes the rule — make sure
              that is intended before adopting.
            </div>
          )}
        </div>
      )}

      {/* Script diff vs the active script */}
      {preview?.script_diff != null && (
        <div style={{ marginTop: 10 }}>
          <button className="disclosure" onClick={() => setShowDiff(!showDiff)} style={{ marginBottom: 0 }}>
            {showDiff ? "Hide script diff" : "Show script diff"}
          </button>
          {showDiff &&
            (preview.script_diff === "" ? (
              <div className="muted" style={{ fontSize: 13 }}>Identical to the active script.</div>
            ) : (
              <pre className="bk-code-scroll bk-diff">
                {diffLines(preview.script_diff).map((l, i) => (
                  <span
                    key={i}
                    className={
                      l.kind === "add"
                        ? "bk-diff-add"
                        : l.kind === "del"
                          ? "bk-diff-del"
                          : l.kind === "hunk" || l.kind === "meta"
                            ? "bk-diff-hunk"
                            : ""
                    }
                  >
                    {l.text}
                  </span>
                ))}
              </pre>
            ))}
        </div>
      )}

      {/* Reproduce-last-month validation */}
      {previewing && <LoadingBlock label="Re-running the last month with the active and the proposed algorithm…" />}
      {v && !previewing && (
        <div style={{ marginTop: 10 }}>
          <div className="bk-candidate-label">Reproduce {v.month || "last month"}</div>
          {v.status === "error" ? (
            <div className="error" style={{ marginTop: 0 }}>{v.error}</div>
          ) : (
            <>
              <div className={v.status === "pass" ? "ok" : "bk-warn"} style={{ marginTop: 0 }}>
                {v.changed_count} of {v.slot_count} assignments change
                {v.added_count + v.removed_count > 0 &&
                  ` · +${v.added_count}/−${v.removed_count} slots${v.unexpected_count === 0 ? " (explained by time shifts)" : ""}`}
                {v.reasons.map((r, i) => (
                  <div key={i}>{r}</div>
                ))}
              </div>
              {unexpected.added.length + unexpected.removed.length > 0 && (
                <div className="bk-slot-alert" role="alert">
                  <strong>
                    This version changes which classes exist in {v.month}: no time-shift rule
                    explains these slots.
                  </strong>
                  <ul className="bk-change-list">
                    {unexpected.added.map((c, i) => (
                      <li key={`a${i}`} className="bk-change-added">+ {changeLabel(c)}</li>
                    ))}
                    {unexpected.removed.map((c, i) => (
                      <li key={`r${i}`} className="bk-change-removed">− {changeLabel(c)}</li>
                    ))}
                  </ul>
                  <div style={{ marginTop: 6 }}>
                    Adopt only if the change is meant to add or remove these classes.
                  </div>
                </div>
              )}
              {v.changes.length > 0 && (
                <>
                  <button className="disclosure" style={{ marginTop: 8, marginBottom: 0 }} onClick={() => setShowChanges(!showChanges)}>
                    {showChanges ? "Hide per-slot changes" : `Show per-slot changes (${v.changes.length})`}
                  </button>
                  {showChanges && (
                    <ul className="bk-change-list bk-code-scroll" style={{ fontFamily: "inherit", whiteSpace: "normal" }}>
                      {v.changes.map((c, i) => (
                        <li
                          key={i}
                          className={
                            c.kind === "removed" ? "bk-change-removed" : c.kind === "added" ? "bk-change-added" : ""
                          }
                        >
                          {changeLabel(c)}
                          {c.expected && <span className="muted"> (time shift)</span>}
                        </li>
                      ))}
                    </ul>
                  )}
                </>
              )}
              {v.status === "needs_confirm" && adoptedAs == null && (
                <label className="row" style={{ marginTop: 8, fontSize: 13, gap: 6 }}>
                  <input
                    type="checkbox"
                    style={{ accentColor: "var(--accent)" }}
                    checked={confirmAnyway}
                    onChange={(e) => setConfirmAnyway(e.target.checked)}
                  />
                  {confirmLabel(v)}
                </label>
              )}
            </>
          )}
        </div>
      )}

      <div className="row" style={{ marginTop: 10 }}>
        {adoptedAs != null ? (
          <span className="ok" style={{ marginTop: 0 }}>Adopted as v{adoptedAs} — now active.</span>
        ) : (
          <>
            <button
              className="btn-primary"
              onClick={onAdopt}
              disabled={!canAdopt || adopting || previewing}
              title={
                canAdopt
                  ? ""
                  : status === "needs_confirm"
                    ? "Tick the confirmation above first"
                    : "The candidate must reproduce the last month first"
              }
            >
              {adopting
                ? "Adopting…"
                : `${status === "needs_confirm" ? "Adopt anyway" : "Adopt"} as v${nextVersion ?? "next"}`}
            </button>
            <button className="btn-ghost btn-sm" onClick={runPreview} disabled={previewing || adopting}>
              {previewing ? "Checking…" : "Re-check"}
            </button>
          </>
        )}
      </div>
      {error && <div className="error">{error}</div>}
    </div>
  );
}

import { useEffect, useState } from "react";
import { GitBranchPlus } from "lucide-react";
import { api } from "../../lib/api";
import type { RulesetProposal } from "../../types";

interface Props {
  proposal: RulesetProposal;
  runId: number;
  /** Also pass a script for code-draft adoptions. */
  scriptContent?: string;
  adoptDisabled?: boolean;
  adoptDisabledNote?: string;
  onAdopted: (version: number) => void;
}

/** A proposed algorithm version (rules or code) with an explicit Adopt. */
export function VersionProposalCard({
  proposal,
  runId,
  scriptContent,
  adoptDisabled,
  adoptDisabledNote,
  onAdopted,
}: Props) {
  const [nextVersion, setNextVersion] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [adoptedAs, setAdoptedAs] = useState<number | null>(null);

  useEffect(() => {
    api
      .listAlgorithmVersions()
      .then((vs) => setNextVersion(Math.max(9, ...vs.map((v) => v.version)) + 1))
      .catch(() => setNextVersion(null));
  }, []);

  const onAdopt = async () => {
    setBusy(true);
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
      setBusy(false);
    }
  };

  return (
    <div className="suggestion" style={{ marginTop: 14 }}>
      <div className="row" style={{ marginBottom: 6 }}>
        <GitBranchPlus size={16} style={{ color: "var(--accent)" }} />
        <strong>{scriptContent ? "Proposed code version" : "Proposed rule version"}</strong>
      </div>
      <div>{proposal.description}</div>
      {!scriptContent && (
        <pre className="bk-code-scroll" style={{ marginTop: 8 }}>
          {JSON.stringify(proposal.rules, null, 2)}
        </pre>
      )}
      <div className="row" style={{ marginTop: 10 }}>
        {adoptedAs != null ? (
          <span className="ok" style={{ marginTop: 0 }}>Adopted as v{adoptedAs}.</span>
        ) : (
          <button
            className="btn-primary"
            onClick={onAdopt}
            disabled={busy || adoptDisabled}
            title={adoptDisabled ? adoptDisabledNote ?? "" : ""}
          >
            {busy ? "Adopting…" : `Adopt as v${nextVersion ?? "next"}`}
          </button>
        )}
      </div>
      {error && <div className="error">{error}</div>}
    </div>
  );
}

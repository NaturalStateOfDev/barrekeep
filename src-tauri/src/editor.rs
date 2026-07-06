//! The Claude proposal editor: turns a user instruction into concrete,
//! validated shift edits, with optional escalation to a rules version or a
//! code-change request (spec: docs/superpowers/specs/
//! 2026-07-06-claude-proposal-editor-design.md).
//!
//! The system prompt is read from prompts/proposal-editor.md at runtime so
//! the user can tune wording without recompiling; the compile-time embed of
//! the same file is the fallback (e.g. installed builds without the repo).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::review::{call_anthropic, compute_cost, extract_json};

const EDITOR_MAX_TOKENS: u32 = 8192;

/// Compile-time copy of prompts/proposal-editor.md (the runtime file wins
/// when present and non-empty).
pub const INLINE_PROMPT: &str = include_str!("../../prompts/proposal-editor.md");

pub fn editor_system_prompt(project_root: Option<&std::path::Path>) -> String {
    if let Some(root) = project_root {
        if let Ok(text) = std::fs::read_to_string(root.join("prompts").join("proposal-editor.md")) {
            if !text.trim().is_empty() {
                return text;
            }
        }
    }
    INLINE_PROMPT.to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProposedEdit {
    pub proposal_shift_id: i64,
    /// "reassign" | "unassign" | "change_format"
    pub action: String,
    #[serde(default)]
    pub new_user_id: Option<i32>,
    #[serde(default)]
    pub new_class_name: Option<String>,
    pub rationale: String,
    /// Set app-side by validation, not by Claude.
    #[serde(default = "default_true")]
    pub valid: bool,
    #[serde(default)]
    pub validation_note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RulesetProposal {
    pub description: String,
    pub rules: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NeedsCodeChange {
    pub rationale: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EditorPayload {
    pub summary: String,
    #[serde(default)]
    pub edits: Vec<ProposedEdit>,
    #[serde(default)]
    pub ruleset_proposal: Option<RulesetProposal>,
    #[serde(default)]
    pub needs_code_change: Option<NeedsCodeChange>,
}

/// Parsed editor response + audit-log accounting.
pub struct EditorCall {
    pub payload: EditorPayload,
    pub raw_input: String,
    pub raw_output: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
    pub duration_ms: u32,
}

pub fn run_editor(
    api_key: &str,
    model: &str,
    system: &str,
    user_payload: &Value,
) -> anyhow::Result<EditorCall> {
    let user_text = format!(
        "Here is the schedule context and the user's instruction as JSON. Respond per your instructions.\n\n{}",
        serde_json::to_string_pretty(user_payload)?
    );

    let call = call_anthropic(api_key, model, system, &user_text, EDITOR_MAX_TOKENS)?;

    let payload: EditorPayload =
        serde_json::from_str(extract_json(&call.raw_output)).map_err(|e| {
            anyhow::anyhow!(
                "Claude did not return valid editor JSON: {e}\n---\n{}",
                call.raw_output
            )
        })?;

    let cost_usd = compute_cost(&call.model, &call.usage);

    Ok(EditorCall {
        payload,
        raw_input: call.raw_input,
        raw_output: call.raw_output,
        model: call.model,
        input_tokens: call.usage.input_tokens,
        output_tokens: call.usage.output_tokens,
        cost_usd,
        duration_ms: call.duration_ms,
    })
}

// ============================================================
// Code drafts (tier 3) — second call that carries the script source.
// ============================================================

/// Appended to the editor system prompt for the code-drafting call.
pub const CODE_DRAFT_PROMPT: &str = r#"You are drafting a new version of the studio's schedule-generation script.

The user payload contains: the current script source (current_script), the
active rules, the original instruction, and the rationale for why rules
cannot express it.

Respond with ONLY valid JSON, no markdown fences:
{"description": "v-next — <one line, what changed>",
 "script": "<the COMPLETE new python script>"}

The script MUST:
- keep the same CLI: --json-out --from-stdin --target-month YYYY-MM
- keep reading the same stdin payload schema, including the "rules" and
  "version_label" keys
- keep emitting the same output JSON schema (algorithm_version echoes
  version_label, target_month, parameters, shifts[])
- change only what the instruction requires; preserve all other behavior.
"#;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CodeDraftPayload {
    pub description: String,
    pub script: String,
}

pub struct CodeDraftCall {
    pub payload: CodeDraftPayload,
    pub raw_input: String,
    pub raw_output: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
    pub duration_ms: u32,
}

pub fn run_code_draft(
    api_key: &str,
    model: &str,
    user_payload: &Value,
) -> anyhow::Result<CodeDraftCall> {
    let user_text = format!(
        "Here is the current script and context as JSON. Draft the new script per your instructions.\n\n{}",
        serde_json::to_string_pretty(user_payload)?
    );

    // Code drafts need room for a full script (~700 lines).
    let call = call_anthropic(api_key, model, CODE_DRAFT_PROMPT, &user_text, 32_000)?;

    let payload: CodeDraftPayload =
        serde_json::from_str(extract_json(&call.raw_output)).map_err(|e| {
            anyhow::anyhow!(
                "Claude did not return valid code-draft JSON: {e}\n---\n{}",
                &call.raw_output[..call.raw_output.len().min(2000)]
            )
        })?;

    let cost_usd = compute_cost(&call.model, &call.usage);

    Ok(CodeDraftCall {
        payload,
        raw_input: call.raw_input,
        raw_output: call.raw_output,
        model: call.model,
        input_tokens: call.usage.input_tokens,
        output_tokens: call.usage.output_tokens,
        cost_usd,
        duration_ms: call.duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_editor_response_with_all_sections() {
        let raw = json!({
            "summary": "Swapped two slots and noticed a pattern.",
            "edits": [
                {"proposal_shift_id": 12, "action": "reassign", "new_user_id": 501,
                 "rationale": "Morgan asked for Saturdays"},
                {"proposal_shift_id": 13, "action": "change_format",
                 "new_class_name": "Classic", "rationale": "thin Reform coverage"},
                {"proposal_shift_id": 14, "action": "unassign", "rationale": "no cover"}
            ],
            "ruleset_proposal": {
                "description": "v-next — Casey off Reform",
                "rules": {"teacher_class_blocklist": [
                    {"sling_user_id": 502, "class_name": "Reform", "reason": "recurring"}]}
            },
            "needs_code_change": null
        })
        .to_string();
        let parsed: EditorPayload = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.edits.len(), 3);
        assert!(parsed.edits.iter().all(|e| e.valid), "valid defaults true");
        assert_eq!(parsed.edits[0].new_user_id, Some(501));
        assert_eq!(parsed.edits[1].new_class_name.as_deref(), Some("Classic"));
        assert!(parsed.ruleset_proposal.is_some());
        assert!(parsed.needs_code_change.is_none());

        // Minimal response: only a summary.
        let minimal: EditorPayload =
            serde_json::from_str(r#"{"summary": "Nothing to do."}"#).unwrap();
        assert!(minimal.edits.is_empty());
    }

    #[test]
    fn system_prompt_falls_back_inline() {
        let p = editor_system_prompt(Some(std::path::Path::new("/nonexistent")));
        assert!(p.contains("Escalation tiers"));
        assert_eq!(p, INLINE_PROMPT);
    }
}

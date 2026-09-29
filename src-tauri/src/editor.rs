//! The Claude proposal editor: turns a user instruction into concrete,
//! validated shift edits, with optional escalation to a rules version or a
//! code-change request (spec: docs/superpowers/specs/
//! 2026-07-06-claude-proposal-editor-design.md).
//!
//! The system prompt is read from prompts/proposal-editor.md at runtime so
//! the user can tune wording without recompiling; the compile-time embed of
//! the same file is the fallback (e.g. installed builds without the repo).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::review::{call_anthropic, compute_cost, extract_json, CallOptions, SystemBlock};

/// Adaptive thinking counts against max_tokens; 16k leaves room for it and
/// the JSON answer while staying a reasonable non-streaming request.
const EDITOR_MAX_TOKENS: u32 = 16_000;
const EDITOR_TIMEOUT: Duration = Duration::from_secs(300);
/// Code drafts return search/replace edits (not a whole script), so the same
/// output budget fits; they think harder (effort high), so allow longer.
const CODE_DRAFT_MAX_TOKENS: u32 = 16_000;
const CODE_DRAFT_TIMEOUT: Duration = Duration::from_secs(600);

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

    // The editor prompt (with its rule-key reference) is well above the
    // 512-token caching minimum on the 5.5 models — cache it.
    let call = call_anthropic(
        api_key,
        model,
        &[SystemBlock { text: system, cache: true }],
        &user_text,
        &CallOptions { max_tokens: EDITOR_MAX_TOKENS, timeout: EDITOR_TIMEOUT, effort: "medium" },
    )?;

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
// Code drafts (tier 3) — a separate call whose system prompt carries the
// active script. Claude returns search/replace edits; they are applied
// locally so an unchanged 700-line script never round-trips as output.
// ============================================================

/// System prompt (instructions block) for the code-drafting call. The
/// active script follows it as a second, cached system block.
pub const CODE_DRAFT_PROMPT: &str = r#"You are drafting a change to the studio's schedule-generation script (Python). The script is in the next system block; the user message contains the active rules, the original instruction, and the rationale for why the rule keys cannot express it.

Respond with ONLY valid JSON, no markdown fences:
{"description": "v-next — <one line, what changed>",
 "edits": [
   {"search": "<exact text copied from the script>",
    "replace": "<the text to put in its place>"}
 ]}

Edit rules:
- Each "search" must be copied EXACTLY from the script (whitespace and
  indentation included) and must occur exactly ONCE in it. Include enough
  surrounding lines to make it unique, but keep it short.
- Edits are applied in order; a later edit sees the result of earlier ones.
- To insert code, search for the neighbouring lines and repeat them in
  "replace" with the new lines added.
- Never search for text that spans a region an earlier edit changed.

The changed script MUST:
- keep the same CLI: --json-out --from-stdin --target-month YYYY-MM
- keep reading the same stdin payload schema, including the "rules" and
  "version_label" keys
- keep emitting the same output JSON schema (algorithm_version echoes
  version_label, target_month, parameters, shifts[])
- produce exactly the current output when the instruction's situation does
  not arise — change only what the instruction requires.
"#;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct EditBlock {
    pub search: String,
    pub replace: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CodeDraftPayload {
    pub description: String,
    #[serde(default)]
    pub edits: Vec<EditBlock>,
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

/// Apply search/replace edit blocks in order. Every search string must
/// match exactly once in the script as it stands after the earlier edits.
pub fn apply_edit_blocks(script: &str, edits: &[EditBlock]) -> Result<String, String> {
    if edits.is_empty() {
        return Err("Claude returned no edits — nothing to change".to_string());
    }
    let mut out = script.to_string();
    for (i, e) in edits.iter().enumerate() {
        let n = i + 1;
        if e.search.is_empty() {
            return Err(format!("edit {n}: empty search text"));
        }
        let hits = out.matches(e.search.as_str()).count();
        let preview: String = e.search.lines().take(3).collect::<Vec<_>>().join("\n");
        match hits {
            1 => out = out.replacen(e.search.as_str(), &e.replace, 1),
            0 => {
                return Err(format!(
                    "edit {n}: search text not found in the active script{} — starts:\n{preview}",
                    if i > 0 { " (after the earlier edits)" } else { "" }
                ))
            }
            k => {
                return Err(format!(
                    "edit {n}: search text matches {k} places — it must match exactly once. Starts:\n{preview}"
                ))
            }
        }
    }
    if out == script {
        return Err("the edits leave the script unchanged".to_string());
    }
    Ok(out)
}

pub fn run_code_draft(
    api_key: &str,
    model: &str,
    current_script: &str,
    user_payload: &Value,
) -> anyhow::Result<CodeDraftCall> {
    let user_text = format!(
        "Here is the context as JSON. Draft the change to the script per your instructions.\n\n{}",
        serde_json::to_string_pretty(user_payload)?
    );
    let script_block = format!("The active script:\n\n{current_script}");

    // The script (~10k tokens) is the stable, cacheable prefix: a retry or
    // a second draft within 5 minutes reads it from cache.
    let call = call_anthropic(
        api_key,
        model,
        &[
            SystemBlock { text: CODE_DRAFT_PROMPT, cache: false },
            SystemBlock { text: &script_block, cache: true },
        ],
        &user_text,
        &CallOptions {
            max_tokens: CODE_DRAFT_MAX_TOKENS,
            timeout: CODE_DRAFT_TIMEOUT,
            effort: "high",
        },
    )?;

    let payload: CodeDraftPayload =
        serde_json::from_str(extract_json(&call.raw_output)).map_err(|e| {
            anyhow::anyhow!(
                "Claude did not return valid code-draft JSON: {e}\n---\n{}",
                call.raw_output.chars().take(2000).collect::<String>()
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

    #[test]
    fn prompt_documents_every_rule_key() {
        for key in [
            "teacher_class_blocklist",
            "teacher_slot_blocklist",
            "priority_slots",
            "slot_class_overrides",
            "variety_penalty_multiplier",
            "variety_penalty_per_class",
            "sat_time_shifts",
            "sun_time_shifts",
        ] {
            assert!(INLINE_PROMPT.contains(&format!("`{key}`")), "{key} undocumented");
        }
    }

    fn eb(s: &str, r: &str) -> EditBlock {
        EditBlock { search: s.into(), replace: r.into() }
    }

    #[test]
    fn apply_edit_blocks_in_order() {
        let script = "a = 1\nb = 2\nc = 3\n";
        let out = apply_edit_blocks(
            script,
            &[eb("b = 2\n", "b = 20\nb2 = 21\n"), eb("b2 = 21", "b2 = 22")],
        )
        .unwrap();
        assert_eq!(out, "a = 1\nb = 20\nb2 = 22\nc = 3\n");
    }

    #[test]
    fn apply_edit_blocks_errors_are_specific() {
        let script = "x = 1\nx = 1\ny = 2\n";
        let e = apply_edit_blocks(script, &[eb("x = 1", "x = 3")]).unwrap_err();
        assert!(e.contains("edit 1") && e.contains("2 places"), "{e}");
        let e = apply_edit_blocks(script, &[eb("y = 2", "y = 5"), eb("y = 2", "y = 6")])
            .unwrap_err();
        assert!(e.contains("edit 2") && e.contains("not found") && e.contains("earlier"), "{e}");
        assert!(apply_edit_blocks(script, &[]).is_err());
        assert!(apply_edit_blocks(script, &[eb("", "z")]).is_err());
        assert!(apply_edit_blocks(script, &[eb("y = 2", "y = 2")]).is_err());
    }

    #[test]
    fn parses_code_draft_payload() {
        let p: CodeDraftPayload = serde_json::from_str(
            r#"{"description": "v-next — x", "edits": [{"search": "a", "replace": "b"}]}"#,
        )
        .unwrap();
        assert_eq!(p.edits, vec![eb("a", "b")]);
    }
}

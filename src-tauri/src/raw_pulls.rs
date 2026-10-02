// Raw pull audit files. Every Sling pull / availability refresh writes the
// JSON it received — calendar pages and availability-set responses — to
//   <app_local_data>/raw_pulls/<timestamp>-<label>.json
// (next to the database and the backups folder), so "why does the app think
// X?" can be answered from what Sling actually sent. The newest
// `KEEP` files are kept.
//
// What is NOT in a file: request headers (so no bearer token — and the token
// is additionally scrubbed from the text before it is written) and the
// roster response, which carries teachers' contact details.

use std::path::{Path, PathBuf};

use crate::sling::AuditEntry;

/// How many raw pull files are kept.
pub const KEEP: usize = 20;

pub fn raw_pulls_dir(db_file: &Path) -> PathBuf {
    db_file.parent().unwrap_or_else(|| Path::new(".")).join("raw_pulls")
}

fn safe_label(label: &str) -> String {
    label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

pub fn file_name(now: chrono::DateTime<chrono::Utc>, label: &str) -> String {
    format!("{}-{}.json", now.format("%Y%m%dT%H%M%SZ"), safe_label(label))
}

/// Write one audit file and rotate the folder. `secret` (the bearer token)
/// is replaced wherever it appears, should a response ever echo it.
pub fn write(
    dir: &Path,
    now: chrono::DateTime<chrono::Utc>,
    label: &str,
    months: &[String],
    audit: &[AuditEntry],
    summary: serde_json::Value,
    secret: Option<&str>,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    let doc = serde_json::json!({
        "app_version": env!("CARGO_PKG_VERSION"),
        "pulled_at": now.to_rfc3339(),
        "months": months,
        "summary": summary,
        "requests": audit,
    });
    let mut text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    if let Some(s) = secret.map(str::trim).filter(|s| s.len() >= 8) {
        text = text.replace(s, "[redacted]");
        // The token may also be stored as "Bearer <token>".
        if let Some(bare) = s.split_whitespace().last().filter(|b| b.len() >= 8 && *b != s) {
            text = text.replace(bare, "[redacted]");
        }
    }
    let mut path = dir.join(file_name(now, label));
    // Two pulls within the same second: keep both.
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = dir.join(file_name(now, &format!("{label}-{n}")));
    }
    std::fs::write(&path, text).map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    rotate(dir, KEEP);
    Ok(path)
}

/// The raw pull files in `dir`, newest first (names sort chronologically).
pub fn list(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files.reverse();
    files
}

/// Delete all but the newest `keep` files. Best effort.
pub fn rotate(dir: &Path, keep: usize) {
    for old in list(dir).into_iter().skip(keep) {
        let _ = std::fs::remove_file(old);
    }
}

#[derive(serde::Serialize)]
pub struct RawPullsInfo {
    pub dir: String,
    pub count: usize,
    pub keep: usize,
    /// File name of the newest raw pull, if any.
    pub latest: Option<String>,
}

#[tauri::command]
pub fn raw_pulls_info(app: tauri::AppHandle) -> Result<RawPullsInfo, String> {
    let dir = raw_pulls_dir(&crate::db::db_path(&app).map_err(|e| e.to_string())?);
    let files = list(&dir);
    Ok(RawPullsInfo {
        dir: dir.display().to_string(),
        count: files.len(),
        keep: KEEP,
        latest: files.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()),
    })
}

/// Open the raw pulls folder in the OS file manager (created if missing).
#[tauri::command]
pub fn open_raw_pulls_folder(app: tauri::AppHandle) -> Result<(), String> {
    let dir = raw_pulls_dir(&crate::db::db_path(&app).map_err(|e| e.to_string())?);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // explorer.exe returns exit code 1 even on success, so only a failure to
    // launch counts as an error (same as backup::open_backups_folder).
    std::process::Command::new(program)
        .arg(&dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't open {}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "barrekeep-rawpull-test-{tag}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn at(secs: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_790_000_000 + secs, 0).unwrap()
    }

    fn entry(response: serde_json::Value) -> AuditEntry {
        AuditEntry {
            label: "availability user 7 (bare)".into(),
            url: "https://api.getsling.com/v1/availability".into(),
            query: vec![("userId".into(), "7".into())],
            outcome: "ok".into(),
            response,
        }
    }

    #[test]
    fn writes_requests_and_names_file_by_time_and_month() {
        let dir = tmpdir("write");
        let p = write(
            &dir,
            at(0),
            "2026-11",
            &["2026-11".to_string()],
            &[entry(serde_json::json!([{"interval": "P1W"}]))],
            serde_json::json!({"sets": 1}),
            None,
        )
        .unwrap();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with("-2026-11.json"), "{name}");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(doc["months"][0], "2026-11");
        assert_eq!(doc["requests"][0]["response"][0]["interval"], "P1W");
        assert_eq!(doc["requests"][0]["query"][0][1], "7");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn never_contains_the_token() {
        let dir = tmpdir("token");
        let token = "Bearer abcdef0123456789abcdef";
        // Even if Sling echoed the credential back inside a response body.
        let p = write(
            &dir,
            at(0),
            "2026-11",
            &[],
            &[entry(serde_json::json!({"echo": token, "also": "abcdef0123456789abcdef"}))],
            serde_json::Value::Null,
            Some(token),
        )
        .unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("abcdef0123456789abcdef"), "token leaked into raw pull file");
        assert!(!text.to_lowercase().contains("authorization"));
        assert!(text.contains("[redacted]"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn keeps_only_the_newest_twenty() {
        let dir = tmpdir("rotate");
        for i in 0..25 {
            write(&dir, at(i * 60), "2026-11", &[], &[], serde_json::Value::Null, None).unwrap();
        }
        let files = list(&dir);
        assert_eq!(files.len(), KEEP);
        // Newest first; the five oldest are gone.
        assert_eq!(files[0].file_name().unwrap().to_string_lossy(), file_name(at(24 * 60), "2026-11"));
        assert_eq!(files[KEEP - 1].file_name().unwrap().to_string_lossy(), file_name(at(5 * 60), "2026-11"));
        // Non-JSON files in the folder are left alone.
        std::fs::write(dir.join("notes.txt"), "keep me").unwrap();
        rotate(&dir, 1);
        assert_eq!(list(&dir).len(), 1);
        assert!(dir.join("notes.txt").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn same_second_pulls_do_not_overwrite_each_other() {
        let dir = tmpdir("collide");
        let a = write(&dir, at(0), "refresh", &[], &[], serde_json::Value::Null, None).unwrap();
        let b = write(&dir, at(0), "refresh", &[], &[], serde_json::Value::Null, None).unwrap();
        assert_ne!(a, b);
        assert_eq!(list(&dir).len(), 2);
        std::fs::remove_dir_all(dir).ok();
    }
}

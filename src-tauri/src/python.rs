//! Python interpreter discovery for the propose.py sidecar.
//!
//! propose.py is stdlib-only but needs Python >= 3.11. On Windows the obvious
//! `python` is often the Microsoft Store "App execution alias" stub, which
//! prints "Python was not found…" and exits 9009 instead of running anything.
//! So instead of spawning a hard-coded name, probe candidates in order —
//! Windows: `py -3`, `python`, `python3`; elsewhere: `python3`, `python` —
//! and use the first real interpreter that is new enough. A successful
//! resolution is cached for the life of the process; failures are not, so the
//! Settings "Re-check" button (check_python) probes again.
//!
//! On Windows every probe/sidecar spawn uses CREATE_NO_WINDOW so a console
//! window doesn't flash up over the app.

use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

pub const MIN_VERSION: (u32, u32) = (3, 11);

/// Prints "X.Y.Z" then sys.executable, one per line.
const PROBE_SCRIPT: &str =
    "import sys; print('%d.%d.%d' % tuple(sys.version_info[:3])); print(sys.executable)";

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Exit code of the Microsoft Store python.exe alias when Python isn't installed.
const STORE_STUB_EXIT: i32 = 9009;

/// (program, leading args) to try, in order.
pub fn candidates(windows: bool) -> Vec<(&'static str, Vec<&'static str>)> {
    if windows {
        vec![("py", vec!["-3"]), ("python", vec![]), ("python3", vec![])]
    } else {
        vec![("python3", vec![]), ("python", vec![])]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interpreter {
    pub program: String,
    pub prefix_args: Vec<String>,
    pub version: (u32, u32, u32),
    pub executable: String,
}

impl Interpreter {
    /// A Command for this interpreter with its leading args already applied.
    pub fn command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.prefix_args);
        hide_console(&mut c);
        c
    }

    /// How the user would type it: "py -3", "python3".
    pub fn display(&self) -> String {
        label(&self.program, &self.prefix_args)
    }
}

fn label<S: AsRef<str>>(program: &str, args: &[S]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(|a| a.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
fn hide_console(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    c.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_c: &mut Command) {}

/// What one candidate turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    Ok { version: (u32, u32, u32), executable: String },
    /// The Microsoft Store alias stub — Python isn't actually installed.
    StoreStub,
    /// No such program on PATH.
    NotFound,
    /// Ran but didn't behave like Python.
    Failed(String),
}

/// Parse "3.12.1" (or "Python 3.12.1", "3.13.0rc1") into (major, minor, patch).
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim();
    let s = s.strip_prefix("Python").map(str::trim).unwrap_or(s);
    let mut parts = s.split('.');
    let major: u32 = parts.next()?.trim().parse().ok()?;
    let minor: u32 = parts.next()?.trim().parse().ok()?;
    let patch: u32 = parts
        .next()
        .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    Some((major, minor, patch))
}

pub fn meets_minimum(v: (u32, u32, u32)) -> bool {
    (v.0, v.1) >= MIN_VERSION
}

/// Classify a finished probe. `resolved_path` is where the program was found
/// on PATH (if known) — a WindowsApps path that produced no version is the
/// Store stub even when the exit code isn't the usual 9009.
pub fn classify_probe(
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
    resolved_path: Option<&str>,
) -> ProbeOutcome {
    let mut lines = stdout.lines().map(str::trim).filter(|l| !l.is_empty());
    let version = lines.next().and_then(parse_version);
    if exit_code == Some(0) {
        if let Some(version) = version {
            let executable = lines.next().unwrap_or("").to_string();
            return ProbeOutcome::Ok { version, executable };
        }
    }
    let in_windows_apps = resolved_path
        .map(|p| p.to_ascii_lowercase().contains("windowsapps"))
        .unwrap_or(false);
    if exit_code == Some(STORE_STUB_EXIT)
        || stderr.contains("Microsoft Store")
        || (in_windows_apps && version.is_none())
    {
        return ProbeOutcome::StoreStub;
    }
    let detail = stderr.trim();
    let detail = if detail.is_empty() { stdout.trim() } else { detail };
    ProbeOutcome::Failed(format!(
        "exit {}{}",
        exit_code.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {}", detail.lines().next().unwrap_or(""))
        }
    ))
}

/// Pick the first usable candidate, or build an actionable error message
/// from everything that was tried. `results` are (label, outcome) in probe
/// order.
pub fn select(results: &[(String, ProbeOutcome)], windows: bool) -> Result<usize, String> {
    for (i, (_, outcome)) in results.iter().enumerate() {
        if let ProbeOutcome::Ok { version, .. } = outcome {
            if meets_minimum(*version) {
                return Ok(i);
            }
        }
    }
    Err(not_found_message(results, windows))
}

pub fn not_found_message(results: &[(String, ProbeOutcome)], windows: bool) -> String {
    let (maj, min) = MIN_VERSION;
    let mut msg = format!("Python {maj}.{min}+ not found.");
    let too_old: Vec<String> = results
        .iter()
        .filter_map(|(label, o)| match o {
            ProbeOutcome::Ok { version, .. } => {
                Some(format!("'{label}' is Python {}.{}.{}", version.0, version.1, version.2))
            }
            _ => None,
        })
        .collect();
    if !too_old.is_empty() {
        msg.push_str(&format!(" ({} — too old.)", too_old.join(", ")));
    }
    if windows {
        let stub = results.iter().any(|(_, o)| *o == ProbeOutcome::StoreStub);
        if stub {
            msg.push_str(" 'python' is only the Microsoft Store placeholder, not a real install.");
        }
        msg.push_str(&format!(
            " Install Python {maj}.{min} or newer from python.org with 'Add python.exe to PATH' \
             checked, and turn off the python.exe App execution alias (Settings → Apps → \
             Advanced app settings → App execution aliases). Then restart Barrekeep."
        ));
    } else {
        msg.push_str(&format!(
            " Install Python {maj}.{min} or newer so that 'python3' is on PATH, then restart Barrekeep."
        ));
    }
    let tried: Vec<String> = results
        .iter()
        .map(|(label, o)| {
            let what = match o {
                ProbeOutcome::Ok { version, .. } => format!("{}.{}.{}", version.0, version.1, version.2),
                ProbeOutcome::StoreStub => "Store placeholder".into(),
                ProbeOutcome::NotFound => "not found".into(),
                ProbeOutcome::Failed(e) => e.clone(),
            };
            format!("{label}: {what}")
        })
        .collect();
    if !tried.is_empty() {
        msg.push_str(&format!(" [tried {}]", tried.join("; ")));
    }
    msg
}

/// Locate `program` on PATH (honoring PATHEXT on Windows). Only used to
/// report the path and to spot the WindowsApps stub.
fn which(program: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_string())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return Some(candidate.display().to_string());
            }
        }
    }
    None
}

fn probe(program: &str, args: &[&str]) -> ProbeOutcome {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .args(["-c", PROBE_SCRIPT])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ProbeOutcome::NotFound,
        Err(e) => return ProbeOutcome::Failed(format!("could not start: {e}")),
    };
    // Bounded wait: a misbehaving launcher must not hang proposal generation.
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > PROBE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return ProbeOutcome::Failed("timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return ProbeOutcome::Failed(format!("wait failed: {e}")),
        }
    }
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return ProbeOutcome::Failed(format!("wait failed: {e}")),
    };
    classify_probe(
        output.status.code(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
        which(program).as_deref(),
    )
}

static CACHE: Mutex<Option<Interpreter>> = Mutex::new(None);

fn probe_all() -> Result<Interpreter, String> {
    let windows = cfg!(windows);
    let cands = candidates(windows);
    let mut results = Vec::with_capacity(cands.len());
    for (program, args) in &cands {
        let outcome = probe(program, args);
        let lbl = label(program, args);
        crate::logging::write_line("python", &format!("probe {lbl}: {outcome:?}"));
        let usable = matches!(&outcome, ProbeOutcome::Ok { version, .. } if meets_minimum(*version));
        results.push((lbl, outcome));
        if usable {
            break;
        }
    }
    let idx = select(&results, windows)?;
    let (program, args) = &cands[idx];
    let ProbeOutcome::Ok { version, executable } = &results[idx].1 else {
        unreachable!("select only returns Ok outcomes")
    };
    Ok(Interpreter {
        program: program.to_string(),
        prefix_args: args.iter().map(|a| a.to_string()).collect(),
        version: *version,
        executable: executable.clone(),
    })
}

/// The interpreter to run propose.py with. Probes once, then serves the
/// cached result; an Err is an actionable, user-facing message.
pub fn resolve() -> Result<Interpreter, String> {
    resolve_inner(false)
}

fn resolve_inner(force: bool) -> Result<Interpreter, String> {
    let mut cache = CACHE.lock().map_err(|e| e.to_string())?;
    if !force {
        if let Some(i) = cache.as_ref() {
            return Ok(i.clone());
        }
    }
    let found = probe_all();
    *cache = found.as_ref().ok().cloned();
    found
}

#[derive(Serialize, Debug)]
pub struct PythonStatus {
    pub found: bool,
    pub version: Option<String>,
    /// How it's invoked, e.g. "py -3".
    pub command: Option<String>,
    /// sys.executable of the resolved interpreter.
    pub path: Option<String>,
    pub error: Option<String>,
    pub min_version: String,
}

/// Settings → Python row. Always re-probes (that's what "Re-check" means)
/// and refreshes the cache.
#[tauri::command]
pub async fn check_python() -> PythonStatus {
    // Probing spawns processes and waits on them; keep it off the UI thread.
    let result = tauri::async_runtime::spawn_blocking(|| resolve_inner(true))
        .await
        .unwrap_or_else(|e| Err(format!("python check failed: {e}")));
    let min_version = format!("{}.{}", MIN_VERSION.0, MIN_VERSION.1);
    match result {
        Ok(i) => PythonStatus {
            found: true,
            version: Some(format!("{}.{}.{}", i.version.0, i.version.1, i.version.2)),
            command: Some(i.display()),
            path: Some(i.executable.clone()).filter(|p| !p.is_empty()),
            error: None,
            min_version,
        },
        Err(e) => PythonStatus {
            found: false,
            version: None,
            command: None,
            path: None,
            error: Some(e),
            min_version,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_order_per_platform() {
        let w: Vec<String> = candidates(true).iter().map(|(p, a)| label(p, a)).collect();
        assert_eq!(w, ["py -3", "python", "python3"]);
        let u: Vec<String> = candidates(false).iter().map(|(p, a)| label(p, a)).collect();
        assert_eq!(u, ["python3", "python"]);
    }

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("3.12.1"), Some((3, 12, 1)));
        assert_eq!(parse_version("Python 3.11.9\r\n"), Some((3, 11, 9)));
        assert_eq!(parse_version("3.13.0rc1"), Some((3, 13, 0)));
        assert_eq!(parse_version("3.14"), Some((3, 14, 0)));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("Python was not found; run without arguments"), None);
        assert_eq!(parse_version("3"), None);
    }

    #[test]
    fn minimum_is_3_11() {
        assert!(meets_minimum((3, 11, 0)));
        assert!(meets_minimum((3, 13, 2)));
        assert!(meets_minimum((4, 0, 0)));
        assert!(!meets_minimum((3, 10, 12)));
        assert!(!meets_minimum((2, 7, 18)));
    }

    #[test]
    fn classifies_real_interpreter() {
        let o = classify_probe(Some(0), "3.12.4\nC:\\Python312\\python.exe\n", "", None);
        assert_eq!(
            o,
            ProbeOutcome::Ok { version: (3, 12, 4), executable: "C:\\Python312\\python.exe".into() }
        );
        // Real Store-installed Python lives under WindowsApps and works: accept it.
        let o = classify_probe(
            Some(0),
            "3.12.4\r\nC:\\Users\\u\\AppData\\Local\\Microsoft\\WindowsApps\\PythonSoftwareFoundation.Python.3.12_x\\python.exe\r\n",
            "",
            Some("C:\\Users\\u\\AppData\\Local\\Microsoft\\WindowsApps\\python.exe"),
        );
        assert!(matches!(o, ProbeOutcome::Ok { version: (3, 12, 4), .. }));
    }

    #[test]
    fn classifies_store_stub() {
        let stub_err = "Python was not found; run without arguments to install from the Microsoft Store, or disable this shortcut from Settings > Apps > Advanced app settings > App execution aliases.";
        assert_eq!(classify_probe(Some(9009), "", stub_err, None), ProbeOutcome::StoreStub);
        assert_eq!(classify_probe(Some(9009), "", "", None), ProbeOutcome::StoreStub);
        // No version output from a WindowsApps path, whatever the exit code.
        assert_eq!(
            classify_probe(
                Some(1),
                "",
                "",
                Some("C:\\Users\\u\\AppData\\Local\\Microsoft\\WindowsApps\\python.exe")
            ),
            ProbeOutcome::StoreStub
        );
    }

    #[test]
    fn classifies_other_failures() {
        let o = classify_probe(Some(1), "", "SyntaxError: invalid syntax\nmore", None);
        assert_eq!(o, ProbeOutcome::Failed("exit 1: SyntaxError: invalid syntax".into()));
        // Exit 0 but garbage output is not an interpreter.
        assert!(matches!(classify_probe(Some(0), "hello", "", None), ProbeOutcome::Failed(_)));
    }

    fn ok(v: (u32, u32, u32)) -> ProbeOutcome {
        ProbeOutcome::Ok { version: v, executable: "x".into() }
    }

    #[test]
    fn selects_first_new_enough_candidate() {
        let r = vec![
            ("py -3".to_string(), ProbeOutcome::NotFound),
            ("python".to_string(), ok((3, 9, 1))),
            ("python3".to_string(), ok((3, 12, 0))),
        ];
        assert_eq!(select(&r, true), Ok(2));
        let r = vec![("py -3".to_string(), ok((3, 11, 0))), ("python".to_string(), ok((3, 13, 0)))];
        assert_eq!(select(&r, true), Ok(0));
    }

    #[test]
    fn store_stub_only_gives_alias_advice() {
        let r = vec![
            ("py -3".to_string(), ProbeOutcome::NotFound),
            ("python".to_string(), ProbeOutcome::StoreStub),
            ("python3".to_string(), ProbeOutcome::StoreStub),
        ];
        let e = select(&r, true).unwrap_err();
        assert!(e.starts_with("Python 3.11+ not found."), "{e}");
        assert!(e.contains("Microsoft Store placeholder"), "{e}");
        assert!(e.contains("'Add python.exe to PATH'"), "{e}");
        assert!(e.contains("App execution aliases"), "{e}");
        assert!(e.contains("restart Barrekeep"), "{e}");
        assert!(e.contains("python: Store placeholder"), "{e}");
    }

    #[test]
    fn too_old_is_named_in_the_error() {
        let r = vec![("python3".to_string(), ok((3, 10, 12))), ("python".to_string(), ProbeOutcome::NotFound)];
        let e = select(&r, false).unwrap_err();
        assert!(e.contains("'python3' is Python 3.10.12"), "{e}");
        assert!(e.contains("too old"), "{e}");
        assert!(!e.contains("App execution aliases"), "non-Windows advice only: {e}");
    }
}

# Legacy Sling scripts — do not use

`sling_extract.py` (pull), `push_to_sling.py` (push) and `rollback_push.py`
(delete a push's shifts) are the original standalone Python Sling helpers.
They are **superseded by `src-tauri/src/sling.rs` + `src-tauri/src/push_sync.rs`**
and are kept only as a reference for the request shapes they proved out.

Nothing in the app, CI or the release build runs them. They hard-code June
2026 values (dates, `viewdates`/`cachedates`, a fixed `-05:00` offset that is
wrong during CST) and know nothing about drafts, the push draft, or sync
safety (`push_result_snapshots`). Running them against a live Sling account
can create duplicate or wrong-offset shifts.

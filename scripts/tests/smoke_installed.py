#!/usr/bin/env python3
"""Smoke-test an installed Barrekeep: bundled resources + propose.py sidecar.

Usage: python scripts/tests/smoke_installed.py <install-dir>

release.yml runs this against the freshly installed MSI before the GitHub
Release is published. It would have caught v0.2.4's "could not find project
root" bug (propose.py not bundled, and the sidecar writing under the install
dir). It checks that:

  1. the resources tauri.conf.json bundles exist under the install dir;
  2. the INSTALLED propose.py runs in --json-out --from-stdin mode, fed the
     test fixture, from a fresh temp cwd (as the app does in release builds,
     where cwd is the per-user app-data dir), and prints valid JSON with
     shifts;
  3. it creates nothing under data/ (neither in the cwd nor the install dir,
     which is read-only Program Files on a real install).

Locally, the repo root has the same layout, so
`python scripts/tests/smoke_installed.py .` exercises the same checks.
"""
from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
FIXTURE = HERE / "fixture_payload.json"

# Must mirror bundle.resources in src-tauri/tauri.conf.json.
REQUIRED_RESOURCES = ("scripts/propose.py", "prompts/proposal-editor.md")


def fail(msg: str) -> None:
    print(f"SMOKE TEST FAILED: {msg}", file=sys.stderr)
    sys.exit(1)


def main(argv: list[str]) -> None:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    install_dir = pathlib.Path(argv[1]).resolve()
    if not install_dir.is_dir():
        fail(f"install dir {install_dir} does not exist")
    print(f"install dir: {install_dir}")

    for rel in REQUIRED_RESOURCES:
        path = install_dir / rel
        if not path.is_file():
            fail(f"bundled resource missing: {path}")
        print(f"ok  resource present: {rel}")

    install_data_existed = (install_dir / "data").exists()
    payload = json.loads(FIXTURE.read_text(encoding="utf-8"))
    target_month: str = payload["target_month"]

    with tempfile.TemporaryDirectory(prefix="barrekeep-smoke-") as tmp:
        cwd = pathlib.Path(tmp)
        try:
            proc = subprocess.run(
                [sys.executable, str(install_dir / "scripts" / "propose.py"),
                 "--json-out", "--from-stdin", "--target-month", target_month],
                input=json.dumps(payload).encode("utf-8"),
                cwd=cwd, capture_output=True, timeout=300)
        except subprocess.TimeoutExpired:
            fail("installed propose.py timed out after 300s")
        if proc.returncode != 0:
            fail(f"installed propose.py exited {proc.returncode}\n"
                 f"--- stderr ---\n{proc.stderr.decode('utf-8', 'replace')}")
        try:
            out = json.loads(proc.stdout)
        except json.JSONDecodeError as e:
            fail(f"propose.py stdout is not valid JSON ({e}); first 500 bytes:\n"
                 f"{proc.stdout[:500]!r}")
        if not isinstance(out, dict) or not out.get("shifts"):
            fail("propose.py JSON has no shifts for the fixture month")
        print(f"ok  propose.py --json-out: {len(out['shifts'])} shifts, "
              f"algorithm {out.get('algorithm_version')}")

        created = sorted(p.name for p in cwd.iterdir())
        if (cwd / "data").exists():
            fail(f"propose.py created data/ in its cwd (cwd now has: {created})")
        print(f"ok  nothing written to data/ in cwd (cwd contents: {created or 'empty'})")

    if not install_data_existed and (install_dir / "data").exists():
        fail("propose.py created data/ under the install dir")
    print("ok  nothing written to data/ under the install dir")
    print("SMOKE TEST PASSED")


if __name__ == "__main__":
    main(sys.argv)

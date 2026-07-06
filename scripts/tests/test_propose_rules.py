#!/usr/bin/env python3
"""propose.py rules regression: empty rules == no rules, byte-identical.

Run from anywhere: python3 scripts/tests/test_propose_rules.py
Guards the schedule-algorithm invariant that versioned rules (payload key
"rules") leave v9 output untouched when empty, and actually bite when set.
"""
import copy
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).parent
ROOT = HERE.parent.parent
payload = json.loads((HERE / "fixture_payload.json").read_text())


def run(p):
    out = subprocess.run(
        [sys.executable, "scripts/propose.py", "--json-out", "--from-stdin",
         "--target-month", p["target_month"]],
        input=json.dumps(p).encode(), cwd=ROOT, capture_output=True, check=True)
    return out.stdout


base = run(payload)
assert json.loads(base)["algorithm_version"] == "v9"
assert len(json.loads(base)["shifts"]) > 0, "fixture must produce shifts"

# 1. Empty rules are byte-identical to no rules.
with_empty_rules = copy.deepcopy(payload)
with_empty_rules["rules"] = {}
assert run(with_empty_rules) == base, "empty rules must be byte-identical to no rules"

# 2. version_label echoes through.
labeled = copy.deepcopy(payload)
labeled["version_label"] = "v10"
out = json.loads(run(labeled))
assert out["algorithm_version"] == "v10", out["algorithm_version"]

# 3. A class blocklist rule actually removes the teacher from that class.
first_shift = json.loads(base)["shifts"][0]
blocked = copy.deepcopy(payload)
blocked["rules"] = {"teacher_class_blocklist": [
    {"sling_user_id": first_shift["sling_user_id"],
     "class_name": first_shift["class_name"], "reason": "test"}]}
out2 = json.loads(run(blocked))
same_class = [s for s in out2["shifts"] if s["class_name"] == first_shift["class_name"]]
assert same_class, "blocked class slots should still exist (reassigned or dropped)"
assert all(s["sling_user_id"] != first_shift["sling_user_id"] for s in same_class), \
    "blocklisted teacher must not keep any slot of the blocked class"

# 4. A slot blocklist removes the teacher from that (weekday, time) only.
slot_blocked = copy.deepcopy(payload)
slot_blocked["rules"] = {"teacher_slot_blocklist": [
    {"sling_user_id": 501, "weekday": "Mon", "time": "09:00", "reason": "test"}]}
out3 = json.loads(run(slot_blocked))
mondays = [s for s in out3["shifts"] if s["weekday"] == "Mon" and s["start_time"] == "09:00"]
assert mondays and all(s["sling_user_id"] != 501 for s in mondays)

# 5. variety_penalty_per_class override changes the parameters echo.
tuned = copy.deepcopy(payload)
tuned["rules"] = {"variety_penalty_per_class": 0.9}
out4 = json.loads(run(tuned))
assert out4["parameters"]["variety_penalty_per_class"] == 0.9

print("OK")

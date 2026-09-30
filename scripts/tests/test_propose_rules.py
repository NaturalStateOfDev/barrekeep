#!/usr/bin/env python3
"""propose.py rules regression: empty rules == no rules, byte-identical.

Run from anywhere: python3 scripts/tests/test_propose_rules.py
Guards the schedule-algorithm invariant that versioned rules (payload key
"rules") leave v9 output untouched when empty, and actually bite when set.
"""
import ast
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

# 6. Lead overflow respects teacher_slot_blocklist. Block the non-leads from
# every class so the lead is the only candidate anywhere, then block the lead
# from Mon 09:00: that slot must drop, not fall back to "LEAD OVERFLOW".
lead_uid = next(t["sling_user_id"] for t in payload["teachers"] if t["is_lead"])
others = [t["sling_user_id"] for t in payload["teachers"] if not t["is_lead"]]
classes = sorted({s["class_name"] for s in json.loads(base)["shifts"]})
lead_blocked = copy.deepcopy(payload)
lead_blocked["rules"] = {
    "teacher_class_blocklist": [
        {"sling_user_id": u, "class_name": c} for u in others for c in classes],
    "teacher_slot_blocklist": [
        {"sling_user_id": lead_uid, "weekday": "Mon", "time": "09:00"}],
}
out5 = json.loads(run(lead_blocked))
mon9 = [s for s in out5["shifts"] if s["weekday"] == "Mon" and s["start_time"] == "09:00"]
assert mon9, "Mon 09:00 slots must still be reported (as dropped)"
assert all(s["sling_user_id"] != lead_uid for s in mon9), \
    "lead overflow must not assign the lead to a slot-blocklisted slot"
assert all(s["is_dropped"] for s in mon9), [s["generation_reason"] for s in mon9]

# 7. Studio timezone (_USCentral) matches US Central time hour by hour,
#    including fold handling, 2026-2028. The reference is IANA
#    America/Chicago via zoneinfo when the tz database is available; stock
#    Windows Python (CI) has none without the tzdata package, so there the
#    reference is the published 2026-2028 transition instants. propose.py
#    itself stays stdlib-only for Windows.
from datetime import datetime, timedelta, timezone

_src = (ROOT / "scripts" / "propose.py").read_text()
_node = next(n for n in ast.parse(_src).body
             if isinstance(n, ast.ClassDef) and n.name == "_USCentral")
_ns = {}
exec("from datetime import datetime, timedelta, tzinfo\n"
     + ast.get_source_segment(_src, _node), _ns)
central = _ns["_USCentral"]()

try:
    from zoneinfo import ZoneInfo
    chicago = ZoneInfo("America/Chicago")
except Exception:  # ZoneInfoNotFoundError: no system tz db and no tzdata
    chicago = None

# (spring-forward, fall-back) in UTC: 2am local -> 08:00Z / 07:00Z.
_TRANSITIONS = [
    (datetime(2026, 3, 8, 8, tzinfo=timezone.utc), datetime(2026, 11, 1, 7, tzinfo=timezone.utc)),
    (datetime(2027, 3, 14, 8, tzinfo=timezone.utc), datetime(2027, 11, 7, 7, tzinfo=timezone.utc)),
    (datetime(2028, 3, 12, 8, tzinfo=timezone.utc), datetime(2028, 11, 5, 7, tzinfo=timezone.utc)),
]


def ref_utc(t):
    """(naive wall clock, utcoffset, fold) for UTC instant t."""
    if chicago is not None:
        r = t.astimezone(chicago)
        return r.replace(tzinfo=None), r.utcoffset(), r.fold
    dst = any(a <= t < b for a, b in _TRANSITIONS)
    off = timedelta(hours=-5 if dst else -6)
    fold = int(any(b <= t < b + timedelta(hours=1) for _, b in _TRANSITIONS))
    return (t + off).replace(tzinfo=None), off, fold


t = datetime(2026, 1, 1, tzinfo=timezone.utc)
while t < datetime(2029, 1, 1, tzinfo=timezone.utc):
    ours = t.astimezone(central)
    wall, off, fold = ref_utc(t)
    assert ours.replace(tzinfo=None) == wall, (t, ours, wall)
    assert ours.utcoffset() == off, (t, ours.utcoffset(), off)
    assert ours.fold == fold, (t, ours.fold, fold)
    t += timedelta(minutes=30)
# Wall-clock -> offset (what date.replace(hour=...) relies on).
for y, mo, d, h, cdt in [(2026, 11, 1, 0, True), (2026, 11, 1, 9, False),
                         (2026, 10, 31, 23, True), (2027, 3, 14, 1, False),
                         (2027, 3, 14, 5, True), (2027, 3, 13, 9, False)]:
    w = datetime(y, mo, d, h)
    want = timedelta(hours=-5 if cdt else -6)
    if chicago is not None:
        assert w.replace(tzinfo=chicago).utcoffset() == want, w
    assert w.replace(tzinfo=central).utcoffset() == want, w

# 8. End to end across fall-back: CST-offset history keeps its wall-clock
#    slot (09:00, not 08:00), and a CST availability block given in UTC
#    blocks the 09:00 class it overlaps.
def shift(date, off, uid=501):
    return {"type": "shift", "dtstart": f"{date}T09:00:00{off}",
            "dtend": f"{date}T10:00:00{off}", "user": {"id": uid},
            "position": {"id": 29303965}, "location": {"id": 901}}

winter = copy.deepcopy(payload)
winter["target_month"] = "2026-12"
winter["history_events"] = [shift(d, "-05:00") for d in ("2026-10-05", "2026-10-12", "2026-10-19")] \
    + [shift(d, "-06:00") for d in ("2026-11-02", "2026-11-09", "2026-11-16")]
winter["month_events"] = [{"type": "leave", "dtstart": "2026-12-07T15:00:00Z",
                           "dtend": "2026-12-07T16:00:00Z", "user": {"id": 501}}]
w_out = json.loads(run(winter))
mon = [s for s in w_out["shifts"] if s["weekday"] == "Mon"]
assert mon and all(s["start_time"] == "09:00" for s in mon), \
    [(s["shift_date"], s["start_time"]) for s in mon]
dec7 = [s for s in mon if s["shift_date"] == "2026-12-07"]
assert dec7 and all(s["sling_user_id"] != 501 or s["is_dropped"] for s in dec7), dec7
assert any(s["sling_user_id"] == 501 for s in mon if s["shift_date"] == "2026-12-14"), \
    "501 should keep the unblocked Mondays"

print("OK")

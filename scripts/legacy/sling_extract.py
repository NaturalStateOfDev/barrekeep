"""
Sling Extract Script v2 (Read-Only)
------------------------------------
Pulls April 2026 shifts and May 2026 unavailability for the 12 home location
teachers. Uses the correct Sling endpoint pattern discovered via DevTools:

    GET /v1/{orgId}/calendar/{orgId}/users/{actingUserId}
        ?dates=START/END

The endpoint returns a single feed with mixed event types (shift / leave /
availability), partitioned by the `type` field. Because the acting user is
an admin (Teacher A), the response includes events for ALL users -- we filter
client-side to our 12 teachers.

We also pull /availability for the underlying recurring rrules, in case
teachers use weekly availability patterns rather than one-off time-off.

=============================================================
READ-ONLY GUARANTEE
=============================================================
Only GET requests. No writes. The http() helper hard-refuses anything
other than GET.
=============================================================

Usage:
    pip install requests
    python sling_extract.py

Outputs:
    - sling_extract.json     (full raw API responses)
    - april_shifts.csv       (April 2026 shifts for our 12 teachers)
    - may_unavailability.csv (May 2026 leave + availability blocks)
    - may_availability.csv   (inverted: when each teacher IS available)
    - april_summary.txt      (human-readable April pattern by teacher)
"""

import csv
import json
import sys
import time
from collections import defaultdict
from datetime import datetime, timedelta, timezone
from getpass import getpass

import requests

ORG_ID = 0
ACTING_USER_ID = 1001  # Teacher A -- has manager role, full visibility
BASE_URL = "https://api.getsling.com/v1"

HOME_LOCATION_ID = 0

TEACHERS = {
    1006: "Teacher F",
    1005: "Teacher E",
    1003: "Teacher C",
    1012: "Teacher L",
    1002: "Teacher B",
    1009: "Teacher I",
    1004: "Teacher D",
    1007: "Teacher G",
    1008: "Teacher H",
    1011: "Teacher K",
    1010: "Teacher J",
    1001: "Teacher A",
}

POSITION_NAMES = {
    29470407: "Empower",
    29470419: "Focus",
    29470489: "Breaking Down the Barre",
    29303958: "Align",
    29303965: "Classic",
    29304030: "Define",
    29304197: "Reform",
    29303535: "Teacher",
    29303536: "Sales Rep",
}

# Date ranges -- Sling expects ISO 8601 with timezone offset
# America/Chicago is UTC-5 in CDT (April through early November)
APRIL_START = "2026-04-01T00:00:00-05:00"
APRIL_END   = "2026-05-01T00:00:00-05:00"
MAY_START   = "2026-05-01T00:00:00-05:00"
MAY_END     = "2026-06-01T00:00:00-05:00"

TZ = timezone(timedelta(hours=-5))

# Daily business-hours window for the inversion (we'll confirm with Teacher A)
BIZ_HOURS_START = 5   # 5 AM
BIZ_HOURS_END = 21    # 9 PM


def http(method: str, path: str, token: str, params: dict | None = None) -> dict | list:
    """HTTP helper -- read-only guard. Refuses any non-GET."""
    if method.upper() != "GET":
        raise RuntimeError(f"Refusing {method} -- read-only script.")

    # The "nonce" query param Sling uses is just a cache-buster
    if params is None:
        params = {}
    params.setdefault("nonce", str(int(time.time() * 1000)))

    resp = requests.get(
        f"{BASE_URL}{path}",
        headers={
            "Authorization": token,
            "Accept": "*/*",
            "Origin": "https://app.getsling.com",
        },
        params=params,
        timeout=45,
    )
    print(f"  GET {path} -> {resp.status_code} ({len(resp.content)} bytes)")
    if resp.status_code == 401:
        print("  Token rejected -- grab a fresh one from DevTools.", file=sys.stderr)
        sys.exit(1)
    if resp.status_code != 200:
        return {"error": resp.status_code, "body": resp.text[:500]}
    try:
        return resp.json()
    except Exception:
        return {"error": "non-JSON response", "body": resp.text[:500]}


def fetch_calendar(token: str, start: str, end: str, label: str) -> list:
    """Pull the calendar feed for a date range -- returns mixed event types."""
    print(f"\nFetching {label} calendar...")
    path = f"/{ORG_ID}/calendar/{ORG_ID}/users/{ACTING_USER_ID}"
    params = {"dates": f"{start}/{end}", "user-fields": "id"}
    result = http("GET", path, token, params)
    if isinstance(result, list):
        print(f"  Got {len(result)} events")
        return result
    if isinstance(result, dict) and "error" in result:
        print(f"  ERROR: {result}")
        return []
    # Some endpoints wrap the array
    if isinstance(result, dict):
        for key in ("events", "data", "items"):
            if key in result and isinstance(result[key], list):
                print(f"  Got {len(result[key])} events (wrapped under '{key}')")
                return result[key]
    return []


def fetch_availability_rules(token: str) -> list:
    """Pull recurring availability rules (rrule-based, may include unavailability)."""
    print("\nFetching availability rules...")
    result = http("GET", f"/{ORG_ID}/availability", token)
    if isinstance(result, list):
        print(f"  Got {len(result)} availability rules")
        return result
    if isinstance(result, dict):
        for key in ("availability", "rules", "data"):
            if key in result and isinstance(result[key], list):
                print(f"  Got {len(result[key])} rules (under '{key}')")
                return result[key]
    if isinstance(result, dict) and "error" in result:
        print(f"  Could not fetch availability rules: {result.get('error')}")
    return []


def event_user_id(event: dict) -> int | None:
    """Extract user ID from an event regardless of nested vs flat shape."""
    user = event.get("user")
    if isinstance(user, dict):
        return user.get("id")
    return event.get("userId") or event.get("user")


def event_location_id(event: dict) -> int | None:
    loc = event.get("location")
    if isinstance(loc, dict):
        return loc.get("id")
    return event.get("locationId")


def event_position_id(event: dict) -> int | None:
    pos = event.get("position")
    if isinstance(pos, dict):
        return pos.get("id")
    return event.get("positionId")


def is_home_teacher_event(event: dict) -> bool:
    """Filter: event for one of our 12 teachers, at home location (or no location)."""
    uid = event_user_id(event)
    if uid not in TEACHERS:
        return False
    loc_id = event_location_id(event)
    # Allow no-location events (time-off often has none)
    if loc_id and loc_id != HOME_LOCATION_ID:
        return False
    return True


def fmt_dt(s: str | None) -> str:
    if not s:
        return ""
    try:
        return datetime.fromisoformat(s.replace("Z", "+00:00")).astimezone(TZ).strftime("%Y-%m-%d %H:%M")
    except Exception:
        return s


def write_shifts_csv(events: list, path: str) -> int:
    """Write all 'shift' type events for our teachers."""
    rows = []
    for e in events:
        if (e.get("type") or "").lower() != "shift":
            continue
        if not is_home_teacher_event(e):
            continue
        uid = event_user_id(e)
        pos_id = event_position_id(e)
        rows.append({
            "teacher": TEACHERS.get(uid, f"unknown-{uid}"),
            "user_id": uid,
            "start": fmt_dt(e.get("dtstart") or e.get("start")),
            "end": fmt_dt(e.get("dtend") or e.get("end")),
            "weekday": (datetime.fromisoformat((e.get("dtstart") or e.get("start") or "1970-01-01").replace("Z", "+00:00")).astimezone(TZ).strftime("%a") if e.get("dtstart") or e.get("start") else ""),
            "class_type": POSITION_NAMES.get(pos_id, ""),
            "position_id": pos_id,
            "status": e.get("status", ""),
            "published": e.get("published", ""),
            "notes": (e.get("notes") or "")[:120],
        })
    rows.sort(key=lambda r: (r["start"], r["teacher"]))
    with open(path, "w", newline="") as f:
        if not rows:
            f.write("no shifts found\n")
            return 0
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)
    print(f"  Wrote {len(rows)} shifts -> {path}")
    return len(rows)


def write_timeoff_csv(events: list, path: str) -> list:
    """Write all 'leave' / time-off / unavailability events for our teachers."""
    rows = []
    raw_blocks = []  # for inversion later
    for e in events:
        ev_type = (e.get("type") or "").lower()
        if ev_type not in ("leave", "timeoff", "time_off", "unavailability"):
            continue
        if not is_home_teacher_event(e):
            continue
        uid = event_user_id(e)
        rows.append({
            "teacher": TEACHERS.get(uid, f"unknown-{uid}"),
            "user_id": uid,
            "start": fmt_dt(e.get("dtstart") or e.get("start")),
            "end": fmt_dt(e.get("dtend") or e.get("end")),
            "all_day": e.get("allDay", False),
            "type": ev_type,
            "status": e.get("status", ""),
            "reason": (e.get("notes") or e.get("reason") or "")[:120],
        })
        raw_blocks.append((uid, e.get("dtstart") or e.get("start"), e.get("dtend") or e.get("end")))
    rows.sort(key=lambda r: (r["teacher"], r["start"]))
    with open(path, "w", newline="") as f:
        if not rows:
            f.write("no timeoff found\n")
            return []
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)
    print(f"  Wrote {len(rows)} unavailability blocks -> {path}")
    return raw_blocks


def invert_to_availability(blocks: list, month_start: datetime, month_end: datetime) -> dict:
    """For each teacher, compute available windows = biz hours minus unavailability."""
    by_teacher = defaultdict(list)
    for uid, start_str, end_str in blocks:
        if not (start_str and end_str):
            continue
        try:
            s = datetime.fromisoformat(start_str.replace("Z", "+00:00")).astimezone(TZ)
            e = datetime.fromisoformat(end_str.replace("Z", "+00:00")).astimezone(TZ)
            by_teacher[uid].append((s, e))
        except Exception as ex:
            print(f"  Skipping malformed block: {ex}")

    availability = {}
    for uid, name in TEACHERS.items():
        teacher_blocks = sorted(by_teacher.get(uid, []))
        avail = []
        day = month_start
        while day < month_end:
            day_open = day.replace(hour=BIZ_HOURS_START, minute=0)
            day_close = day.replace(hour=BIZ_HOURS_END, minute=0)
            free = [(day_open, day_close)]
            for bs, be in teacher_blocks:
                if be <= day_open or bs >= day_close:
                    continue
                new_free = []
                for fs, fe in free:
                    if be <= fs or bs >= fe:
                        new_free.append((fs, fe))
                        continue
                    if bs > fs:
                        new_free.append((fs, bs))
                    if be < fe:
                        new_free.append((be, fe))
                free = new_free
            for fs, fe in free:
                if fe > fs:
                    avail.append((fs, fe))
            day += timedelta(days=1)
        availability[name] = avail
    return availability


def write_availability_csv(availability: dict, path: str):
    rows = []
    for teacher, windows in availability.items():
        for s, e in windows:
            rows.append({
                "teacher": teacher,
                "date": s.strftime("%Y-%m-%d"),
                "weekday": s.strftime("%a"),
                "available_from": s.strftime("%H:%M"),
                "available_to": e.strftime("%H:%M"),
                "hours": round((e - s).total_seconds() / 3600, 2),
            })
    rows.sort(key=lambda r: (r["teacher"], r["date"], r["available_from"]))
    with open(path, "w", newline="") as f:
        if not rows:
            f.write("no availability computed\n")
            return
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)
    print(f"  Wrote {len(rows)} availability windows -> {path}")


def write_april_summary(events: list, path: str):
    """Human-readable April pattern: who taught what, when, by weekday."""
    by_teacher = defaultdict(lambda: defaultdict(list))
    for e in events:
        if (e.get("type") or "").lower() != "shift":
            continue
        if not is_home_teacher_event(e):
            continue
        uid = event_user_id(e)
        try:
            s = datetime.fromisoformat((e.get("dtstart") or e.get("start")).replace("Z", "+00:00")).astimezone(TZ)
            end = datetime.fromisoformat((e.get("dtend") or e.get("end")).replace("Z", "+00:00")).astimezone(TZ)
        except Exception:
            continue
        weekday = s.strftime("%A")
        time_str = f"{s.strftime('%H:%M')}-{end.strftime('%H:%M')}"
        cls = POSITION_NAMES.get(event_position_id(e), "?")
        by_teacher[TEACHERS[uid]][weekday].append((time_str, cls, s.strftime("%m/%d")))

    weekday_order = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
    with open(path, "w") as f:
        f.write("APRIL 2026 SCHEDULE PATTERN -- Example Barre Studio\n")
        f.write("=" * 60 + "\n\n")
        for teacher in sorted(by_teacher.keys()):
            f.write(f"\n{teacher}\n")
            f.write("-" * len(teacher) + "\n")
            for wd in weekday_order:
                if wd not in by_teacher[teacher]:
                    continue
                f.write(f"  {wd}:\n")
                # Group by time slot to see which dates that slot was taught
                by_slot = defaultdict(list)
                for time_str, cls, date_str in by_teacher[teacher][wd]:
                    by_slot[(time_str, cls)].append(date_str)
                for (time_str, cls), dates in sorted(by_slot.items()):
                    f.write(f"    {time_str}  {cls:25s}  ({len(dates)}x: {', '.join(sorted(dates))})\n")
    print(f"  Wrote April pattern summary -> {path}")


def main():
    print("Paste your Authorization token from DevTools:")
    token = getpass("Authorization token: ").strip()
    if not token:
        print("No token provided.", file=sys.stderr)
        sys.exit(1)

    # Sanity check token first
    print("\n[1] Verifying token (account/session)...")
    session = http("GET", f"/{ORG_ID}/account/session", token)
    if isinstance(session, dict) and session.get("user"):
        print(f"  OK -- logged in as {session['user'].get('name')}")
    else:
        print(f"  Unexpected response: {str(session)[:200]}")

    # Pull both months from the calendar endpoint
    april_events = fetch_calendar(token, APRIL_START, APRIL_END, "April 2026")
    may_events = fetch_calendar(token, MAY_START, MAY_END, "May 2026")

    # Recurring availability rules (separate endpoint)
    avail_rules = fetch_availability_rules(token)

    # Save raw payload for transparency
    with open("sling_extract.json", "w") as f:
        json.dump({
            "session": session,
            "april_events": april_events,
            "may_events": may_events,
            "availability_rules": avail_rules,
            "teachers": TEACHERS,
        }, f, indent=2, default=str)
    print("\nRaw payload -> sling_extract.json")

    # Quick event-type breakdown for visibility
    def types_of(events):
        c = defaultdict(int)
        for e in events:
            c[e.get("type", "?")] += 1
        return dict(c)
    print(f"\nApril event types: {types_of(april_events)}")
    print(f"May event types:   {types_of(may_events)}")

    # April: shifts CSV + human summary
    print("\nWriting April outputs...")
    n_shifts = write_shifts_csv(april_events, "april_shifts.csv")
    write_april_summary(april_events, "april_summary.txt")

    # May: unavailability + inverted availability
    print("\nWriting May outputs...")
    may_blocks = write_timeoff_csv(may_events, "may_unavailability.csv")
    print("  Inverting unavailability -> availability windows...")
    may_start_dt = datetime(2026, 5, 1, tzinfo=TZ)
    may_end_dt = datetime(2026, 6, 1, tzinfo=TZ)
    availability = invert_to_availability(may_blocks, may_start_dt, may_end_dt)
    write_availability_csv(availability, "may_availability.csv")

    print("\n" + "=" * 60)
    print("DONE")
    print("=" * 60)
    print("Files generated:")
    print("  - sling_extract.json      (full raw API responses)")
    print("  - april_shifts.csv        (April schedule, one row per shift)")
    print("  - april_summary.txt       (human-readable pattern by teacher)")
    print("  - may_unavailability.csv  (raw timeoff blocks)")
    print("  - may_availability.csv    (inverted availability windows)")
    print(f"\nApril shifts found for our 12 teachers: {n_shifts}")
    print("\nUpload all 5 files back to chat for the May draft.")


if __name__ == "__main__":
    main()

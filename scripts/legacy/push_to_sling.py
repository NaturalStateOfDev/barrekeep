"""
push_to_sling.py - push v10 schedule to Sling as planning-status shifts

Usage:
  python push_to_sling.py --csv june_proposed_v10.csv --dry-run
  python push_to_sling.py --csv june_proposed_v10.csv --execute

Required env vars (in .env or shell):
  SLING_TOKEN        - bearer token from DevTools (Authorization header value)
  SLING_ORG_ID       - 0
  SLING_ACTING_USER  - 1001 (Teacher A, used for the GET path)

Reads june_proposed_v10.csv, dedupes against existing planning shifts at home location,
and POSTs only the missing ones. Co-teach rows (e.g. "Teacher A + Teacher E")
become two separate POST calls.

Always run --dry-run first. Always.
"""
from __future__ import annotations
import argparse, csv, json, os, sys, time
from datetime import datetime, timezone, timedelta
from urllib.parse import quote
from urllib.request import Request, urlopen
from urllib.error import HTTPError, URLError

# ============================================================
# Constants (from recon + project memory)
# ============================================================
ORG_ID = "0"
HOME_LOCATION_ID = 0
ACTING_USER_ID = 1001  # Teacher A
TZ = timezone(timedelta(hours=-5))  # Central, no DST adjustment for now

# Roster: name -> Sling user id
ROSTER = {
    "Teacher A": 1001,
    "Teacher B": 1002,
    "Teacher C": 1003,
    "Teacher D": 1004,
    "Teacher E": 1005,
    "Teacher F": 1006,
    "Teacher G": 1007,
    "Teacher H": 1008,
    "Teacher I": 1009,
    "Teacher J": 1010,
}

# Position mapping: class name -> Sling position id
POSITIONS = {
    "Empower": 29470407,
    "Focus": 29470419,
    "Breaking Down the Barre": 29470489,
    "Align": 29303958,
    "Classic": 29303965,
    "Define": 29304030,
    "Reform": 29304197,
}

# Batching: Sling rate-limits aggressively. Push N at a time, then pause.
BATCH_SIZE = 10
INTRA_BATCH_DELAY = 1.0       # seconds between requests within a batch
INTER_BATCH_DELAY = 10.0      # seconds between batches
RATE_LIMIT_BACKOFF = 30.0     # seconds to wait after a 429 before retrying
MAX_RETRIES_PER_REQUEST = 3   # how many times to retry a single shift on 429

# View window: covers the full June 2026 month plus padding
VIEWDATES = "2026-05-31T00:00:00-0500/2026-07-05T00:00:00-0500"
CACHEDATES = "2026-05-30T00:00:00-0500/2026-07-06T00:00:00-0500"

# ============================================================
# Sling HTTP helpers
# ============================================================

class SlingClient:
    def __init__(self, token: str, org_id: str):
        self.token = token
        self.org_id = org_id
        self.base = "https://api.getsling.com"

    def _request(self, method: str, path: str, query: dict | None = None,
                 body: dict | None = None) -> tuple[int, dict | list | None]:
        """Single request. Returns (status, parsed_json_or_none)."""
        url = self.base + path
        if query:
            qs = "&".join(f"{k}={quote(str(v), safe='')}" for k, v in query.items())
            url += "?" + qs
        # Browser-like headers to satisfy Cloudflare WAF.
        # Sling's API is browser-only; without these Cloudflare returns 1010.
        headers = {
            "Authorization": self.token,
            "Accept": "application/json, text/plain, */*",
            "Accept-Language": "en-US,en;q=0.9",
            "User-Agent": (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
                "AppleWebKit/537.36 (KHTML, like Gecko) "
                "Chrome/131.0.0.0 Safari/537.36"
            ),
            "Origin": "https://app.getsling.com",
            "Referer": "https://app.getsling.com/",
            "sec-ch-ua": '"Chromium";v="131", "Not_A Brand";v="24"',
            "sec-ch-ua-mobile": "?0",
            "sec-ch-ua-platform": '"Windows"',
            "Sec-Fetch-Dest": "empty",
            "Sec-Fetch-Mode": "cors",
            "Sec-Fetch-Site": "same-site",
        }
        data = None
        if body is not None:
            headers["Content-Type"] = "application/json;charset=UTF-8"
            data = json.dumps(body).encode("utf-8")
        req = Request(url, data=data, method=method, headers=headers)
        try:
            with urlopen(req) as resp:
                status = resp.status
                raw = resp.read().decode("utf-8") if resp.length != 0 else ""
                parsed = json.loads(raw) if raw else None
                return status, parsed
        except HTTPError as e:
            raw = e.read().decode("utf-8", errors="replace") if e.fp else ""
            try:
                parsed = json.loads(raw) if raw else None
            except json.JSONDecodeError:
                parsed = {"error": raw}
            return e.code, parsed

    def list_calendar(self, start_iso: str, end_iso: str,
                      acting_user: int) -> list[dict]:
        """GET planning + published shifts/leaves/availability for the org."""
        path = f"/v1/{self.org_id}/calendar/{self.org_id}/users/{acting_user}"
        query = {
            "dates": f"{start_iso}/{end_iso}",
            "user-fields": "id",
            "nonce": str(int(time.time() * 1000)),
        }
        status, body = self._request("GET", path, query=query)
        if status != 200:
            raise RuntimeError(f"Calendar GET failed: {status} {body}")
        return body or []

    def create_shift(self, dtstart: str, dtend: str, user_id: int,
                     position_id: int, location_id: int,
                     status: str = "planning") -> dict:
        """POST a new shift. Returns the created shift dict (unwrapped from array).
        Retries on 429 with backoff."""
        path = f"/v1/{self.org_id}/shifts"
        query = {
            "user-fields": "id",
            "checkRestBreakConflicts": "true",
            "viewdates": VIEWDATES,
            "cachedates": CACHEDATES,
            "checkConsecutiveWorkDaysConflicts": "true",
        }
        body = {
            "location": {"id": location_id},
            "dtstart": dtstart,
            "dtend": dtend,
            "users": [{"id": user_id}],   # NB: array on create
            "slots": 1,
            "position": {"id": position_id},
            "status": status,
        }
        last_err = None
        for attempt in range(1, MAX_RETRIES_PER_REQUEST + 1):
            code, resp = self._request("POST", path, query=query, body=body)
            if code in (200, 201):
                if isinstance(resp, list) and resp:
                    return resp[0]
                return resp or {}
            if code == 429:
                # Rate limited -- back off and retry
                wait = RATE_LIMIT_BACKOFF * attempt
                print(f"      rate limited, backing off {wait:.0f}s "
                      f"(attempt {attempt}/{MAX_RETRIES_PER_REQUEST})...")
                time.sleep(wait)
                last_err = f"429 (after {attempt} retries): {resp}"
                continue
            # Non-retryable error
            raise RuntimeError(f"Create failed {code}: {resp}")
        raise RuntimeError(f"Create failed after {MAX_RETRIES_PER_REQUEST} retries: {last_err}")

    def delete_shift(self, shift_id: str) -> bool:
        """DELETE a shift. Returns True on 204."""
        path = f"/v1/{self.org_id}/shifts/{shift_id}"
        query = {"viewdates": VIEWDATES, "cachedates": CACHEDATES}
        code, _ = self._request("DELETE", path, query=query)
        return code == 204


# ============================================================
# v10 CSV -> intended shifts
# ============================================================

def load_v10_csv(path: str) -> list[dict]:
    """Return list of shift specs. Co-teach rows expand to multiple specs.
    Accepts either 'teacher' (widget export) or 'proposed_teacher' (server export)."""
    rows = list(csv.DictReader(open(path)))
    if not rows:
        return []
    teacher_col = "proposed_teacher" if "proposed_teacher" in rows[0] else "teacher"
    if teacher_col not in rows[0]:
        raise RuntimeError(
            f"CSV missing teacher column. Found columns: {list(rows[0].keys())}. "
            f"Expected 'teacher' or 'proposed_teacher'.")
    specs = []
    for r in rows:
        teacher = r[teacher_col]
        if teacher in ("DROPPED", ""):
            continue

        cls = r["class"]
        if cls not in POSITIONS:
            print(f"  ! Skipping unknown class: {cls} on {r['date']} {r['start']}")
            continue
        position_id = POSITIONS[cls]

        dtstart = f"{r['date']}T{r['start']}"
        dtend = f"{r['date']}T{r['end']}"

        teachers = teacher.split(" + ") if " + " in teacher else [teacher]
        for tname in teachers:
            if tname not in ROSTER:
                print(f"  ! Skipping unknown teacher: {tname} on {r['date']} {r['start']}")
                continue
            specs.append({
                "date": r["date"],
                "start": r["start"],
                "end": r["end"],
                "class": cls,
                "teacher_name": tname,
                "user_id": ROSTER[tname],
                "position_id": position_id,
                "dtstart": dtstart,
                "dtend": dtend,
                "is_coteach": len(teachers) > 1,
            })
    return specs


# ============================================================
# Dedupe: existing shifts -> set of (date, start_HHMM, user_id, position_id)
# ============================================================

def fingerprint(dtstart_iso: str, user_id: int, position_id: int,
                location_id: int) -> tuple:
    """Stable key for matching v10 specs against existing Sling shifts."""
    # Sling returns dtstart as "2026-06-01T05:45:00-05:00"
    # We want (date, HH:MM, user_id, position_id, location_id)
    if "T" in dtstart_iso:
        date_part, time_part = dtstart_iso.split("T", 1)
        # strip seconds and timezone
        time_hhmm = time_part[:5]
    else:
        date_part = dtstart_iso[:10]
        time_hhmm = "00:00"
    return (date_part, time_hhmm, user_id, position_id, location_id)


def existing_shifts_at_home(client: SlingClient,
                                 acting_user: int) -> dict[tuple, dict]:
    """Map fingerprint -> existing shift dict for all June home location shifts."""
    cal = client.list_calendar(
        "2026-05-31T00:00:00-05:00",
        "2026-07-05T00:00:00-05:00",
        acting_user,
    )
    out = {}
    for ev in cal:
        if ev.get("type") != "shift":
            continue
        loc = ev.get("location") or {}
        if loc.get("id") != HOME_LOCATION_ID:
            continue
        # only count planning + published (don't double up on either)
        if ev.get("status") not in ("planning", "published"):
            continue
        user = ev.get("user") or {}
        pos = ev.get("position") or {}
        if not user.get("id") or not pos.get("id"):
            continue
        key = fingerprint(ev.get("dtstart", ""), user["id"], pos["id"],
                           loc["id"])
        out[key] = ev
    return out


# ============================================================
# Main
# ============================================================

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--csv", required=True,
                        help="Path to v10 CSV")
    parser.add_argument("--dry-run", action="store_true",
                        help="Show what would happen, don't push")
    parser.add_argument("--execute", action="store_true",
                        help="Actually push to Sling")
    parser.add_argument("--log", default="push_log.json",
                        help="Path to write audit log")
    args = parser.parse_args()

    if not args.dry_run and not args.execute:
        print("ERROR: pass either --dry-run or --execute")
        sys.exit(1)
    if args.dry_run and args.execute:
        print("ERROR: pass either --dry-run or --execute, not both")
        sys.exit(1)

    token = os.environ.get("SLING_TOKEN")
    if not token:
        print("ERROR: set SLING_TOKEN env var with the Authorization header value")
        print("  (grab fresh from DevTools - tokens expire)")
        sys.exit(1)

    org_id = os.environ.get("SLING_ORG_ID", ORG_ID)
    acting_user = int(os.environ.get("SLING_ACTING_USER", str(ACTING_USER_ID)))

    client = SlingClient(token, org_id)

    # 1. Load intended shifts from v10
    print(f"Loading v10 from {args.csv}...")
    specs = load_v10_csv(args.csv)
    print(f"  Intended shifts: {len(specs)}")
    coteach_count = sum(1 for s in specs if s["is_coteach"])
    print(f"  (includes {coteach_count} co-teach assignments)")

    # 2. Read existing Sling shifts for dedupe
    print(f"\nReading existing Sling shifts at home location...")
    try:
        existing = existing_shifts_at_home(client, acting_user)
    except Exception as e:
        print(f"ERROR fetching existing shifts: {e}")
        sys.exit(2)
    print(f"  Existing planning+published shifts at home location: {len(existing)}")

    # 3. Compute diff
    to_create = []
    skipped = []
    for spec in specs:
        key = (spec["date"], spec["start"], spec["user_id"],
               spec["position_id"], HOME_LOCATION_ID)
        if key in existing:
            skipped.append((spec, existing[key]))
        else:
            to_create.append(spec)

    print(f"\nDiff vs Sling:")
    print(f"  Already present (will skip): {len(skipped)}")
    print(f"  To create: {len(to_create)}")

    # 4. Show plan
    print(f"\n{'=' * 60}")
    print(f"PLAN ({'DRY RUN' if args.dry_run else 'EXECUTE'})")
    print(f"{'=' * 60}")
    for spec in to_create[:10]:
        coteach = " [co-teach]" if spec["is_coteach"] else ""
        print(f"  CREATE  {spec['date']} {spec['start']}-{spec['end']} "
              f"{spec['class']:24s} -> {spec['teacher_name']}{coteach}")
    if len(to_create) > 10:
        print(f"  ... and {len(to_create) - 10} more")

    if args.dry_run:
        print(f"\n(Dry run: no requests sent. Re-run with --execute to actually push.)")
        return

    # 5. Confirm before execute
    n = len(to_create)
    n_batches = (n + BATCH_SIZE - 1) // BATCH_SIZE
    # Estimate: each batch = BATCH_SIZE * INTRA_BATCH_DELAY of sending,
    # plus INTER_BATCH_DELAY between batches (n_batches - 1 gaps).
    est_seconds = (n * INTRA_BATCH_DELAY) + ((n_batches - 1) * INTER_BATCH_DELAY)
    print(f"\nAbout to POST {n} shifts to Sling in batches of {BATCH_SIZE}")
    print(f"  ~{INTRA_BATCH_DELAY:.0f}s between requests within a batch")
    print(f"  ~{INTER_BATCH_DELAY:.0f}s pause between batches")
    print(f"  Estimated total time: {est_seconds:.0f}s "
          f"(~{est_seconds/60:.1f} min) if no rate limiting")
    confirm = input("Type 'PUSH' to proceed, anything else to abort: ")
    if confirm.strip() != "PUSH":
        print("Aborted.")
        return

    # 6. Push in batches with throttle and audit log
    log = []
    succeeded = 0
    failed = 0
    print()
    for batch_idx in range(n_batches):
        batch_start = batch_idx * BATCH_SIZE
        batch = to_create[batch_start:batch_start + BATCH_SIZE]
        print(f"--- Batch {batch_idx + 1}/{n_batches} "
              f"(shifts {batch_start + 1}-{batch_start + len(batch)}) ---")
        for j, spec in enumerate(batch):
            i = batch_start + j + 1
            try:
                result = client.create_shift(
                    dtstart=spec["dtstart"],
                    dtend=spec["dtend"],
                    user_id=spec["user_id"],
                    position_id=spec["position_id"],
                    location_id=HOME_LOCATION_ID,
                    status="planning",
                )
                shift_id = result.get("id", "?")
                entry = {
                    "ok": True, "spec": spec, "shift_id": shift_id,
                    "ts": datetime.now(TZ).isoformat(),
                }
                log.append(entry)
                succeeded += 1
                print(f"  [{i}/{n}] OK  id={shift_id}  "
                      f"{spec['date']} {spec['start']} {spec['class']} "
                      f"-> {spec['teacher_name']}")
            except Exception as e:
                entry = {
                    "ok": False, "spec": spec, "error": str(e),
                    "ts": datetime.now(TZ).isoformat(),
                }
                log.append(entry)
                failed += 1
                print(f"  [{i}/{n}] FAIL {e}  "
                      f"{spec['date']} {spec['start']} {spec['class']} "
                      f"-> {spec['teacher_name']}")
            # Always write log incrementally so a crash doesn't lose progress
            with open(args.log, "w") as f:
                json.dump(log, f, indent=2)
            # Intra-batch delay (skip after last item in batch)
            if j < len(batch) - 1:
                time.sleep(INTRA_BATCH_DELAY)
        # Inter-batch delay (skip after last batch)
        if batch_idx < n_batches - 1:
            print(f"  ... pausing {INTER_BATCH_DELAY:.0f}s before next batch ...")
            time.sleep(INTER_BATCH_DELAY)

    print(f"\n{'=' * 60}")
    print(f"DONE: {succeeded} created, {failed} failed")
    print(f"Audit log: {args.log}")
    print(f"{'=' * 60}")
    if failed:
        print(f"\nFailures listed above. To roll back successful pushes, "
              f"use the rollback script with {args.log}.")


if __name__ == "__main__":
    main()

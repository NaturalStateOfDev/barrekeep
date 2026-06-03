"""
rollback_push.py - delete shifts created by a previous push run

Usage:
  python rollback_push.py --log push_log.json --dry-run
  python rollback_push.py --log push_log.json --execute

Reads push_log.json, finds the shift IDs of successful creates, and DELETEs them.
"""
import argparse, json, os, sys, time
from urllib.parse import quote
from urllib.request import Request, urlopen
from urllib.error import HTTPError

ORG_ID = "0"
RATE_LIMIT_SECONDS = 1.0
VIEWDATES = "2026-05-31T00:00:00-0500/2026-07-05T00:00:00-0500"
CACHEDATES = "2026-05-30T00:00:00-0500/2026-07-06T00:00:00-0500"


def delete_shift(token: str, org_id: str, shift_id: str) -> tuple[int, str]:
    url = (f"https://api.getsling.com/v1/{org_id}/shifts/{shift_id}"
           f"?viewdates={quote(VIEWDATES, safe='')}"
           f"&cachedates={quote(CACHEDATES, safe='')}")
    req = Request(url, method="DELETE", headers={
        "Authorization": token,
        "Accept": "application/json, text/plain, */*",
        "Accept-Language": "en-US,en;q=0.9",
        "User-Agent": (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
            "AppleWebKit/537.36 (KHTML, like Gecko) "
            "Chrome/131.0.0.0 Safari/537.36"
        ),
        "Origin": "https://app.getsling.com",
        "Referer": "https://app.getsling.com/",
        "Sec-Fetch-Dest": "empty",
        "Sec-Fetch-Mode": "cors",
        "Sec-Fetch-Site": "same-site",
    })
    try:
        with urlopen(req) as resp:
            return resp.status, ""
    except HTTPError as e:
        body = e.read().decode("utf-8", errors="replace") if e.fp else ""
        return e.code, body


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--log", required=True)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()

    if not args.dry_run and not args.execute:
        print("ERROR: pass --dry-run or --execute")
        sys.exit(1)

    token = os.environ.get("SLING_TOKEN")
    if not token:
        print("ERROR: set SLING_TOKEN env var")
        sys.exit(1)

    log = json.load(open(args.log))
    successful = [e for e in log if e.get("ok") and e.get("shift_id")]
    print(f"Log has {len(log)} entries, {len(successful)} successful creates")

    if args.dry_run:
        print(f"\nDRY RUN. Would DELETE {len(successful)} shifts:")
        for e in successful[:10]:
            spec = e["spec"]
            print(f"  id={e['shift_id']}  {spec['date']} {spec['start']} "
                  f"{spec['class']} -> {spec['teacher_name']}")
        if len(successful) > 10:
            print(f"  ... and {len(successful) - 10} more")
        return

    confirm = input(f"Type 'ROLLBACK' to delete {len(successful)} shifts: ")
    if confirm.strip() != "ROLLBACK":
        print("Aborted.")
        return

    deleted = failed = 0
    for i, entry in enumerate(successful, 1):
        sid = entry["shift_id"]
        spec = entry["spec"]
        code, body = delete_shift(token, ORG_ID, sid)
        if code == 204:
            deleted += 1
            print(f"  [{i}/{len(successful)}] DEL ok  id={sid}")
        else:
            failed += 1
            print(f"  [{i}/{len(successful)}] DEL FAIL {code}  id={sid}  {body}")
        time.sleep(RATE_LIMIT_SECONDS)
    print(f"\nDeleted {deleted}, failed {failed}")


if __name__ == "__main__":
    main()

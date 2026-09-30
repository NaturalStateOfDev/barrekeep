# Sling API reference

Everything the team has reverse-engineered about Sling's API by watching DevTools and running production pushes. This is not an official spec — Sling has no public API documentation. Verify against live behavior before making structural changes.

## Org and location identifiers

These are studio-specific and configured at runtime (Settings → Studio
configuration; stored in the `studio_config` table). They are NOT compiled in.
They are detected automatically from `account/session`, `users/concise` and
`groups` after a Sling login; otherwise find them in a Sling DevTools session —
see the calendar request URL.

| Thing | Where it comes from |
|---|---|
| Organization ID | runtime config (`studio_config.org_id`) |
| Acting user ID (admin calendar feed) | runtime config (`studio_config.acting_user_id`) |
| Home location ID | runtime config (`studio_config.home_location_id`) |
| Other locations | filtered out (anything that isn't the home location) |

## Authentication

- **Token type:** opaque bearer string in the `Authorization` header
- **How to obtain:** log into `https://app.getsling.com`, open DevTools → Network, find any request to `api.getsling.com`, copy the `Authorization` header value
- **Expiration:** unknown but tokens have died mid-session. Always grab fresh before a push.
- **Storage:** Stronghold (OS keychain). Never `.env`, never git, never DuckDB.

## Cloudflare WAF

Sling sits behind Cloudflare. Default HTTP-client user-agents (Python, ureq) are blocked with a 1010 error. All requests must include browser-like headers:

```
User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ...
Origin: https://app.getsling.com
Referer: https://app.getsling.com/
Sec-Fetch-Dest: empty
Sec-Fetch-Mode: cors
Sec-Fetch-Site: same-site
```

See the header helpers in `src-tauri/src/sling.rs` for the full working header set.

## Endpoints

### GET calendar (read shifts, leaves, availability)

```
GET /v1/{orgId}/calendar/{orgId}/users/{actingUserId}
  ?dates=<startISO>/<endISO>
  &user-fields=id
  &nonce=<epoch-ms>
```

Returns array of events with `type` ∈ `{"shift", "leave", "availability"}`.

**Critical: `availability` events represent BLOCKED time, not available time.** The naming is backward.

### POST shift (create new shift, planning status)

```
POST /v1/{orgId}/shifts
  ?user-fields=id
  &checkRestBreakConflicts=true
  &viewdates=<startISO>/<endISO>
  &cachedates=<startISO>/<endISO>
  &checkConsecutiveWorkDaysConflicts=true
```

Body:

```json
{
  "location": {"id": "<home_location_id>"},
  "dtstart": "2026-06-01T05:45",
  "dtend": "2026-06-01T06:45",
  "users": [{"id": "<teacher_user_id>"}],
  "slots": 1,
  "position": {"id": "<position_id>"},
  "status": "planning"
}
```

**Note:** `users` is an array on POST, but `user` (singular) on PUT and in responses. Don't symmetrize.

`dtstart`/`dtend` are sent as naive local time strings (no timezone offset). Sling echoes them back with the timezone applied (`-05:00` during CDT, `-06:00` during CST). Query-side offsets (`dates=`, `viewdates`/`cachedates`) must likewise use each date's own America/Chicago offset — see `sling::month_range` / `view_cache_dates`.

Returns array of one shift on success (200/201). Unwrap `resp[0]`.

### PUT shift (update existing) — NOT used by the app

```
PUT /v1/{orgId}/shifts/{shiftId}?publish=false&...same query params as POST
```

Body uses `user: {id}` singular. Always send `publish=false` to keep the shift in planning status.

**Unverified.** No PUT request has been captured from the Sling web client in
full or exercised by this project — the notes above are all we know (the full
body shape Sling expects, and whether a partial body clobbers other fields, are
unknown). The incremental push (`src-tauri/src/push_sync.rs`) therefore does
**not** PUT: an "update" is a DELETE of the old shift followed by a POST of the
new one. Consequence: an updated shift gets a new Sling id (tracked in
`push_results`, see below). Capture a real PUT from DevTools before switching.

### DELETE shift

```
DELETE /v1/{orgId}/shifts/{shiftId}
  ?viewdates=<startISO>/<endISO>
  &cachedates=<startISO>/<endISO>
```

Same browser-like headers as POST (plus `Accept: application/json, text/plain, */*`,
`Accept-Language: en-US,en;q=0.9`). No body.

Returns **204 with an empty body** on success — don't parse JSON. The app treats
**404** as "already gone" (deleted in Sling's UI or by an earlier run), not an
error. 401/429 as elsewhere. Ported from the legacy `scripts/legacy/rollback_push.py` into
`sling::delete_shift` (same 429 backoff as creates: 30s/60s/90s, max 3 tries).

## Incremental push (sync) and safety rules

`push_sync.rs`, migration 0013. What the app may touch in Sling:

- **Only shifts it created.** Each create records `push_results` (outcome
  `created`/`updated`/`adopted`, with `sling_shift_id`) plus a
  `push_result_snapshots` row of exactly what was sent. The latest tracking row
  per Sling id decides whether the app still owns it (`deleted` /
  `skipped_missing` end tracking). A shift with no tracking row is never
  updated or deleted.
- **Only planning, unmodified shifts.** Before any update/delete the month's
  calendar is fetched (one GET) and each shift must still exist, be
  `status: "planning"`, sit at the home location, and match its snapshot
  (teacher, position, date, start, end). Otherwise it's skipped with a reason
  (`skipped_conflict`, or `skipped_missing` when it's gone). Pushes made before
  0013 have no snapshot: if Sling shows one as planning, at the home location
  and exactly equal to its current draft shift, executing the push records
  Sling's state as its snapshot (an `adopted` row, no Sling call) and it is
  syncable from then on. Otherwise it's listed as "pushed before sync
  tracking; differs from draft — fix in Sling or remove manually" and never
  touched. A tracked shift deleted in Sling is shown as "deleted in Sling since
  last push — will be re-created on the next push unless you remove it from
  the draft" (recorded `skipped_missing`, which ends tracking).
- **Plan = create / update / delete / unchanged** per proposal shift, matched
  by (proposal_shift_id, teacher) — a co-teach slot is two tracked shifts.
  Identical shifts owned by another draft of the month are *adopted* (tracked
  for the push draft, no Sling call). Other drafts' remaining shifts are offered
  for cleanup (explicit, default off); cleanup deletes run before creates.
- **Dedupe still applies to creates**, using the same fingerprints as before,
  minus shifts this plan deletes (so a slot replaced in place isn't mistaken
  for a duplicate of itself).
- Execution order: adopt/skip bookkeeping → cleanup deletes → deletes →
  replacements (DELETE then POST) → creates; rate-limited across all calls
  (1s apart, 10s after every 10th). Execute re-plans from a fresh fetch and
  refuses if the plan differs from the confirmed preview.

## Availability refresh

`refresh_availability_from_sling` re-pulls, for every month from the current
one on that has a pull or a draft: `/users/concise` + `/groups` (roster sync)
and one calendar GET per month (1s apart). It rewrites that month's
`availability_blocks` and `external_sling_shifts` and bumps
`month_pulls.pulled_at` — drafts are NOT regenerated. `check_draft_conflicts`
then re-validates a draft (blocked/leave overlap, deactivated, unqualified,
over weekly cap, unassigned) and records `draft_checks.checked_at`, which clears
the stale banner. Overlaps are computed with US Central DST rules, not the
fixed `-05:00` the calendar query uses.

## Rate limiting

- **Observed limit:** approximately 20 requests per minute. After ~20 rapid requests, Sling returns `429 Too many requests`.
- **Recovery time:** ~30 seconds (sometimes longer)
- **Strategy:** batch in 10s (1s between calls, 10s pause between batches — `push_sync.rs` `BATCH_SIZE` / `INTRA_DELAY_SECS` / `INTER_DELAY_SECS`); on 429, linear backoff (30s, 60s, 90s) up to 3 retries per shift (`sling.rs` `PUSH_MAX_RETRIES`).

## Position IDs (the studio)

| Class type | Position ID |
|---|---|
| Empower | 29470407 |
| Focus | 29470419 |
| Breaking Down the Barre | 29470489 |
| Align | 29303958 |
| Classic | 29303965 |
| Define | 29304030 |
| Reform | 29304197 |

Excluded from auto-scheduling: `29303535` (legacy "Teacher"), `29303536` ("Sales Rep").

## Teacher qualifications source of truth

Teachers' qualifications come from their `groupIds` in the user object. Position groups exist for each class type. To check if teacher T can teach class C, check whether T's `groupIds` includes C's position ID.

This is more reliable than inferring qualifications from past teaching history, which is incomplete (e.g., a teacher cleared for Define who hasn't yet taught it).

## Idempotency

There is no idempotency key. Sling will happily create duplicate shifts at the same time slot for the same teacher. The app must dedupe client-side by:

1. Reading existing shifts at the target location for the target month
2. Building fingerprints `(date, HH:MM, user_id, position_id, location_id)`
3. Only POSTing shifts whose fingerprint isn't already present

## Co-teaching

Sling has no co-teach concept. Two teachers in the same time slot = two separate shift records at that slot. The app handles this by expanding co-teach rows into multiple POSTs.

## Recurrence

Existing shifts may have `rrule` fields (weekly recurring shifts). The push doesn't create rrules — every shift it creates is single-occurrence. If a recurring April shift extends into June, its instances will appear in the calendar GET as if they were individually created. Dedupe handles this correctly.

## Things we don't know

- Whether Sling has a "publish all planning shifts" API. Currently the manager publishes via the web UI.
- Whether the rate limit is per-token or per-org or per-IP.
- Whether `checkRestBreakConflicts=true` and `checkConsecutiveWorkDaysConflicts=true` change validation behavior or just UI feedback. The app sends both as `true` to match the Sling web client exactly.
- Whether Sling's API has any way to get notification settings or send a notification programmatically.

If you need any of these, capture the relevant request from the Sling web client's DevTools and document here.

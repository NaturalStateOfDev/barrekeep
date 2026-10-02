# Sling API reference

What this app relies on in Sling's API.

**Sling does publish an API spec**: Swagger 2.0 at https://api.getsling.com/v1/spec.json (browsable at https://api.getsling.com/). Use it as the reference for endpoints, parameters and object shapes. It is thin, though — it does not document wire formats (e.g. an availability set's `interval`), defaults (calendar page size), rate limits, or the Cloudflare header requirements. This file records what has been verified on top of the spec by watching DevTools and running production pulls/pushes, and flags what is still assumed. Verify against live behavior before making structural changes.

A note on paths: the spec's base path is `/v1` and it lists e.g. `/calendar/{org_id}/users/{user_id}` and `/availability`. The web client (and this app) call the calendar and shift endpoints with an extra org prefix — `/v1/{orgId}/calendar/{orgId}/users/{userId}`, `/v1/{orgId}/shifts` — which is what has been verified to work. `/users/concise` and `/groups` are called without it.

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
  &pageSize=500
  &page=<n>
```

Returns array of events with `type` ∈ `{"shift", "leave", "availability"}`.

**Critical: `availability` events represent BLOCKED time, not available time.** The naming is backward.

**Event ids may be non-numeric strings** (the spec types the availability-event id as a string). `CalendarEvent.id` parses to `None` in that case rather than failing the event — an availability/leave event needs no id, and dropping it would hide blocked time. Events that fail to parse at all are counted and reported as a pull warning.

**Paging.** The spec lists `pageSize` ("number of results to return") and `page` on this endpoint, with no default page size and no word on whether `page` counts from 0 or 1. Before migration 0014 the app sent neither and took whatever came back, so a server-side default could have truncated a busy month silently. `sling::fetch_calendar_paged` now sends `pageSize=500&page=0,1,…` and assumes nothing:

- if Sling rejects the paging params (any 4xx/5xx other than 401/429), it repeats the request without them — the previous behaviour;
- a response larger than the page size means paging is ignored and everything already arrived — stop;
- otherwise it keeps requesting pages, de-duplicating by event id, until a page is empty or two pages in a row add nothing new (one repeat is expected if `page` is 1-based, where page 0 and page 1 are the same page). A short page is deliberately NOT treated as the last page: a server cap below 500 would look identical.

So a typical calendar fetch costs 2–3 GETs instead of 1. Which case Sling actually is can be read off the raw pull file (`calendar … page N` entries). **Unverified against a live session at the time of writing** — check the first real raw pull.

Other documented query params (not used): `eventTypes`, `userIds`, `locationIds`, `positionIds`, `showPlanningEvents`, `skipUnscheduled`, `groupBy`.

### GET availability sets (recurring unavailability)

```
GET /v1/{orgId}/availability?userId=<teacherId>     ← tried first
GET /v1/availability?userId=<teacherId>             ← the spec's form; fallback
```

Teachers enter their standing unavailability ("I can't teach Tuesdays 9:45") as a recurring **availability set**. These do NOT reliably appear in the calendar feed — before migration 0014 the app read only `/calendar`, so most teachers' recurring unavailability never arrived and the scheduler treated them as free. One GET per roster teacher (`sling::fetch_availability_sets`), through the paced pull session.

The spec documents the bare path; the app's other org-scoped calls take an org prefix and the retired Python extractor used `/{orgId}/availability`. Both forms are supported: the org-prefixed one is tried first, a 404/405 falls back to the other, and whichever answers is used for the remaining teachers and logged (`availability sets: the … URL form answered`; also `summary.availability_path_form` in the raw pull file). A set that names a different user than the one requested is ignored (it arrives with its owner's request). If neither form answers for the first two teachers the rest are skipped and a warning is shown.

Response: an array of `AvailabilitySet` (a wrapped array — `{"data": […]}` etc. — or a single object is also accepted):

```json
{
  "id": 9001,
  "name": "Fall mornings",
  "start": "2026-09-01T00:00:00-05:00",
  "until": null,
  "interval": "…",
  "approved": "2026-08-30T14:00:00-05:00",
  "user": {"id": 1930004},
  "availabilities": [
    {"id": "…", "dtstart": "2026-09-01T09:45:00-05:00", "dtend": "2026-09-01T10:45:00-05:00",
     "summary": "", "fullDay": false}
  ]
}
```

How the app reads it (`src-tauri/src/availability.rs`) — **these are assumptions until checked against a real raw pull**:

- Like calendar `availability` events, every entry is **unavailable** time.
- Each entry of `availabilities` gives a weekday + local time range (America/Chicago). It repeats every `interval` from its own `dtstart`, keeping the wall-clock time across DST changes. `fullDay` = the whole local day(s).
- Occurrences before the `start` date or after the `until` date are dropped; `until` is inclusive (the safer reading) and `null` = forever.
- `interval` is documented only as "recurrence interval i.e. every week, every two weeks, etc." The parser accepts: numbers 1–6 (weeks) and multiples of 604800 (seconds), also as strings; ISO-8601 `P1W` / `P2W` / `P7D` / `P14D`; RRULE fragments `FREQ=WEEKLY;INTERVAL=2` (and `FREQ=DAILY`); words (`weekly`, `every week`, `biweekly`, `fortnightly`, `every other week`, `every two weeks`, `2 weeks`, `daily`); Python-timedelta text (`7 days, 0:00:00`). A bare `7` or `14` is ambiguous (weeks? days?) and is NOT guessed.
- Anything else (including a missing interval or unreadable dates) makes the set **uninterpreted**: it is stored with a `problem`, produces no blocks, and the pull shows "N availability sets from Sling couldn't be interpreted — schedule may miss unavailability; see raw pull file". Extend `parse_interval` (with a test) when a new spelling turns up.
- `approved: null` (or absent) = pending approval. Pending sets still block — scheduling over a pending request is the worse mistake — but are stored as `availability_set_pending` and labelled "pending approval".

Sets are stored raw in `sling_availability_sets` (replaced per teacher on each successful fetch; a teacher whose fetch failed keeps the previous sets) and expanded into `availability_blocks` for each pulled month, skipping occurrences identical to a block the calendar already supplied.

Related, unused: `GET /availability/{set_id}`, `GET /availability/event/{event_id}`, `GET /calendar/available`.

### Raw pull files

Every pull and availability refresh writes what Sling returned — every calendar page and every availability response, each with its URL, query and outcome — to `<app_local_data>/raw_pulls/<UTC timestamp>-<month or refresh-range>.json` (`raw_pulls.rs`; Settings → Backups → "Open raw pulls folder"). The newest 20 are kept. A failed pull writes `…-failed.json` with whatever arrived. No request headers are stored, the bearer token is scrubbed from the text, and the roster response (contact details) is left out.

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
one on that has a pull or a draft: `/users/concise` + `/groups` (roster sync),
the month's calendar (paged) and — once, not per month — every roster
teacher's availability sets, all through one paced `PullSession` (1s apart,
10s after every 10th GET). It rewrites that month's `availability_blocks`
(calendar blocks + expanded sets), `teacher_availability_windows` and
`external_sling_shifts` and bumps
`month_pulls.pulled_at` — drafts are NOT regenerated. `check_draft_conflicts`
then re-validates a draft (blocked/leave overlap, deactivated, unqualified,
over weekly cap, unassigned) and records `draft_checks.checked_at`, which clears
the stale banner. Overlaps are computed with US Central DST rules, not the
fixed `-05:00` the calendar query uses.

## Rate limiting

- **Observed limit:** approximately 20 requests per minute. After ~20 rapid requests, Sling returns `429 Too many requests`.
- **Recovery time:** ~30 seconds (sometimes longer)
- **Strategy:** batch in 10s (1s between calls, 10s pause between batches — `push_sync.rs` `BATCH_SIZE` / `INTRA_DELAY_SECS` / `INTER_DELAY_SECS`); on 429, linear backoff (30s, 60s, 90s) up to 3 retries per shift (`sling.rs` `PUSH_MAX_RETRIES`).
- **Pulls** use the same cadence via `sling::PullSession` (1s between GETs, 10s after every 10th, 30s/60s backoff on 429, three tries). A month pull is roughly 2 + 2×(2–3) + one GET per teacher ≈ 20 requests, so expect it to take about half a minute.

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

- The wire format of an availability set's `interval`, whether `until` is inclusive, and whether an entry can be marked as *available/preferred* rather than unavailable (the app assumes every entry is unavailability). Check a raw pull file.
- Which URL form `/availability` answers on, and whether `?userId=` is honoured for an admin token on both.
- The calendar endpoint's default page size and whether `page` is 0- or 1-based.

- Whether Sling has a "publish all planning shifts" API. Currently the manager publishes via the web UI.
- Whether the rate limit is per-token or per-org or per-IP.
- Whether `checkRestBreakConflicts=true` and `checkConsecutiveWorkDaysConflicts=true` change validation behavior or just UI feedback. The app sends both as `true` to match the Sling web client exactly.
- Whether Sling's API has any way to get notification settings or send a notification programmatically.

If you need any of these, capture the relevant request from the Sling web client's DevTools and document here.

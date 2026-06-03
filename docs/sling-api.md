# Sling API reference

Everything the team has reverse-engineered about Sling's API by watching DevTools and running production pushes. This is not an official spec — Sling has no public API documentation. Verify against live behavior before making structural changes.

## Org and location identifiers

These are studio-specific and configured at runtime (Settings → Studio
configuration; stored in the `studio_config` table). They are NOT compiled in.
Find your values in a Sling DevTools session — see the calendar request URL.

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

Sling sits behind Cloudflare. Default Python user-agent is blocked with a 1010 error. All requests must include browser-like headers:

```
User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ...
Origin: https://app.getsling.com
Referer: https://app.getsling.com/
Sec-Fetch-Dest: empty
Sec-Fetch-Mode: cors
Sec-Fetch-Site: same-site
```

See `scripts/push_to_sling.py` for the full working header set.

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

`dtstart`/`dtend` are sent as naive local time strings (no timezone offset). Sling echoes them back with the timezone applied (`-05:00`).

Returns array of one shift on success (200/201). Unwrap `resp[0]`.

### PUT shift (update existing)

```
PUT /v1/{orgId}/shifts/{shiftId}?publish=false&...same query params as POST
```

Body uses `user: {id}` singular. Always send `publish=false` to keep the shift in planning status.

### DELETE shift

```
DELETE /v1/{orgId}/shifts/{shiftId}
  ?viewdates=<startISO>/<endISO>
  &cachedates=<startISO>/<endISO>
```

Returns 204 with empty body on success.

## Rate limiting

- **Observed limit:** approximately 20 requests per minute. After ~20 rapid requests, Sling returns `429 Too many requests`.
- **Recovery time:** ~30 seconds (sometimes longer)
- **Strategy:** batch in 10s with 10 second pauses between batches; on 429, exponential backoff (30s, 60s, 90s) up to 3 retries per shift.

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

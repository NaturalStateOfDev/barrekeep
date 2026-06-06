---
name: sling-integration
description: Use this skill when modifying any code that talks to the Sling API — pull, push, dedupe, rate-limit handling, or auth. Sling's API is brittle, undocumented, and protected by Cloudflare WAF.
---

# Working on Sling integration

Sling has no public API. Everything we know is in `docs/sling-api.md`. Read it before changing any code in `scripts/sling_*.py` or `src/lib/sling.ts`.

## Always do these

1. **Read `docs/sling-api.md` for the endpoint shape** before assuming. The POST and PUT shapes are subtly different (array vs singular). Responses are always arrays.

2. **Throttle aggressively.** Sling rate-limits at ~20 requests/minute. The current scripts use batches of 10 with 10-second pauses and exponential backoff on 429. Don't loosen these without re-testing against a real Sling session.

3. **Always send browser-like headers.** Cloudflare's WAF blocks any request that doesn't look like a real browser. The User-Agent, Origin, Referer, and Sec-Fetch-* headers are mandatory.

4. **Audit-log every request.** For pushes, write to `pushes` and `push_results` tables. For pulls, write the raw JSON to a timestamped file in `data/raw_pulls/` for forensics.

## Never do these

- **Never push with `status: "published"`.** All pushes from this app create planning-status shifts. Manager publishes from Sling's web UI.

- **Never assume idempotency.** Sling will create duplicate shifts if you POST the same shift twice. Always read existing shifts and dedupe by `(date, HH:MM, user_id, position_id, location_id)` before pushing.

- **Never store the bearer token in plaintext.** Stronghold (OS keychain) only.

- **Never strip the `viewdates` and `cachedates` query params.** They're not just bookkeeping — Sling's server uses them to invalidate cached views. Omitting them causes UI inconsistency for users who have Sling open in another tab.

- **Never extend the rate limit retries beyond 3.** If a single shift fails 3 times, log it and move on. The user can re-run the push and dedupe will handle the rest.

## When the API behavior changes

Sling can update their API at any time without notice. If the existing scripts start failing:

1. **Capture a fresh request from Sling's web UI** with DevTools open. Compare against `docs/sling-api.md`. Update the doc with what changed.
2. **Update the scripts** to match the new shape.
3. **Add a note to `docs/decisions/`** documenting the change.

## Token refresh

There is no programmatic token refresh. The user must:

1. Log into Sling in a browser
2. Open DevTools → Network
3. Find a request to `api.getsling.com`, copy the Authorization header value
4. Paste it into the app's settings (which writes to Stronghold)

The app should detect 401 errors and surface a clear "your token expired, please refresh" message with a button that opens the Sling site.

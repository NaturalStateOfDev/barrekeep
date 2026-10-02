---
name: sling-integration
description: Use this skill when modifying any code that talks to the Sling API — pull, push, dedupe, rate-limit handling, or auth. Sling's API is brittle, only partly documented (Swagger spec at https://api.getsling.com/), and protected by Cloudflare WAF.
---

# Working on Sling integration

Sling publishes a Swagger 2.0 spec for its API (UI: https://api.getsling.com/, JSON: https://api.getsling.com/v1/spec.json). It lists endpoints, parameters and object shapes, but it is thin: wire formats (e.g. the availability-set `interval`), defaults (calendar page size), rate limits and the Cloudflare requirements are not in it. What we have verified on top of the spec is in `docs/sling-api.md`. Read both before changing `src-tauri/src/sling.rs` (HTTP, pull, push primitives, dedupe) or `src-tauri/src/push_sync.rs` (incremental push / sync). The Python scripts in `scripts/legacy/` are retired reference copies — never edit or run them.

## Always do these

1. **Read `docs/sling-api.md` (and the spec) for the endpoint shape** before assuming. The POST and PUT shapes are subtly different (array vs singular). Responses are always arrays. Where the spec is silent, parse defensively and surface what you could not interpret — never drop it silently (see `availability::parse_interval`).

2. **Throttle aggressively.** Sling rate-limits at ~20 requests/minute. `push_sync.rs` uses batches of 10 with 10-second pauses (`BATCH_SIZE` / `INTER_DELAY_SECS`) and `sling.rs` backs off 30s/60s/90s on 429. Pull-side GETs go through `sling::PullSession`, which applies the same cadence (1s apart, 10s after every 10th) — add new GETs to the session, not as bare `http_get` calls. Don't loosen these without re-testing against a real Sling session.

3. **Always send browser-like headers.** Cloudflare's WAF blocks any request that doesn't look like a real browser. The User-Agent, Origin, Referer, and Sec-Fetch-* headers are mandatory.

4. **Audit-log every request.** For pushes, write to `pushes` and `push_results` tables. For pulls, the raw JSON goes to a timestamped file in `<app_local_data>/raw_pulls/` (`raw_pulls.rs`; newest 20 kept; no auth headers, token scrubbed, roster response excluded) — pass `record = true` to `PullSession::get` for any new pull endpoint.

## Never do these

- **Never push with `status: "published"`.** All pushes from this app create planning-status shifts. Manager publishes from Sling's web UI.

- **Never assume idempotency.** Sling will create duplicate shifts if you POST the same shift twice. Always read existing shifts and dedupe by `(date, HH:MM, user_id, position_id, location_id)` before pushing.

- **Never store the bearer token in plaintext.** Stronghold (OS keychain) only.

- **Never strip the `viewdates` and `cachedates` query params.** They're not just bookkeeping — Sling's server uses them to invalidate cached views. Omitting them causes UI inconsistency for users who have Sling open in another tab.

- **Never extend the rate limit retries beyond 3.** If a single shift fails 3 times, log it and move on. The user can re-run the push and dedupe will handle the rest.

## When the API behavior changes

Sling can update their API at any time without notice. If the pull or push starts failing:

1. **Check the newest raw pull file, the spec, and a fresh request from Sling's web UI** (DevTools open). Compare against `docs/sling-api.md`. Update the doc with what changed.
2. **Update `sling.rs`** (and its fixture tests) to match the new shape.
3. **Add a note to `docs/decisions/`** documenting the change.

## Token refresh

There is no programmatic token refresh. The user either logs in again via the
in-app Sling login window (`sling_login.rs` captures the bearer token), or
copies the Authorization header from a DevTools session on `api.getsling.com`
and pastes it into Settings. Either way it is stored in Stronghold.

The app should detect 401 errors and surface a clear "your token expired, please refresh" message with a button that opens the Sling site.

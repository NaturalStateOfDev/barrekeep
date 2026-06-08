// Roster seeding is intentionally disabled: teachers, positions, and
// qualifications are now sourced from Sling (Settings → log in + set studio
// config, then "Refresh from Sling" on the Teachers page, or run a month
// pull).  A fresh install starts with no placeholder data so it's immediately
// obvious whether Sling is actually connected.

use duckdb::Connection;

pub fn run_if_empty(_conn: &Connection) -> anyhow::Result<()> {
    // Intentionally empty: roster, positions, and qualifications are now
    // sourced from Sling (Settings → log in + set studio config, then
    // "Refresh from Sling" on the Teachers page, or run a month pull).
    // A fresh install starts with no teachers/positions on purpose — no
    // placeholder data that obscures whether Sling is actually connected.
    Ok(())
}

---
name: schema-change
description: Use this skill whenever a change to the DuckDB schema is needed — adding columns, adding tables, renaming fields, changing constraints. Migrations must be written as forward-only files with explicit version numbers. Backfills must be idempotent.
---

# How to make a schema change

The DuckDB schema is the source of truth for app state. Changes must be migration-driven, never ad-hoc.

## Steps

1. **Update `docs/data-model.md` first.** Document the change in the markdown DDL before touching code. This is the human-readable canonical reference.

2. **Add a migration file at `src/lib/migrations/NNN_short_description.ts`** where NNN is the next sequential number. Format:

   ```typescript
   import type { Migration } from '../duckdb';

   export const migration: Migration = {
     version: 23,
     description: "Add coteach_partner_shift_id to proposal_shifts",
     async up(conn) {
       await conn.run(`
         ALTER TABLE proposal_shifts
         ADD COLUMN coteach_partner_shift_id BIGINT REFERENCES proposal_shifts(id)
       `);
     },
   };
   ```

3. **Add a backfill if needed.** If existing rows need values for the new column, add a second statement in the migration that's idempotent (uses `WHERE column IS NULL` etc.).

4. **Update affected TypeScript types in `src/types.ts`.** The schedule type, the proposal type, etc.

5. **Update affected queries.** `src/lib/queries.ts` is the canonical location.

6. **Test by deleting `data/scheduler.duckdb` and rerunning the app.** Confirm seed data + migrations bring the DB to the expected state.

## Things to avoid

- **Never DROP a column.** Add a new column instead, mark the old one deprecated in `data-model.md`, and stop reading from it. We can prune deprecated columns in a quarterly cleanup migration.
- **Never alter a primary key.** If you need a different PK, create a new table and migrate rows.
- **Don't use DuckDB-specific syntax that won't work in plain SQL.** This DB might one day move to Postgres or SQLite for some features. Keep DDL portable.
- **Don't write migrations that fail if run twice.** All `CREATE` should be `CREATE TABLE IF NOT EXISTS`, all `ALTER ADD COLUMN` should be guarded with a column-exists check.

## When the change touches Sling integration

If a column maps to a Sling API field (e.g., `sling_user_id`, `sling_position_id`), update `docs/sling-api.md` too. Cross-reference the migration in the doc.

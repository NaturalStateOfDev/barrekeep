---
name: schema-change
description: Use this skill whenever a change to the DuckDB schema is needed — adding columns, adding tables, renaming fields, changing constraints. Migrations must be written as forward-only files with explicit version numbers. Backfills must be idempotent.
---

# How to make a schema change

The DuckDB schema is the source of truth for app state. Changes must be
migration-driven, never ad-hoc. Migrations are plain SQL files in
`src-tauri/migrations/`, registered in `src-tauri/src/migrations.rs`, and
applied once at startup (tracked in the `_migrations` table).

## Steps

1. **Update `docs/data-model.md` first.** Document the change in the markdown
   DDL before touching code. This is the human-readable canonical reference.

2. **Add a migration file at `src-tauri/migrations/NNNN_short_description.sql`**
   where NNNN is the next sequential number. Start the file with a comment
   block explaining what and WHY (see 0003 and 0009 for the expected depth).

3. **Register it in `src-tauri/src/migrations.rs`** — append a
   `Migration { version, label, sql: include_str!(...) }` entry. Versions are
   monotonically increasing; once shipped, never edit a migration's SQL —
   write a new one.

4. **Add a backfill if needed.** If existing rows need values for a new
   column, make the backfill statement idempotent (`WHERE column IS NULL`,
   `INSERT ... SELECT ... WHERE NOT EXISTS`, etc.).

5. **Update affected Rust structs and queries** in `src-tauri/src/commands.rs`
   and the TypeScript types in `src/types.ts`.

6. **Add a test in `migrations.rs`'s `#[cfg(test)]` module** exercising the
   new shape on a fresh in-memory DB (`migrations::run` + representative
   data). `cargo test --lib migrations` must pass.

## DuckDB constraint rules (learned the hard way — migrations 0003/0004/0009)

- **Never put UNIQUE constraints (beyond the PK) or incoming FKs on a table
  whose rows get UPDATEd.** DuckDB executes an UPDATE that touches an indexed
  column as DELETE+INSERT; the delete half trips incoming FK references with
  "still referenced by a foreign key in a different table".
- **DuckDB has no `ALTER TABLE DROP CONSTRAINT`.** Removing a constraint
  means rebuilding the table: `CREATE TABLE t_new (...)`,
  `INSERT INTO t_new SELECT ...`, `DROP TABLE t`, `RENAME`. Recreate any
  indexes; sequences keep their high-water marks. Rebuild referencing tables
  first if they hold FKs into the one being dropped.
- **Validate destructive migrations against the shipped engine version
  before committing.** `pip install duckdb==<engine>` (the engine version is
  encoded in the crate version: `1.1MMPP` = libduckdb `1.MM.PP`, see
  `src-tauri/Cargo.toml`), run the full migration chain + realistic data, and
  assert row counts/ids/sequences are preserved.

## Things to avoid

- **Never DROP a column.** Add a new column instead, mark the old one
  deprecated in `data-model.md`, and stop reading from it.
- **Never alter a primary key.** If you need a different PK, create a new
  table and migrate rows.
- **Don't write migrations that fail if run twice.** Additive DDL should be
  `CREATE ... IF NOT EXISTS` / `ADD COLUMN IF NOT EXISTS`. (Table-rebuild
  migrations can't be statement-idempotent — they rely on the `_migrations`
  version gate instead; that's acceptable.)
- **Don't use DuckDB-specific syntax that won't work in plain SQL** where a
  portable form exists. This DB might one day move to Postgres or SQLite for
  some features.

## When the change touches Sling integration

If a column maps to a Sling API field (e.g., `sling_user_id`,
`sling_position_id`), update `docs/sling-api.md` too. Cross-reference the
migration in the doc.

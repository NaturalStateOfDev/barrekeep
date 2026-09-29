#!/usr/bin/env node
// Set (or check) the app version everywhere it is recorded.
//
//   npm run bump -- 0.3.0          # rewrite all five locations
//   node scripts/bump-version.mjs --check 0.3.0
//                                  # exit 1 unless every location says 0.3.0
//                                  # (release.yml runs this against the tag)
//
// Locations:
//   package.json                   "version"
//   package-lock.json              top-level "version" and packages[""].version
//   src-tauri/tauri.conf.json      "version"
//   src-tauri/Cargo.toml           [package] version
//   src-tauri/Cargo.lock           the `barrekeep` [[package]] entry
//
// Edits are targeted text substitutions, not parse+reserialize, so file
// formatting is left exactly as it was. No dependencies beyond Node itself.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

// Each target: file, human label, a regex whose group 1 is the text before
// the version and group 2 is the version itself. Every regex must match
// exactly once.
const TARGETS = [
  {
    file: "package.json",
    label: 'package.json "version"',
    re: /^(\{\s*\n(?:.*\n)*?\s{2}"version":\s*")([^"]+)(")/,
  },
  {
    file: "package-lock.json",
    label: 'package-lock.json top-level "version"',
    re: /^(\{\s*\n\s{2}"name":\s*"barrekeep",\s*\n\s{2}"version":\s*")([^"]+)(")/,
  },
  {
    file: "package-lock.json",
    label: 'package-lock.json packages[""].version',
    re: /(\n\s{4}"":\s*\{\s*\n\s{6}"name":\s*"barrekeep",\s*\n\s{6}"version":\s*")([^"]+)(")/,
  },
  {
    file: "src-tauri/tauri.conf.json",
    label: 'tauri.conf.json "version"',
    re: /^(\{\s*\n(?:.*\n)*?\s{2}"version":\s*")([^"]+)(")/,
  },
  {
    file: "src-tauri/Cargo.toml",
    label: "Cargo.toml [package] version",
    re: /(^\[package\]\s*\n(?:(?!\[).*\n)*?version\s*=\s*")([^"]+)(")/m,
  },
  {
    file: "src-tauri/Cargo.lock",
    label: "Cargo.lock barrekeep entry",
    re: /(\[\[package\]\]\nname = "barrekeep"\nversion = ")([^"]+)(")/,
  },
];

function usage(msg) {
  if (msg) console.error(`error: ${msg}`);
  console.error("usage: npm run bump -- <X.Y.Z>");
  console.error("       node scripts/bump-version.mjs --check <X.Y.Z>");
  process.exit(2);
}

const args = process.argv.slice(2);
const check = args[0] === "--check";
if (check) args.shift();
if (args.length !== 1) usage();
const version = args[0].replace(/^v/, "");
if (!SEMVER.test(version)) usage(`"${args[0]}" is not a semver version (X.Y.Z)`);

const files = new Map();
const read = (f) => {
  if (!files.has(f)) files.set(f, readFileSync(join(ROOT, f), "utf8"));
  return files.get(f);
};

let mismatches = 0;
for (const t of TARGETS) {
  const text = read(t.file);
  const global = new RegExp(t.re.source, t.re.flags.includes("g") ? t.re.flags : t.re.flags + "g");
  const hits = [...text.matchAll(global)];
  if (hits.length !== 1) {
    console.error(`error: expected exactly one match for ${t.label}, found ${hits.length}`);
    process.exit(1);
  }
  const current = hits[0][2];
  if (check) {
    const ok = current === version;
    if (!ok) mismatches++;
    console.log(`${ok ? "ok      " : "MISMATCH"} ${t.label}: ${current}`);
  } else {
    files.set(t.file, text.replace(t.re, `$1${version}$3`));
    console.log(`${t.label}: ${current} -> ${version}`);
  }
}

if (check) {
  if (mismatches) {
    console.error(
      `\n${mismatches} location(s) do not match ${version}. ` +
        `Run \`npm run bump -- ${version}\` and commit before tagging.`,
    );
    process.exit(1);
  }
  console.log(`\nAll version locations match ${version}.`);
} else {
  for (const [f, text] of files) writeFileSync(join(ROOT, f), text);
  console.log(`\nNext: git commit -am "chore: bump version to ${version}" && git tag v${version}`);
}

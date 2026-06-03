# Setup

Two scenarios: **dev machine** (you, on macOS/Linux/Windows) and **release pipeline** (the GitHub Actions workflow that builds the installer for Teacher A's Windows laptop). Teacher A herself only needs the MSI from the latest GitHub Release — no setup required after first install; the app self-updates.

## Dev machine

### 1. Toolchain via mise

We pin Rust/Node/Python in [`mise.toml`](./mise.toml) so versions match across machines.

Install mise once (https://mise.jdx.dev/getting-started.html), then in the repo root:

```sh
mise install
mise exec -- rustc --version   # 1.83
mise exec -- node --version    # v22.x
mise exec -- python --version  # 3.12
```

With `mise activate` in your shell profile, you can drop the `mise exec --` prefix.

### 2. System libraries (Linux only)

Tauri's webview is GTK-backed on Linux. Install once:

```sh
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libssl-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libsoup-3.0-dev \
  build-essential pkg-config curl wget file
```

macOS users get this from Xcode Command Line Tools. Windows users need the Microsoft C++ Build Tools (see README).

### 3. App dependencies

```sh
npm ci
```

### 4. Run

```sh
npm run tauri dev
```

First Rust build compiles DuckDB from C++ source: 5–10 minutes. Subsequent builds are fast.

> **Note on `bundle.targets: "msi"`:** This is fine for `tauri dev` (no bundling happens). `tauri build` will only succeed on Windows; on Linux it errors when it can't produce an MSI. For local Linux smoke-testing of a build, override with `npm run tauri build -- --bundles deb,appimage`.

## Release pipeline

Releases are signed MSI installers published to GitHub Releases. The app's updater plugin checks the latest release on launch and offers an update if newer.

### One-time: generate the updater signing key

On any machine with the Tauri CLI available (run `mise install` first):

```sh
mkdir -p ~/.tauri
npx tauri signer generate -w ~/.tauri/barrekeep.key
```

It prints a public key and writes the private key to `~/.tauri/barrekeep.key`. It will prompt for a password.

Then:

1. **Public key:** copy it into `src-tauri/tauri.conf.json` at `plugins.updater.pubkey`, replacing the `REPLACE_WITH_PUBLIC_KEY_FROM_TAURI_SIGNER_GENERATE` placeholder. Commit this change.
2. **Private key + password:** set as GitHub Actions secrets on the repo:
   - `TAURI_SIGNING_PRIVATE_KEY` — paste the *contents* of `~/.tauri/barrekeep.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — the password you set

```sh
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/barrekeep.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

**Back up `~/.tauri/barrekeep.key` somewhere safe.** If you lose it, you can't sign updates — installed apps will reject new releases and need a manual reinstall.

### Cutting a release

```sh
# Bump versions in package.json and src-tauri/tauri.conf.json (keep them aligned)
git commit -am "Release v0.1.0"
git tag v0.1.0
git push origin main --tags
```

The `release` workflow (.github/workflows/release.yml) picks up the tag, builds on a Windows runner, signs the MSI, and creates a public GitHub Release with:

- `the barre studio Scheduler_0.1.0_x64_en-US.msi` — the installer
- `the barre studio Scheduler_0.1.0_x64_en-US.msi.sig` — signature
- `latest.json` — updater manifest the app reads

Teacher A's installed app will see the new release on next launch and offer to install it.

## Updater flow (how it works end-to-end)

1. App launches → `checkForUpdatesOnStartup()` runs (see `src/lib/updater.ts`)
2. It calls `check()` from `@tauri-apps/plugin-updater`
3. The plugin fetches `https://github.com/NaturalStateOfDev/barrekeep/releases/latest/download/latest.json`
4. If `latest.json` version > current, it returns an `Update` object
5. We `window.confirm` the user, then `update.downloadAndInstall()` runs the new MSI
6. `relaunch()` restarts into the new version

If the signature in `latest.json` doesn't verify against the public key baked into the app, the install is refused — that's the security guarantee.

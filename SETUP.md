# Setup

Two scenarios: **dev machine** (you, on macOS/Linux/Windows) and **release pipeline** (the GitHub Actions workflow that builds the installer for Teacher A's Windows laptop). Teacher A herself only needs the MSI from the latest GitHub Release plus Python (see [Studio PC requirements](#studio-pc-requirements)); after first install the app self-updates.

## Dev machine

### 1. Toolchain via mise

We pin Rust/Node/Python in [`mise.toml`](./mise.toml) so versions match across machines.

Install mise once (https://mise.jdx.dev/getting-started.html), then in the repo root:

```sh
mise install
mise exec -- rustc --version   # 1.88.0 (rust-toolchain.toml pins the exact version)
mise exec -- node --version    # v22.x
mise exec -- python --version  # 3.12
```

With `mise activate` in your shell profile, you can drop the `mise exec --` prefix.
Without mise, plain rustup works too: `rust-toolchain.toml` pins Rust 1.88.0
(+ clippy, rustfmt) and rustup installs it on first `cargo` use.

### 2. System libraries (Linux only)

Tauri's webview is GTK-backed on Linux. The repo's `.cargo/config.toml` also
links with [mold](https://github.com/rui314/mold) on x86_64 Linux (GNU ld
needs 2–3 GB RAM per link of this app), so mold is required too.

Debian / Ubuntu:

```sh
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libsoup-3.0-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libxdo-dev \
  libssl-dev \
  build-essential pkg-config curl wget file \
  mold
```

Arch:

```sh
sudo pacman -S --needed \
  webkit2gtk-4.1 gtk3 libsoup3 \
  libappindicator-gtk3 librsvg xdotool \
  openssl base-devel pkgconf curl wget file \
  mold
```

No mold (or a distro without it)? Override the linker for a shell session —
`RUSTFLAGS` replaces the `rustflags` from `.cargo/config.toml`:

```sh
export RUSTFLAGS="-C link-arg=-fuse-ld=bfd"
```

macOS users get the system libraries from Xcode Command Line Tools. Windows users need the Microsoft C++ Build Tools and the WebView2 runtime (see README).

### 3. App dependencies

```sh
npm ci
```

### 4. Run

```sh
npm run tauri dev
```

First Rust build compiles DuckDB from C++ source: 5–10 minutes. Subsequent builds are fast.

### 5. Checks (what CI runs)

```sh
npm run build && npm test
python3 scripts/tests/test_propose_rules.py
cargo test --release --manifest-path src-tauri/Cargo.toml
cargo clippy --release --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
```

Clippy is blocking in CI.

> **Note on `bundle.targets: "msi"`:** This is fine for `tauri dev` (no bundling happens). `tauri build` will only succeed on Windows; on Linux it errors when it can't produce an MSI. For local Linux smoke-testing of a build, override with `npm run tauri build -- --bundles deb,appimage`.

## Studio PC requirements

The Windows laptop that runs the installed app needs:

- **The MSI** from the latest GitHub Release (WebView2 ships with Windows 10/11).
- **Python 3.11 or newer** — the schedule algorithm (`propose.py`) runs as a
  Python subprocess. Install from the [python.org](https://www.python.org/downloads/windows/)
  installer and tick **"Add python.exe to PATH"**. Then turn off the Microsoft
  Store stubs: *Settings → Apps → Advanced app settings → App execution
  aliases* → switch off `python.exe` and `python3.exe` (otherwise `python`
  opens the Store instead of running Python). No extra packages are needed.

Open the app's **Settings → Python** row to verify: it shows the Python it
found and its version, or what's wrong (not found, Store placeholder, too old).

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

1. Bump the version everywhere it lives (package.json, package-lock.json,
   tauri.conf.json, Cargo.toml, Cargo.lock) on a branch and open a PR:

   ```sh
   git checkout -b chore/bump-0.3.0 origin/main
   npm run bump -- 0.3.0
   git commit -am "chore: bump version to 0.3.0"
   git push -u origin HEAD && gh pr create --base main --fill
   ```

2. Merge the PR once its checks pass.
3. **Wait for the `ci` run on main to go green** for the merge commit. That
   run also saves the Rust build cache the release restores.
4. Tag that exact commit and push the tag:

   ```sh
   git pull && git tag v0.3.0 && git push origin v0.3.0
   ```

The `release` workflow (.github/workflows/release.yml) then:

- refuses to run unless `ci` succeeded on main for the tagged commit (it waits
  if that run is still in progress);
- checks the tag matches all five version files
  (`node scripts/bump-version.mjs --check v0.3.0`);
- builds and signs the MSI into a **draft** release;
- installs the MSI silently and smoke-tests the installed `propose.py`
  (`scripts/tests/smoke_installed.py`);
- only then publishes the release as latest.

If a step after the build fails, the draft release is left for inspection —
delete it (`gh release delete v0.3.0 --yes`) before re-running the workflow.
The published release contains:

- `Barrekeep_0.3.0_x64_en-US.msi` — the installer (`Barrekeep_<version>_x64_en-US.msi`)
- `Barrekeep_0.3.0_x64_en-US.msi.sig` — signature
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

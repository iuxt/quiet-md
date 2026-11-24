# Contributing

Code style and commit conventions are in [AGENTS.md](AGENTS.md) (the same as CLAUDE.md). Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before you start.

## Building from source

macOS needs the Xcode Command Line Tools; Windows needs the Visual Studio 2022 C++ Build Tools and the Windows SDK. The Rust version is pinned in `rust-toolchain.toml`, and rustup picks it up automatically.

```sh
cargo build --release --locked
./target/release/quiet_md                          # uses ~/Documents/Quiet.md
QUIET_MD_HOME=/tmp/lib ./target/release/quiet_md   # uses another library
```

While developing, point `QUIET_MD_HOME` at a scratch directory; don't test against your real library.

The repository is a Cargo workspace: `crates/core` is the document engine (`fdl_core`), which does not depend on GPUI, and the root crate is the app. GPUI, taffy and xattr are locally patched copies in `vendor/`; the changes are listed in [vendor/PATCHES.md](vendor/PATCHES.md). `target/debug` can grow past 10 GB; if you only build release, `cargo clean --profile dev` clears it.

## Testing

```sh
scripts/check.sh                                        # tests + clippy with zero warnings; run before committing
cargo test --release -p fdl_core                        # the document engine only, without building GPUI
FDL_FUZZ_SEEDS=2000 cargo test --release random_edits   # more random edits, for the editing core
```

- The editing core has a random test: starting from a document with every block type, it makes random selections, typing, structural, style, table and undo edits, checks the model's invariants after each step, and saves and reloads every 25 steps.
- The document format has a regression test: `crates/core/tests/fixtures/all-blocks.xml` must read and write back byte for byte.
- UI screenshots and scripted runs are covered in [tools/harness/README.md](tools/harness/README.md).

To run the checks on every commit: `git config core.hooksPath scripts/hooks`.

## Documentation

| Document | Contents |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Layers, the editing flow, invariants, undo, rendering, the module map |
| [docs/FORMAT.md](docs/FORMAT.md) | Library layout, index recovery, DocxXML read/write rules and local extensions |
| [docs/THEMES.md](docs/THEMES.md) | Writing document themes in CSS: selectors, properties, named colors |
| [AGENTS.md](AGENTS.md) | Code style, pre-commit checks, testing conventions |

Most of these, AGENTS.md included, are written in Chinese for now.

## Building release packages

Before a release, `scripts/check.sh` must pass. Branch pushes and pull requests do not start builds; pushing a version tag automatically runs the Release workflow, which checks every platform before publishing. The CI workflow remains available for manual validation builds. The default Release workflow publishes free test prereleases: ad-hoc signed, unnotarized macOS ZIPs, an unsigned Windows installer and a Linux AppImage, each with SHA-256. No platform signing credentials are required. Download and install these builds manually. See [the release guide](docs/RELEASES.md). The Windows CI build does not replace installing, uninstalling and editing on a real machine.

Packaging scripts build with `cargo build --profile dist` (Cargo.toml: fat LTO, no symbol table) into `target/dist`, about a third smaller than `--release`, which stays the quick everyday build. A full `dist` build takes about four minutes, and so does relinking after a change. Line tables go to a separate file that is kept but never published: `release.sh` writes `dSYM-quiet-md-<version>-macos-<arch>.zip`, `package-windows.ps1` writes `quiet-md-<version>-windows-x64.pdb`. A panic in the log carries `base … frames: …`; on macOS, resolve them with `atos -o <unzipped dSYM> -arch <arch> -l <base> <frames>`.

The declared macOS minimum is 13, not yet verified on real hardware. Intel packages can be cross-compiled on Apple Silicon; real Intel hardware validation is still needed.

### macOS

For a local build installed in `/Applications/quiet.md.app`, run `./build.sh`. It packages the native architecture, requests a normal quit of the installed app, waits up to 60 seconds for it to exit, replaces it and launches the new version. A cancelled or timed-out quit stops installation. Writing to `/Applications` requires permission for that directory.

Local builds default to `SIGNING_IDENTITY=local`: the first build creates a self-signed certificate and private key in a dedicated keychain under `~/Library/Application Support/quiet-md/local-signing`, then reuses them on subsequent builds. The directory and its keychain password are private to your user; no system trust settings are changed. Keep this directory to preserve the app's identity across rebuilds. macOS may ask for Documents access once when switching from an older ad-hoc build, but rebuilding with the same certificate keeps the same designated requirement. Ad-hoc signatures bind permissions to one binary hash, which changes when the app is rebuilt; see [Apple's code requirements note](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements). `QUIET_MD_SIGNING_DIR` can select a separate signing directory for tests. Local packages are not notarized and retain the `-unsigned.zip` suffix; distribute them only as explicitly labeled test prereleases. Set `SIGNING_IDENTITY` and `NOTARY_PROFILE` to use the signed release flow below, or explicitly use `SIGNING_IDENTITY=-` for ad-hoc CI validation.

`scripts/release.sh` builds for the local architecture, generates the app icon, signs with a Developer ID Application certificate, submits for Apple notarization, staples the ticket and runs a Gatekeeper check, then writes a ZIP and its SHA-256. It does not upload to GitHub or launch the app.

Install the signing certificate, with its private key, in the login keychain and set up a notarytool keychain profile, then run:

```sh
SIGNING_IDENTITY="Developer ID Application: …" NOTARY_PROFILE="quiet-md" scripts/release.sh
```

The script stops if signing or notarization fails; whatever is left in the failed run's directory must not be shipped.

To build both architectures sequentially on one Apple Silicon Mac, install the Intel Rust target once, then run the same signing and notarization flow for each architecture:

```sh
rustup target add x86_64-apple-darwin
export SIGNING_IDENTITY="Developer ID Application: …" NOTARY_PROFILE="quiet-md"
scripts/release.sh
MACOS_TARGET=x86_64-apple-darwin scripts/release.sh
```

Each run reuses `dist-release/` (override with `RELEASE_OUT`), rebuilding the app bundle and icon set. Packages and debug symbols retain their version and architecture in the filename; rebuilding the same version and architecture overwrites those files. Run builds sequentially when sharing an output directory. Intel ZIPs use `macos-x86_64`; Apple Silicon ZIPs keep `macos-arm64`. CI builds both on its existing ARM runner and verifies the binary architectures and signatures. `SIGNING_IDENTITY=-` skips notarization and creates ad-hoc signed `-unsigned.zip` validation packages, which must not be distributed and are rejected by `publish-downloads.sh`.

### Windows

Uses Inno Setup 7. In a Visual Studio x64 Developer PowerShell:

```powershell
# An unsigned installer for local testing; nothing is uploaded or published
.\scripts\package-windows.ps1 -Iscc 'C:\path\to\ISCC.exe'

# Once the certificate is issued and connected through a hardware token or a cloud signing client,
# pick the current user's certificate by thumbprint
.\scripts\package-windows.ps1 -Iscc 'C:\path\to\ISCC.exe' `
  -CertificateThumbprint '<40-character certificate thumbprint>' `
  -TimestampUrl 'http://time.certum.pl'
```

Signing uses `signtool.exe` from the Windows SDK; `-SignTool` sets its path. The timestamp URL should be the certificate authority's RFC 3161 service (Certum in the example). The script signs the app, the installer and the uninstaller separately, checks the publisher, the trusted signature and the timestamp, and stops on any failure. The final installer and its SHA-256 go to `dist/windows/signed/` or `dist/windows/unsigned/`; the intermediate directories are not for release.

The installer lets you choose the location and an optional desktop shortcut, and by default installs for the current user only. Uninstalling keeps your documents. On Windows, documents live in `Quiet.md` inside the system Documents folder.

On every platform, never commit credentials, private keys or certificate files.

### Releases

All platforms use manual downloads from [GitHub Releases](https://github.com/iuxt/quiet-md/releases). The app does not check for or install releases. Version notes live in `docs/releases/<version>.md`.

A push to `main` that changes `VERSION` runs `.github/workflows/release.yml`. Actions builds all four packages, tags the build commit and publishes a verified stable release with SHA-256 checksums. For local preparation, `scripts/publish-github-release.py --dry-run` inspects the four packages without writes or network access. `--prepare <new-directory>` creates the eight package and checksum assets. `--publish <new-directory>` checks the release checkout and pushed tag, runs the checks, uploads and verifies a draft, then publishes and checks public downloads. Add `--draft` to leave it unpublished, or `--notes <release-notes-file>` to select notes. Existing releases and assets are never overwritten. See [the release guide](docs/RELEASES.md).

The release version is recorded in `VERSION`. Use the script below to read it or bump it together with both `Cargo.toml` files and the workspace packages in `Cargo.lock` (Python 3 required; works from any directory):

```sh
python3 scripts/version.py               # print the current version (same as get)
python3 scripts/version.py get
python3 scripts/version.py bump-major    # 0.2.0 -> 1.0.0
python3 scripts/version.py bump-minor    # 0.2.0 -> 0.3.0
python3 scripts/version.py bump-patch    # 0.2.0 -> 0.2.1
```

Each bump command prints the new version. Major and minor bumps reset the lower components to zero. All commands reject invalid or inconsistent versions before changing files; `scripts/check.sh` also checks consistency. Bump commands do not create commits or tags. After committing and pushing a release, run `python3 scripts/version.py release` to create and push its version tag.

### Linux (experimental)

Install Rust through rustup, then run `scripts/linux-deps.sh` on Ubuntu/Debian or Arch-based systems. Run `scripts/preview-linux.sh` for an isolated temporary library, or set `QUIET_MD_HOME` to reuse a test library. `scripts/package-linux.sh` produces a tarball and SHA-256 in `dist/linux`; it includes a desktop entry and a 512×512 icon generated with ImageMagick. The binary is dynamically linked; build on the oldest distribution you intend to support and check `runtime-libraries.txt` on the target system.

The Linux build and full checks run on Ubuntu 22.04. `scripts/package-appimage.sh` also produces an x86_64 AppImage in `dist/linux`, using a pinned, SHA-256-verified linuxdeploy. It bundles shared dependencies and checks the payload against a glibc 2.35 baseline; graphics drivers remain system-provided. SHA-256 and dependency reports are generated alongside it. AppImage can be run after `chmod +x`; if FUSE is unavailable, use `--appimage-extract-and-run`. Install new versions manually for all package formats. Interactive testing has been done on Omarchy 4.5 with Wayland, floating windows and 250% scaling, including Chinese title input. GNOME Wayland, X11 and systems without Noto fonts still need interactive verification; this is not a claim of support for every desktop distribution. Software Vulkan (`vulkan-swrast` on Arch) is useful in VMs or when no hardware Vulkan driver is available. Global quick-note shortcuts are not supported on Linux yet.

Linux support started in [PR #7](https://github.com/s1dashu/quiet-md/pull/7) by [VJ-2303](https://github.com/VJ-2303); maintainer follow-ups add desktop integration and runtime fixes.

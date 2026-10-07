# Releasing

## Cutting a release

1. Bump `version` in `Cargo.toml`, run `cargo build` (updates `Cargo.lock`), commit.
2. Tag and push:
   ```sh
   git tag v0.2.0
   git push origin main v0.2.0
   ```
3. The **Release** workflow checks the tag matches `Cargo.toml`, then builds and uploads to a
   GitHub release:

   | Asset | For |
   |---|---|
   | `fss-mqtt-<v>-x86_64-unknown-linux-musl.tar.gz` | Linux x86_64 (static, any distro) |
   | `fss-mqtt-<v>-aarch64-unknown-linux-musl.tar.gz` | Linux arm64 (static) |
   | `fss-mqtt_<v>-1_amd64.deb`, `fss-mqtt_<v>-1_arm64.deb` | Debian/Ubuntu: `sudo apt install ./fss-mqtt_*.deb` |
   | `fss-mqtt-<v>-1.x86_64.rpm`, `fss-mqtt-<v>-1.aarch64.rpm` | Fedora/RHEL/openSUSE: `sudo dnf install ./fss-mqtt-*.rpm` |
   | `fss-mqtt-<v>-{x86_64,aarch64}-apple-darwin.tar.gz` | macOS |
   | `fss-mqtt-<v>-x86_64-pc-windows-msvc.zip` | Windows x64 (also runs on Windows on Arm) |
   | `SHA256SUMS` | checksums for all of the above |

4. If configured (below), it then updates the Homebrew tap and Scoop bucket and opens a winget PR.

The `aarch64-unknown-linux-musl` build uses GitHub's `ubuntu-24.04-arm` runner, which is free for
public repositories. For a private repository, remove that matrix entry or use a paid plan.

## One-time setup for package managers

Each channel is off until its repository **variable** is set
(Settings → Secrets and variables → Actions → Variables). Tokens go under **Secrets**.
Use fine-grained personal access tokens limited to the one repository they push to,
with *Contents: read and write*.

### Homebrew (macOS and Linux)

1. Create a public repo named `homebrew-tap` (e.g. `<owner>/homebrew-tap`), with a README.
2. Variable `HOMEBREW_TAP` = `<owner>/homebrew-tap`; secret `HOMEBREW_TAP_TOKEN` = token for it.
3. Users install with:
   ```sh
   brew install <owner>/tap/fss-mqtt
   ```

### Scoop (Windows)

1. Create a public repo, e.g. `<owner>/scoop-bucket`.
2. Variable `SCOOP_BUCKET` = `<owner>/scoop-bucket`; secret `SCOOP_BUCKET_TOKEN` = token for it.
3. Users install with:
   ```powershell
   scoop bucket add fss-mqtt https://github.com/<owner>/scoop-bucket
   scoop install fss-mqtt
   ```

### winget (Windows)

winget packages live in [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) and every
version is a pull request that Microsoft reviews (usually within a day or two).

1. Fork `microsoft/winget-pkgs` to the account that owns the token.
2. Submit the **first** version by hand, after the first GitHub release exists. The easiest way is
   [Komac](https://github.com/russellbanks/Komac):
   ```sh
   komac new <Publisher>.FssMqtt --version 0.1.0 \
     --urls https://github.com/<owner>/<repo>/releases/download/v0.1.0/fss-mqtt-0.1.0-x86_64-pc-windows-msvc.zip
   ```
   Choose installer type `zip` with nested type `portable` and command alias `fss-mqtt`.
3. Once that PR is merged: variable `WINGET_ID` = `<Publisher>.FssMqtt`; secret `WINGET_TOKEN` =
   a **classic** token with `public_repo` scope (needed to open PRs against winget-pkgs).
4. Later releases are submitted automatically. Users install with:
   ```powershell
   winget install <Publisher>.FssMqtt
   ```

## Not covered (yet)

- An apt/dnf **repository** (`apt install fss-mqtt` without downloading a file first). That needs
  hosting and signing, e.g. via packagecloud or Cloudsmith, or a distribution's official archive.
- AUR (Arch), Snap and Flatpak, Chocolatey, and crates.io.
- Code signing for Windows and macOS. Unsigned binaries work, but Windows SmartScreen may warn on
  first run of a downloaded zip; Homebrew, Scoop and winget installs are unaffected.

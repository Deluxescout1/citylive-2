# Cutting a release

Releases are tagged. Tagging `vX.Y.Z` makes GitHub Actions build everything and publish the release; the apps
find it through the GitHub Releases API and update themselves. Only big updates and fixes are released, not every commit.

1. **Version.** Edit `version` under `[workspace.package]` in `Cargo.toml` (one version for the whole product), and
   commit (the lock file follows: `cargo update -w` if cargo says so).
2. **Changelog.** Add a section `## [X.Y.Z] - YYYY-MM-DD` at the top of `CHANGELOG.md`. Its text becomes the release notes.
3. **Test.** `cargo test --release --workspace --exclude present_gnome -- --test-threads=2` (CI runs the same on push).
4. **Tag and push.** `git tag vX.Y.Z && git push origin main vX.Y.Z`.
5. The `release` workflow checks that the tag, `Cargo.toml` and `CHANGELOG.md` agree, builds
   - `Citylive2-Setup-X.Y.Z.exe` (NSIS installer) and `Citylive2-X.Y.Z-windows-x64.zip` on `windows-latest`,
   - `citylive-2-X.Y.Z-linux-x86_64.tar.gz` and `citylive-2_X.Y.Z_amd64.deb` on `ubuntu-latest`,
   - writes `SHA256SUMS` for all four and creates the GitHub release with the notes. A tag with a hyphen
     (`v0.3.0-rc.1`) is published as a pre-release; the updater ignores pre-releases.
6. Check the release page, then install it on a machine (the Windows installer shows the SmartScreen prompt until
   the project is code-signed, see `docs/windows-signing.md`).

The updater downloads exactly the file names above and refuses a download whose SHA-256 is not in `SHA256SUMS`.
Do not rename release files.

## Building the files locally

- Windows (cross-compiled from Linux): `bin/build-installer.sh` builds the MSVC programs with `cargo xwin` and
  packs `dist/Citylive2-Setup-X.Y.Z.exe` and the portable zip (needs `makensis`).
- Linux: `bin/package-linux.sh` writes `dist/citylive-2-X.Y.Z-linux-x86_64.tar.gz` and the `.deb`.

## The public repository

`bin/make-public-tree.sh <empty-dir> --verify` exports a clean tree (no planning notes, handbacks, reference images,
saves or internal scripts), prunes the workspace, scans for secrets and personal data, and checks the exported
tree compiles. Zero findings are required before anything is pushed. History starts fresh: commit the export as the
first commit with the maintainer's own author identity.

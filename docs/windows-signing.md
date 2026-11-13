# Windows: Defender false positives and free code signing

Unsigned, new, rarely downloaded programs that draw into the desktop (a wallpaper must parent a window into
Explorer's desktop layer) are what Defender's machine-learning heuristics (`...!ml`) dislike. The first VM test flagged
`citylive.exe` as `Trojan:Win32/Bearfoos.A!ml`. Free mitigations, in order of effect:

1. Sign the release (SignPath Foundation, below). Reputation follows the signing identity.
2. Submit each release to Microsoft as a false positive (below). Detections are usually cleared within days.
3. What the build already does: MSVC ABI with the static CRT (`bin/build-windows.sh`, the default; the mingw build is
   `bin/build-windows.sh gnu`), a full VERSIONINFO (Product "Citylive 2", Company "Deluxescout1"), a manifest
   (asInvoker, per-monitor-v2 DPI, Win10/11), the icon, no packing or compression, no self-copying, no console tricks,
   and nothing written to the Run key unless the user turns "start with Windows" on.

## 1. SignPath Foundation (free for open source)

Do this once the GitHub repository is public.

Requirements (from signpath.org/foundation; check the current page):
- An OSI-approved licence (MIT is fine) and a public repository with no proprietary parts.
- The project is maintained and released, and the README has a **Code signing policy** (who may approve a signing
  request) and a privacy statement (the program sends nothing except weather and location lookups to the services
  named in the README).
- Builds are made from the public source by a CI system SignPath can verify (GitHub Actions).

Steps:
1. Apply at https://signpath.org/foundation (the "Apply" form): project name Citylive 2, repo URL, licence, a short
   description, and the contact. Approval takes days to a few weeks. The certificate is issued to "SignPath Foundation"
   (the publisher name shown in the SmartScreen prompt).
2. In SignPath: create the project, an artifact configuration for the exe files (or a zip), and a signing policy
   `release-signing` that needs the maintainer's approval.
3. Install the SignPath GitHub App on the repo; add the secret `SIGNPATH_API_TOKEN` and the repository variable
   `SIGNPATH_ORGANIZATION_ID`.
4. In GitHub Actions build on a `windows-latest` runner (`cargo build --release --target x86_64-pc-windows-msvc`),
   upload the exe files with `actions/upload-artifact`, then:
   ```yaml
   - uses: signpath/github-action-submit-signing-request@v1
     with:
       api-token: ${{ secrets.SIGNPATH_API_TOKEN }}
       organization-id: ${{ vars.SIGNPATH_ORGANIZATION_ID }}
       project-slug: citylive2
       signing-policy-slug: release-signing
       github-artifact-id: ${{ steps.upload.outputs.artifact-id }}
       wait-for-completion: true
       output-artifact-directory: signed
   ```
   Attach the files in `signed/` to the GitHub Release. (The Linux cross-build in `bin/build-windows.sh` is for
   testing; SignPath needs the release built on GitHub's runner so its origin can be verified.)
5. The NSIS installer must be signed too, in the same request.

SmartScreen still warns for a while on a new publisher; the README should say "More info > Run anyway".

## 2. Report a false positive to Microsoft (what the maintainer does)

For every release, or whenever Defender flags a build:
1. Go to https://www.microsoft.com/wdsi/filesubmission and sign in with a Microsoft account.
2. Submit as **Software developer**. Upload the exact file that was flagged (zip it with the password `infected` if the
   browser blocks it, and say so).
3. Choose "Incorrectly detected as malware/malicious" and enter the detection name Defender showed (for example
   `Trojan:Win32/Bearfoos.A!ml`).
4. In the additional information say: open-source wallpaper program, repository URL, release tag, SHA-256 of the file,
   what it does (draws a city into the desktop wallpaper layer, per user, no admin, no persistence unless the user
   enables start-with-Windows), and that the source is public.
5. Wait for the e-mail (usually 1 to 5 days). Submit each new build; a signed build clears faster.

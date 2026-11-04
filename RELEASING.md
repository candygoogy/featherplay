# Releasing featherplay

The steps for a release, `X.Y.Z` standing for the new version. Pushing a `vX.Y.Z` tag starts
`.github/workflows/release.yml`, which builds the files into a draft release. Nothing is
public until you publish that draft.

## 1. Bump the version

On `main`, with everything for the release merged:

1. Set `version` under `[workspace.package]` in the root `Cargo.toml`. Every crate inherits
   it. Run `cargo build -p featherplay` once so `Cargo.lock` follows, and commit both:
   `release.yml` builds with `--locked` and fails on a stale lock file.
2. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`, put a fresh,
   empty `## [Unreleased]` above it, and update the links at the bottom:

   ```
   [Unreleased]: https://github.com/candygoogy/featherplay/compare/vX.Y.Z...HEAD
   [X.Y.Z]: https://github.com/candygoogy/featherplay/compare/vPREVIOUS...vX.Y.Z
   ```

The README and the website change later (step 4): they link to the release's files and show
their sizes, which only exist once the release has built them.

## 2. Run the checks

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTFLAGS="--cfg ci" cargo nextest run --workspace
RUSTFLAGS="--cfg ci" cargo test --workspace --doc
```

Commit as `chore: release X.Y.Z`, push `main`, and wait for CI to pass on that commit.

## 3. Tag

```
git tag -a vX.Y.Z -m "featherplay X.Y.Z"
git push origin vX.Y.Z
```

`release.yml` runs the tests on x64 and on ARM64 (a `windows-11-arm` runner), checks that
the tag matches the crate version, builds `featherplay.exe` for each with the build keys
from the repository secrets, fails if either exe is over 31 MB, builds both installers with
Inno Setup, and creates a **draft** release with:

- `featherplay.exe` and `featherplay.exe.sha256` (x64)
- `featherplay-setup-X.Y.Z.exe` and `featherplay-setup-X.Y.Z.exe.sha256` (x64)
- `featherplay-arm64.exe` and `featherplay-arm64.exe.sha256`
- `featherplay-setup-X.Y.Z-arm64.exe` and `featherplay-setup-X.Y.Z-arm64.exe.sha256`
- `SHA256SUMS.txt`, covering all four and `THIRD_PARTY.md`
- `THIRD_PARTY.md`

The x64 names never change: 0.1.0's links, the site, the README, winget and Scoop use them.

To try the workflow before tagging, run it on a branch:
`gh workflow run release.yml --ref <branch>` (or Actions > Release > Run workflow). It does
everything up to the draft and uploads the files as the run's `release-assets` artifact
instead; it never creates a release.

If the run warns that it built without some of the secrets (the draft's notes say so too),
don't publish that build. Delete the draft, add the secrets under Settings > Secrets and
variables > Actions, and re-run the workflow.

## 4. Check the draft

Download the draft's files into an empty folder and check them:

```
gh release download vX.Y.Z --dir <empty folder>
cd <empty folder>
sha256sum -c SHA256SUMS.txt
```

Then install and uninstall, on a clean VM if you have one. featherplay's data on your own
PC makes the last check fail.

1. Run `featherplay-setup-X.Y.Z.exe`. featherplay should start from the Start menu, sign in
   and play.
2. Uninstall it from Settings > Apps.
3. Run `powershell -File packaging/windows/check-uninstalled.ps1`. It lists everything
   featherplay keeps and exits with 0 only when all of it is gone.

Run the portable `featherplay.exe` once too. On an ARM64 PC, if you have one, do the same
with `featherplay-setup-X.Y.Z-arm64.exe` and `featherplay-arm64.exe`; Task Manager's
Details tab should show featherplay, yt-dlp and deno as ARM64, not x64. (Over an x64
install, the yt-dlp and deno it downloaded stay, and keep running as x64.)

Then, in one commit that you push after publishing (step 5):

- `README.md`: the version in the installer link
  (`releases/download/vX.Y.Z/featherplay-setup-X.Y.Z.exe`), in the `Get-FileHash` line below
  it, and in the "How light?" table's header; the table's Download row gets both files'
  sizes in MB of 1,048,576 bytes, to two decimals. If you measured again, update the
  table's figures and `site/src/data/numbers.ts` together; they must match.
- `site/src/data/release.ts`: `version`, and each file's size in bytes, from the draft.
- The site tests that pin the version and sizes: `site/tests/unit/data.test.ts`,
  `site/tests/sections.spec.ts`, `site/tests/compare.spec.ts` and `site/tests/seo.spec.ts`.

Check the site:

```
cd site
pnpm build
pnpm unit
pnpm test
pnpm budget
```

## 5. Publish

```
gh release edit vX.Y.Z --draft=false --latest
```

Now push the README and site commit from step 4.

## 6. winget and Scoop

From the published `SHA256SUMS.txt`, take the `featherplay.exe` line. Both manifests install
the portable exe.

winget, the three files in `packaging/winget/`:

- `PackageVersion: X.Y.Z` in all three;
- `ReleaseDate`, `InstallerUrl` (`.../releases/download/vX.Y.Z/featherplay.exe`) and
  `InstallerSha256` in `candygoogy.featherplay.installer.yaml`;
- `ReleaseNotesUrl` in the locale file.

Check them with `winget validate --manifest packaging/winget` and
`winget install --manifest packaging/winget`, then open a pull request to
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) adding them as
`manifests/c/candygoogy/featherplay/X.Y.Z/`.

Scoop, `packaging/scoop/featherplay.json`: set `version`, and the `64bit` `url` and `hash`,
then open a pull request to [ScoopInstaller/Extras](https://github.com/ScoopInstaller/Extras)
adding it as `bucket/featherplay.json`. Once Extras has it, its `checkver` and `autoupdate`
pick up later releases from `featherplay.exe.sha256`.

Commit the manifest changes here as well.

## 7. Redeploy the site

```
cd site
pnpm build
pnpm exec wrangler deploy
```

Then open https://featherplay.candygoogy.dev/: the download buttons should fetch the new
files and show their sizes.

## 8. Afterwards

Install the release on your own PC and play a few songs on a home connection. GitHub's
runners can't check playback, because YouTube asks them to prove they aren't bots.

## When playback breaks

Nothing plays songs on a schedule. Instead, `.github/workflows/ytdlp-watch.yml` reads
yt-dlp's issue tracker every day at 05:30 UTC, because YouTube breakage shows up there
within hours. A report counts when it is:

- open, and created in the last 48 hours;
- about YouTube: labelled `site:youtube`, or with "YouTube" in its title, but not a
  `[youtube:tab]`-style playlist, channel or search page;
- labelled `site-bug` or `high-priority`, and none of `duplicate`, `incomplete`,
  `question`, `cant-reproduce`, `external-issue` and the like;
- busy: labelled `high-priority`, or with 10 or more comments and reactions together.

Reports that count open one issue here, "yt-dlp reports YouTube breakage", labelled
`canary`. The workflow comments on it when the linked reports change, and closes it once
they're all closed. That issue is for you only: the app never reads it.

The app's notice is a step you take by hand:

1. When the canary opens, or users report songs that won't play, play a few songs on a
   home connection. After a failure the app brings yt-dlp up to date by itself first.
2. If playback is broken for everyone, open an issue titled exactly `Playback broken`
   (`ISSUE_TITLE` in `apps/featherplay/src/updates.rs`). Say what fails and that a fix is
   on the way. When a user's songs keep failing after the yt-dlp update, and no newer
   featherplay is out, the "may need an update" toast links to that issue under
   "Details". Only an open issue with exactly that title, opened by the owner, a member
   or a collaborator, counts.
3. When a yt-dlp release fixes it, move the pin in `crates/streams/src/sidecars.rs` so new
   installs start on the fix (the URL and SHA256 together, as the "Sidecar pins" workflow
   says). Installed copies get the fix with their daily `yt-dlp -U`. Then close
   `Playback broken`.

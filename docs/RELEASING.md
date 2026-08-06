# Releasing

How to cut a new version of Seal: where the version number lives, how the release build actually gets triggered, and the one macOS quirk every release currently ships with.

## 1. Bump the version

There's no single source of truth for the version yet, so it has to be bumped in four places by hand — or in one command via `scripts/release.sh` (runs on both macOS and Linux), which does exactly the manual steps below and then `git tag`s the current commit for you, deliberately stopping short of committing or pushing:

```sh
./scripts/release.sh v0.2.0
```

| File | Field | Notes |
|---|---|---|
| `Cargo.toml` (repo root) | `[workspace.package] version` | Drives every library crate under `crates/`, since each of their own `Cargo.toml`s sets `version.workspace = true`. |
| `apps/desktop/src-tauri/Cargo.toml` | `[package] version` | Not workspace-inherited on purpose, since the desktop app's own release version doesn't have to move in lockstep with the library crates. Bump it anyway for a normal release so everything reads the same number. |
| `apps/desktop/src-tauri/tauri.conf.json` | `"version"` | This is the one that actually matters for the release mechanics: `tauri-action` substitutes it into `tagName`/`releaseName` (`v__VERSION__`) in `.github/workflows/release.yml`. |
| `apps/desktop/package.json` | `"version"` | The npm package version. Keep it in sync for consistency; nothing currently reads it at build time. |

After bumping the `Cargo.toml` files, run a build so `Cargo.lock` picks up the new versions instead of leaving it stale:

```sh
cargo check --workspace
```

Commit the version bump on its own, e.g. `chore: Bump version to 0.2.0`.

## 2. Tag and push

The release workflow (`.github/workflows/release.yml`) triggers on any tag matching `v*`:

```sh
git tag v0.2.0
git push origin v0.2.0
```

The tag you push **must match** the version you just set in `tauri.conf.json` (`v` + that version). `tauri-action` builds its own `tagName` from `v__VERSION__`, substituted from `tauri.conf.json`, so a mismatch between the pushed tag and the config version creates a confusing second tag rather than failing loudly.

If you used `scripts/release.sh`, its tag was created *before* the version-bump commit above (it only ever tags whatever commit is currently checked out), so it's still pointing at the old commit at this point — move it before pushing:

```sh
git tag -f v0.2.0
git push origin v0.2.0
```

## 3. What the workflow does

Pushing the tag runs `publish-tauri` across three runners in parallel (macOS, Ubuntu, Windows), each building its own bundle formats (`dmg`+`app`, `appimage`+`deb`, `nsis`+`msi`) and uploading them as assets on a single **draft** GitHub Release for that tag.

Since it's a draft, nothing is public yet: go to the repo's Releases page, review the generated notes and attached artifacts, edit anything that needs it, and publish it manually when it's ready.

## 4. The macOS "is damaged and can't be opened" message

There's no Apple Developer Program membership behind this project (that's a paid, $99/year account), so the macOS build is only ad-hoc signed (`signingIdentity: "-"` in `tauri.conf.json`'s `bundle.macOS`), not signed with a real Developer ID or notarized by Apple. A plain download of an unnotarized app gets quarantined by the browser, and Gatekeeper's response to a quarantined, non-notarized app is the alarming "'Seal' is damaged and can't be opened. You should move it to the Trash" message. It isn't actually damaged, that's just Gatekeeper's blunt way of saying "not notarized."

The release notes for every build already carry the workaround (see `releaseBody` in `release.yml`), so anyone downloading the DMG sees it without having to find this doc. The short version, for reference:

- **System Settings → Privacy & Security**, scroll to the blocked-app notice, click **Open Anyway**, or
- In Terminal, after installing: `xattr -cr /Applications/Seal.app`

This doesn't affect the Linux or Windows builds, only macOS.

### If this project gets a paid Apple Developer account later

Proper code signing + notarization removes the warning entirely. `tauri-action` supports it natively; it just needs these as GitHub Actions secrets, and `release.yml`'s `env:` block updated to pass them through:

- `APPLE_CERTIFICATE`, a Developer ID Application certificate, exported as a base64-encoded `.p12`
- `APPLE_CERTIFICATE_PASSWORD`, the password used when exporting that `.p12`
- `APPLE_SIGNING_IDENTITY`, the certificate's common name, e.g. `Developer ID Application: Your Name (TEAMID)`
- `APPLE_ID` / `APPLE_PASSWORD`, an Apple ID and an app-specific password for it (not the account password), used to submit the build for notarization
- `APPLE_TEAM_ID`, the Apple Developer Team ID

Once those are wired up, `signingIdentity` in `tauri.conf.json` should be changed from `"-"` to the real `APPLE_SIGNING_IDENTITY` value (Tauri accepts the identity from either the config or the env var, not both at once, so pick one and drop the other), and the `releaseBody` note in `release.yml` covering this issue can be deleted.

This file was augmented/rephrased by Claude Codea
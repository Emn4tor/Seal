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

Pushing the tag runs `publish-tauri` across three runners in parallel (macOS, Ubuntu, Windows), each building its own bundle formats (`dmg`+`app`, `appimage`+`deb`, `nsis`+`msi`) and uploading them as assets on a single **draft** GitHub Release for that tag. Once that finishes, `publish-android` and `publish-ios` build the mobile targets and upload to the same release — see §6 for what each actually produces today.

Since it's a draft, nothing is public yet: go to the repo's Releases page, review the generated notes and attached artifacts, edit anything that needs it, and publish it manually when it's ready.

The workflow also runs on-demand from the Actions tab (`workflow_dispatch`), with no tag push required — the easiest way to test a change to this pipeline, especially the mobile jobs, which nothing else exercises.

## 4. The macOS "is damaged and can't be opened" message

There's no Apple Developer Program membership behind this project (that's a paid, $99/year account), so the macOS build is only ad-hoc signed (`signingIdentity: "-"` in `tauri.conf.json`'s `bundle.macOS`), not signed with a real Developer ID or notarized by Apple. A plain download of an unnotarized app gets quarantined by the browser, and Gatekeeper's response to a quarantined, non-notarized app is the alarming "'Seal' is damaged and can't be opened. You should move it to the Trash" message. It isn't actually damaged, that's just Gatekeeper's blunt way of saying "not notarized."

The release notes for every build already carry the workaround (see `releaseBody` in `release.yml`), so anyone downloading the DMG sees it without having to find this doc. The short version, for reference:

- **System Settings → Privacy & Security**, scroll to the blocked-app notice, click **Open Anyway**, or
- In Terminal, after installing: `xattr -cr /Applications/Seal.app`

This doesn't affect the Linux or Windows builds, only macOS.

## 5. Microphone access in the built macOS app

`tauri-bundler` signs the release `.app` with Hardened Runtime enabled even under ad-hoc signing. Hardened Runtime blocks access to protected resources (microphone included) unless the app carries the matching entitlement, regardless of `Info.plist`'s `NSMicrophoneUsageDescription`. `apps/desktop/src-tauri/entitlements.plist` grants `com.apple.security.device.audio-input` and is wired in via `bundle.macOS.entitlements` in `tauri.conf.json`; without it, voice calls silently get no local microphone (`spawn_audio_io_thread` in `crates/core/src/voice.rs` deliberately never fails the call outright on a device error, it just logs a warning and carries on with no local audio, so this fails silently rather than with a visible error). Only affects release builds signed this way; `npm run tauri dev` never goes through this signing path.

### If this project gets a paid Apple Developer account later

Proper code signing + notarization removes the warning entirely. `tauri-action` supports it natively; it just needs these as GitHub Actions secrets, and `release.yml`'s `env:` block updated to pass them through:

- `APPLE_CERTIFICATE`, a Developer ID Application certificate, exported as a base64-encoded `.p12`
- `APPLE_CERTIFICATE_PASSWORD`, the password used when exporting that `.p12`
- `APPLE_SIGNING_IDENTITY`, the certificate's common name, e.g. `Developer ID Application: Your Name (TEAMID)`
- `APPLE_ID` / `APPLE_PASSWORD`, an Apple ID and an app-specific password for it (not the account password), used to submit the build for notarization
- `APPLE_TEAM_ID`, the Apple Developer Team ID

Once those are wired up, `signingIdentity` in `tauri.conf.json` should be changed from `"-"` to the real `APPLE_SIGNING_IDENTITY` value (Tauri accepts the identity from either the config or the env var, not both at once, so pick one and drop the other), and the `releaseBody` note in `release.yml` covering this issue can be deleted.

## 6. Mobile builds (`publish-android`/`publish-ios`)

Both run after `publish-tauri` and upload to the same draft release it created, with filenames that spell out the platform (`Seal-{version}-android-arm64.apk`, `Seal-{version}-iOS.ipa`) — unlike the desktop artifacts above, which keep whatever names `tauri-action` gives them, since those have to stay consistent with what `latest.json` (the auto-updater manifest) expects. Mobile has no such constraint: `tauri-plugin-updater` isn't compiled in on iOS/Android at all.

Android's Rust/Gradle project (`src-tauri/gen/android`) isn't committed to the repo the way iOS's (`src-tauri/gen/apple`) is — the workflow runs `tauri android init` fresh every time instead, so there's no generated Android Studio project to go stale.

### Android release signing

Without a configured keystore, `publish-android` builds a **debug-signed** APK: fine for sideloading and testing, not something to hand out as a real release (every debug build shares the same well-known debug key, so it isn't meaningfully signed by *this project*). To get a real release-signed APK, add these secrets:

- `ANDROID_KEYSTORE_BASE64`, a release keystore (`keytool -genkeypair -v -keystore release.keystore -alias seal -keyalg RSA -keysize 2048 -validity 10000`), base64-encoded
- `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` for that keystore

No Google Play developer account needed for this part — a self-signed release keystore is enough for anyone to install the APK directly. **Keep the keystore file itself somewhere safe outside git**: losing it means every future release has to switch to a new signing key, which Android treats as a different app for update purposes.

### iOS: no signed IPA yet

`publish-ios` builds and archives with `--no-sign --archive-only`, which validates the Rust/Xcode build on every release but can't produce an installable `.ipa` — iOS requires a real Apple-issued certificate and provisioning profile to sign *anything* installable, with no ad-hoc equivalent to Android's self-signed keystore. That needs the same paid Apple Developer Program membership as macOS notarization (§5). Once that exists:

- Export a Distribution certificate as a base64-encoded `.p12` and add it (plus its password) as secrets, imported into a temporary keychain at the start of the job (`security create-keychain`/`security import`, the standard pattern `tauri-action` itself uses for macOS)
- Add the matching provisioning profile as a secret, installed into `~/Library/MobileDevice/Provisioning Profiles/`
- Point `apps/desktop/src-tauri/gen/apple/ExportOptions.plist` at the real team/signing identity instead of automatic personal-team signing
- Drop `--no-sign`, add `--export-method release-testing` (TestFlight) or `--export-method app-store-connect`, and add an upload step after the build — `publish-android`'s `gh release upload` step above is the template for uploading to the same draft release

This file was augmented/rephrased by Claude Codea
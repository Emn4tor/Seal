## Summary

What does this change, and why. Link any related issue.

## Which crates/apps does this touch

- [ ] `crates/wire-proto`
- [ ] `crates/identity`
- [ ] `crates/storage`
- [ ] `crates/net`
- [ ] `crates/crypto-session`
- [ ] `crates/core`
- [ ] `crates/directory-server`
- [ ] `apps/desktop` (Tauri/React frontend)
- [ ] Docs/CI/scripts only

## Testing

How you verified this works. `cargo test --workspace` and `npm run test` (from `apps/desktop`) are the baseline. Note anything you tested manually, especially if it involves running two instances to exercise messaging.

## Checklist

- [ ] `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` pass locally
- [ ] `cargo test --workspace` passes locally
- [ ] If this touches `apps/desktop`, `npm run build` and `npm run test` pass locally
- [ ] If this changes what gets logged, I checked it against the rule in [`docs/SECURITY.md`](../docs/SECURITY.md#logging): no plaintext, key material, or session state
- [ ] If this changes what's protected or not, I updated [`docs/THREAT_MODEL.md`](../docs/THREAT_MODEL.md)
- [ ] I read [`CONTRIBUTING.md`](../CONTRIBUTING.md)

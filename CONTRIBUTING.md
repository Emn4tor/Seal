# Contributing to Seal

Thanks for taking the time to contribute. This is a personal, early-stage project, so please keep expectations calibrated to that: review may take a while, and not every idea will fit the project's direction. That said, bug reports, small fixes, and well-scoped features are all welcome.

## Before you start

For anything beyond a small fix, open an issue first (or comment on an existing one) describing what you'd like to do. This avoids duplicated work and lets us agree on the approach before you sink time into an implementation, especially for anything that touches the wire protocol, the threat model, or cryptography.

Read [`README.md`](README.md) for how the project is laid out, and [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) plus [`docs/SECURITY.md`](docs/SECURITY.md) if your change touches encryption, key handling, storage, or logging.

## Reporting a security issue

Do not open a public issue with exploit details for a vulnerability. See the reporting process in [`docs/SECURITY.md`](docs/SECURITY.md#reporting-a-vulnerability).

## Development setup

Follow [`README.md`](README.md#1-prerequisites) for platform prerequisites, then:

```sh
cargo build --workspace
cd apps/desktop
npm install
npm run tauri dev
```

## Making a change

1. Fork the repo and create a branch off `main`.
2. Make your change. Keep commits focused, one logical change per commit is easier to review than one large commit that mixes several.
3. Add or update tests for the behavior you changed. `crates/core`'s integration tests are a good reference for how this project tests networking code.
4. Run the checks locally before opening a pull request:

   ```sh
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace

   cd apps/desktop
   npm run build
   npm run test
   ```

5. If your change affects what gets logged, re-read [`docs/SECURITY.md`](docs/SECURITY.md#logging) first: no message plaintext, key material, or session state in any log line.
6. Open a pull request against `main` and fill in the template. Explain what changed and why, not just what.

## Code style

- Rust: standard `rustfmt` formatting, and `clippy` must be clean with `-D warnings`. CI enforces both.
- TypeScript/React: match the existing style in `apps/desktop/src`. There's no separate linter configured beyond the TypeScript compiler (`npm run build` runs `tsc`), so keep types honest rather than relying on a linter to catch it.
- Comments should explain *why*, not *what*, especially for anything non-obvious like the CI workarounds in `.github/workflows/ci.yml`. Match that project's existing tone: precise and specific rather than generic.

## Commit messages

Short, imperative summary line (`fix: ...`, `feat: ...`, `docs: ...` prefixes match the existing history, though they're not strictly required). Explain the reasoning in the body if it isn't obvious from the diff alone.

## What we're looking for

Small, well-tested, well-explained changes are far easier to merge than large ones. If you're planning something large, please open an issue first so we can talk through the design before code gets written.

## License

By contributing, you agree that your contributions are licensed under the same [GNU General Public License version 2](LICENSE) that covers the rest of this project.

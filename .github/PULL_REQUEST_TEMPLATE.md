## What this changes

<!-- A sentence or two: what does this pull request do, and why? -->

Fixes #<!-- issue number, if there is one -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Documentation
- [ ] Tests or CI only
- [ ] Refactor or performance (no change in output)

## Checklist

See [CONTRIBUTING.md](https://github.com/lukaj99/ticoraw/blob/main/CONTRIBUTING.md) for
details. Tick what applies; it is fine to open a draft and ask for help with any item.

- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo test --workspace --release` passes.
- [ ] Library changes only: `cargo +1.85 check --workspace --all-targets` and
      `cargo build -p ticoraw --no-default-features --target wasm32-unknown-unknown` pass.
- [ ] New behaviour has tests; malformed input returns a `TicoError`, never a panic.
- [ ] `CHANGELOG.md` has an entry under `[Unreleased]` (if users or library callers would
      notice the change).
- [ ] Docs (README, `docs/FORMAT.md`, `docs/compat.md`) are updated where needed.

### Bit-exactness

- [ ] The **Conformance** workflow (13 public samples against Adobe-verified hashes) is
      green on this pull request. It runs automatically; if it fails, say what you found.
- [ ] The **Bench** instruction-count gate is green, or the slowdown is explained below.

For the Adobe ground-truth suite:

- [ ] This change does not touch decoding or DNG output, **or**
- [ ] I ran the ground-truth suite on the conformance samples and every pair still matches
      (paste the summary below), **or**
- [ ] I could not run it and ask the maintainer to.

<!-- Ground-truth summary, if you ran it:

```
TICORAW_CORPUS=... cargo test --release --test ground_truth -- --nocapture
...
```
-->

### Clean room and privacy

- [ ] I did not read or copy code from Adobe, intoPIX, LibRaw, dnglab or darktable, and
      used Adobe tools only as black-box references.
- [ ] This pull request contains no raw files, DNGs, serial numbers, GPS data, personal
      file names or paths from my machine.

## Notes for the reviewer

<!-- Anything you are unsure about, alternatives you tried, or where you would like a second opinion. -->

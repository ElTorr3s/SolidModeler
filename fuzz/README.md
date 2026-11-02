# Fuzzing

Targets (`cargo fuzz list`):

| Target | Input | Looks for |
|---|---|---|
| `smp_read` | raw bytes as an `.smp` | panics, decompression bombs (`-rss_limit_mb`) |
| `smp_document` | `document.json` in a valid archive, then rebuild | parser memory blowups, slow rebuilds (`-timeout`) |
| `rebuild` | generated always-valid histories up to the kernel caps | escaped panics, slow or huge rebuilds, inexact save/reopen |
| `step_read` | text through truck-stepio's STEP reader | panics in the Part 21 parser |

Build and run from the repository root. Stable plus `RUSTC_BOOTSTRAP=1` is
used because ruststep's `branches` dependency does not build on current
nightlies:

```sh
export RUSTC_BOOTSTRAP=1
cargo fuzz run -O smp_document fuzz/corpus/smp_document -- \
  -dict=fuzz/smp.dict -rss_limit_mb=2048 -timeout=10 -max_len=200000
```

- `corpus/` holds committed seeds (`cargo run --manifest-path fuzz/Cargo.toml
  --example make_corpus` regenerates them). CI fuzzes from them for 60 s per
  target on every pull request.
- `regressions/` holds reproducers for findings that are still open. The
  nightly workflow replays them and stays red until each is fixed; when one
  passes, move it into `corpus/<target>/` so it guards against regressions.

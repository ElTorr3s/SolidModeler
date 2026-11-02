# sm-occt fuzz targets

The STEP worker's output reaches the app process as OpenCascade binary
B-rep, so it is screened in Rust (`sm_occt::brep_screen`) before
OpenCascade reads it. These targets hammer that path:

- `brep_screen`: the Rust screen alone. It must never panic.
- `brep_load`: screen, then OpenCascade's reader (`Shape::load_brep`),
  exactly as the app runs it on worker output. It must never crash.

Needs nightly Rust, `cargo install cargo-fuzz`, and the system
OpenCascade (as for building `sm-occt`). From this directory:

```sh
mkdir -p corpus/brep_load && cp seeds/* corpus/brep_load/
cargo +nightly fuzz run brep_load corpus/brep_load -- -max_len=65536
```

`seeds/` holds B-reps of kernel results (box, booleans, fillet, shell,
sweeps). Regenerate them with
`SM_FUZZ_SEEDS=$PWD/seeds cargo test -p sm-occt write_fuzz_seeds -- --ignored`.

Current nightlies fail to build the `branches` crate (pulled in through
`truck`); a nightly from 2025-09-01 works.

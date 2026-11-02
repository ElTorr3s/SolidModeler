# SolidModeller

A history-based (parametric) solid modeler in Rust, in the spirit of Autodesk
Inventor and SolidWorks. Parts are edited as a feature history, saved in the
native `.smp` format, and exchanged with other CAD tools as STEP.

Target platform: Linux on Wayland, with first-class support for
[Omarchy](https://omarchy.org) (Arch + Hyprland), including following the
system theme.

## Status

Step 1 of the [roadmap](docs/roadmap.md): the workspace, the feature history
model, the `.smp` file format, and a minimal sketch → extrude → STEP pipeline.
There is no GUI yet.

## Layout

| Crate | Purpose |
|---|---|
| `crates/sm-document` | Part document: parameters and ordered feature history, rollback marker |
| `crates/sm-kernel` | Replays the history into B-rep solids using [`truck`](https://github.com/ricosjp/truck) |
| `crates/sm-format` | Reads and writes `.smp` files ([spec](docs/smp-format.md)) |
| `crates/sm-step` | STEP export (geometry only) |
| `crates/sm-sketch` | 2D sketch geometry and constraint solver (degrees of freedom, over-constraint detection) |
| `apps/smcli` | Command-line driver until the GUI exists |

## Try it

```sh
cargo test
cargo run -p smcli -- demo block.smp            # 40×20×10 mm block
cargo run -p smcli -- rebuild block.smp block.step
```

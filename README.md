# SolidModeller

A history-based (parametric) solid modeler in Rust, in the spirit of Autodesk
Inventor and SolidWorks. Parts are edited as a feature history, saved in the
native `.smp` format, and exchanged with other CAD tools as STEP.

Target platform: Linux on Wayland, with first-class support for
[Omarchy](https://omarchy.org) (Arch + Hyprland), including following the
system theme.

## Status

Step 2 of the [roadmap](docs/roadmap.md): polyline sketches (with holes)
extruded with Join / Cut / Intersect / New Solid, saved as `.smp`, and
exported to STEP. There is no GUI yet.

## Layout

| Crate | Purpose |
|---|---|
| `crates/sm-document` | Part document: parameters and ordered feature history, rollback marker |
| `crates/sm-kernel` | Replays the history into B-rep solids using [`truck`](https://github.com/ricosjp/truck) |
| `crates/sm-naming` | Persistent face/edge/vertex names that survive rebuilds ([design](docs/topological-naming.md)) |
| `crates/sm-naming-types` | The name types alone (serde only), for crates that store references |
| `crates/sm-format` | Reads and writes `.smp` files ([spec](docs/smp-format.md)) |
| `crates/sm-step` | STEP export (geometry only) |
| `apps/smcli` | Command-line driver until the GUI exists |

## Try it

```sh
cargo test
cargo run -p smcli -- demo block.smp            # block with a pocket and a notch
cargo run -p smcli -- rebuild block.smp block.step
```

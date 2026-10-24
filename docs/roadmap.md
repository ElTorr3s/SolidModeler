# Roadmap

Long-term goal: professional use with Autodesk Inventor feature parity.

1. **Foundations** *(this PR)*. Rust workspace, feature history model, native
   `.smp` format, sketch → extrude → STEP pipeline on `truck`.
2. **Kernel core.** Booleans (join, cut, intersect) so features act on one
   body, more surface types, STEP import, and AP242 output (truck currently
   writes the AP214 schema).
3. **Sketcher and first features.** Lines, arcs, splines, a 2D constraint
   solver, parameters and equations, then revolve, fillet, chamfer, and hole,
   with Inventor-style rollback and edit-in-place.
4. **Viewer and UI on Omarchy.** Wayland-native window (winit + wgpu) that
   runs well under Hyprland, with a 3D viewport and an Inventor-style model
   browser. The UI follows the OS theme: light/dark from the XDG desktop
   portal `color-scheme` setting, and colors from the active Omarchy theme,
   updated live when the user switches themes. Falls back to plain light and
   dark themes on other desktops.

Progress against Inventor and SolidWorks, feature by feature, is tracked in
[feature-parity.md](feature-parity.md), which also proposes the steps after 4.

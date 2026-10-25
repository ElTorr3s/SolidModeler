# Roadmap

**Vision:** Autodesk Inventor's modeling and workflow, with Onshape's file
and version management (history, branches, merges), running locally on an
Omarchy machine instead of in the cloud. Everything works offline. Files and
version history live on the user's disk, with no server or account needed.

Long-term goal: professional use with Inventor feature parity, on
Linux/Wayland with first-class Omarchy support.

1. **Foundations** *(done)*. Rust workspace, feature history model, native
   `.smp` format, sketch → extrude → STEP pipeline on `truck`.
2. **Kernel core** *(in progress)*.
   - Done: Join / Cut / Intersect / New Solid on extrudes, including flush
     (coplanar) faces, which truck's booleans can't handle directly. Holes
     from nested sketch loops. Unit-correct STEP export with a valid header.
     Input validation and resource limits for untrusted files.
   - Next: STEP import (needs a layer converting truck-stepio's geometry
     types into the modeling kernel's), more surface types, AP242 output
     (truck writes AP214 entities today).
3. **Sketcher and first features.** Lines, arcs, splines, a 2D constraint
   solver, parameters and equations, then revolve, fillet, chamfer, and hole.
   Alongside: an incremental rebuilder that recomputes only from the first
   changed feature, and Inventor-style sick features (a failing feature is
   flagged and skipped instead of aborting the whole rebuild).
4. **Viewer and UI on Omarchy.** Wayland-native window (winit + wgpu) that
   runs well under Hyprland, with a 3D viewport and an Inventor-style model
   browser. The UI follows the OS theme: light/dark from the XDG desktop
   portal `color-scheme` setting, and colors from the active Omarchy theme,
   updated live when the user switches themes. Falls back to plain light and
   dark themes on other desktops.
5. **Versions, branches and merges (local-first).** Onshape-style history on
   the feature tree itself, stored locally next to the part (no cloud): every edit is a change to the feature list, branches can be
   created and merged at the feature level, and geometry is rebuilt from
   whatever version is checked out. Edits to different features merge
   automatically; edits to the same feature need a manual pick. Depends on
   robust topological naming, so references like "this face" survive a merge.
   - Done (first version): version graph stored in `.smp`, branches, tags,
     checkout, three-way feature-level merge with conflict reporting, and
     `smcli` commands. See `docs/versioning.md`.
   - Next: local project manager in the style of Onshape (project folders,
     project-wide releases, where-used across parts and assemblies, a
     version manager UI).

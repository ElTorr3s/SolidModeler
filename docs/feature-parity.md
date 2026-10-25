# Feature parity: Inventor, SolidWorks and Onshape

The vision: Inventor's modeling and workflow, with Onshape's file and version
management, running entirely on a local Omarchy machine. No cloud, no
account, no server.

This is the checklist for that goal. It covers Autodesk Inventor feature
parity from the [roadmap](roadmap.md), the SolidWorks features worth having,
and Onshape's data management features rebuilt on local files, listed in
[their own section](#data-management-and-version-control). It is grouped by
environment and then by tool group, the way the products lay out their
commands.

Each feature appears once. When the products have it under different names,
the row gives each name. A `—` in a column means that product has no equivalent,
so the feature is unique to the other one.

Update the status in the same PR that lands the work, then refresh the
summary with `python3 docs/parity-summary.py`.

## How to read the tables

**Step** is the roadmap step that delivers the item.

| Step | Scope |
|---|---|
| 1 | Foundations: workspace, history model, `.smp`, sketch → extrude → STEP |
| 2 | Kernel core: booleans, more surface types, STEP import, AP242 |
| 3 | Sketcher, parameters, first features, rollback and edit-in-place |
| 4 | Viewer and UI on Omarchy |
| 5 | Versions and the project store: continuous history, versions, branches, merge, compare (Onshape-style, local, no cloud) |
| 6 | *Proposed:* rest of part modeling (work features, patterns, sweeps, lofts, shell, properties, materials) |
| 7 | *Proposed:* assemblies, cross-document references, release management |
| 8 | *Proposed:* drawings and model-based definition |
| 9 | *Proposed:* sheet metal |
| 10 | *Proposed:* advanced modeling: surfacing, freeform, plastic part, rules and automation, table-driven families, weldments and frames |
| 11 | *Proposed:* add-ins and analysis: standard parts libraries, design accelerators, simulation, routing, presentations |

Step 5 follows decision 14 in [decisions.md](decisions.md). Steps 6 to 11
are a suggested order only.

**Status** is one of:

- `not started`
- `in progress`: some of it works (the notes say what)
- `done`: matches the reference behaviour for normal professional use

*(low priority)* marks items that are rarely used in practice or tied to the
vendor's cloud services. They stay listed so nothing gets forgotten.

## Summary

<!-- summary:start -->
| Environment | Items | Inventor and SolidWorks | Inventor only | SolidWorks only | From Onshape | Ours only | Done | In progress |
|---|---|---|---|---|---|---|---|---|
| Core document and history | 25 | 21 | 0 | 4 | 0 | 0 | 0 | 5 |
| Sketch (2D) | 69 | 59 | 2 | 8 | 0 | 0 | 0 | 1 |
| 3D sketch and curves | 15 | 13 | 0 | 2 | 0 | 0 | 0 | 0 |
| Part modeling | 86 | 61 | 4 | 21 | 0 | 0 | 0 | 1 |
| Parameters, properties and automation | 24 | 18 | 5 | 1 | 0 | 0 | 0 | 1 |
| Sheet metal | 31 | 22 | 2 | 7 | 0 | 0 | 0 | 0 |
| Surfacing and freeform | 24 | 11 | 7 | 6 | 0 | 0 | 0 | 0 |
| Plastic part and mold | 10 | 7 | 3 | 0 | 0 | 0 | 0 | 0 |
| Assemblies | 67 | 48 | 4 | 15 | 0 | 0 | 0 | 0 |
| Weldments and frames | 17 | 13 | 2 | 2 | 0 | 0 | 0 | 0 |
| Standard parts and design accelerators | 16 | 11 | 4 | 1 | 0 | 0 | 0 | 0 |
| Drawings | 67 | 56 | 2 | 9 | 0 | 0 | 0 | 0 |
| Model-based definition | 4 | 3 | 0 | 1 | 0 | 0 | 0 | 0 |
| Presentations and animation | 8 | 7 | 1 | 0 | 0 | 0 | 0 | 0 |
| Inspection, analysis and simulation | 25 | 11 | 2 | 12 | 0 | 0 | 0 | 0 |
| Tube, pipe and electrical routing | 11 | 11 | 0 | 0 | 0 | 0 | 0 | 0 |
| File I/O and interoperability | 26 | 24 | 1 | 0 | 0 | 1 | 1 | 2 |
| UI, viewport and navigation | 37 | 32 | 0 | 3 | 0 | 2 | 0 | 0 |
| Data management and version control | 29 | 1 | 0 | 0 | 28 | 0 | 0 | 0 |
| **Total** | **591** | **429** | **39** | **92** | **28** | **3** | **1** | **10** |
<!-- summary:end -->

---

## Core document and history

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Part (.ipt) | Part (.sldprt) | 1 | in progress | `PartDocument` with name, units, parameters, features |
| Model browser | FeatureManager design tree | 1 | in progress | Flat ordered list; no folders or nesting yet |
| Origin planes, axes, center point | Origin, Front/Top/Right planes | 1 | in progress | XY/YZ/XZ planes exist; no origin axes or point |
| End of Part marker | Rollback bar | 1 | in progress | `end_of_part` stored; no UI to drag it yet |
| Suppress features | Suppress / unsuppress | 1 | in progress | `suppressed` flag stored on each feature |
| Edit feature, edit sketch | Edit feature, edit sketch | 3 | not started | |
| Reorder features | Reorder features | 3 | not started | Dependency checks on drop |
| Delete feature (consume or retain dependents) | Delete (with absorbed features) | 3 | not started | |
| Rebuild All / Update | Rebuild / Force Rebuild | 3 | not started | |
| Sick feature markers, Design Doctor | What's Wrong, rebuild errors | 3 | not started | |
| — | FeatureXpert | 6 | not started | Auto-fixes failing fillets and drafts |
| — | Freeze bar | 6 | not started | Locks features above the bar from rebuilding |
| Stable topological naming | Stable face/edge IDs | 2 | not started | Not a command; needed so references survive edits |
| Parent/child relationships | Parent/Child | 3 | not started | |
| Browser folders | Folders | 6 | not started | |
| Rename nodes | Rename | 3 | not started | |
| — | Comments on features | 6 | not started | |
| Undo / redo | Undo / redo | 4 | not started | |
| Solid Bodies folder (multi-body) | Solid Bodies folder (multi-body) | 6 | not started | |
| Surface Bodies folder | Surface Bodies folder | 10 | not started | |
| Visibility per feature/body/work feature | Hide/show | 4 | not started | |
| Model States | Configurations | 10 | not started | Variations of one part in one file |
| Design Views | Display States, Named Views | 6 | not started | Saved camera, visibility, appearance |
| Document Settings | Document Properties | 3 | not started | Length units only so far |
| — | Sensors | 10 | not started | Alerts when mass, dimension, interference cross limits |

## Sketch (2D)

### Create

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Start 2D Sketch on plane | Sketch on plane | 1 | in progress | Origin planes with offset; closed polylines only |
| Sketch on face / work plane | Sketch on face / plane | 3 | not started | |
| Line (with arc gesture) | Line, Centerline, Midpoint Line | 3 | not started | |
| Spline: control vertex | Style Spline | 3 | not started | |
| Spline: interpolation | Spline | 3 | not started | |
| — | Spline on Surface | 10 | not started | |
| Equation Curve | Equation Driven Curve | 6 | not started | |
| Bridge Curve | — | 6 | not started | |
| — | Conic | 6 | not started | |
| — | Parabola | 6 | not started | |
| Circle: center point | Circle | 3 | not started | |
| Circle: tangent | Perimeter Circle | 3 | not started | |
| Ellipse | Ellipse, Partial Ellipse | 3 | not started | |
| Arc: three point, tangent, center point | 3 Point Arc, Tangent Arc, Centerpoint Arc | 3 | not started | |
| Rectangle: two point, three point, center variants | Corner, Center, 3 Point Corner, 3 Point Center Rectangle, Parallelogram | 3 | not started | |
| Slot: all five types | Straight, Centerpoint Straight, 3 Point Arc, Centerpoint Arc Slot | 3 | not started | |
| Polygon | Polygon | 3 | not started | |
| Fillet (2D) | Sketch Fillet | 3 | not started | |
| Chamfer (2D) | Sketch Chamfer | 3 | not started | |
| Point, Center Point | Point | 3 | not started | Hole centers |
| Text, Geometry Text | Text (on curve) | 6 | not started | |
| Construction, Centerline style | For construction | 3 | not started | |
| Project Geometry | Convert Entities | 3 | not started | |
| Project Cut Edges | Intersection Curve | 3 | not started | |
| Import AutoCAD into sketch | DXF/DWG import to sketch | 6 | not started | |
| Insert Image | Sketch Picture | 6 | not started | |
| Insert points from Excel | Curve Through XYZ Points | 6 | not started | *(low priority)* |
| — | Autotrace (picture to sketch) | 10 | not started | *(low priority)* |
| — | Sketch Ink (pen input) | 11 | not started | *(low priority)* |

### Modify and pattern

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Move, Copy, Rotate | Move, Copy, Rotate Entities | 3 | not started | |
| Trim | Trim Entities (power trim) | 3 | not started | |
| Extend | Extend Entities | 3 | not started | |
| Split | Split Entities | 3 | not started | |
| Scale | Scale Entities | 3 | not started | |
| Stretch | Stretch Entities | 3 | not started | |
| Offset | Offset Entities | 3 | not started | |
| Rectangular Pattern | Linear Sketch Pattern | 3 | not started | |
| Circular Pattern | Circular Sketch Pattern | 3 | not started | |
| Mirror | Mirror Entities | 3 | not started | |
| — | Dynamic Mirror | 3 | not started | Mirrors while drawing |

### Constrain

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| 2D constraint solver | 2D relation solver | 3 | not started | Core of the sketcher |
| Dimension | Smart Dimension | 3 | not started | Linear, aligned, radial, diameter, angular |
| Driven dimension | Driven dimension | 3 | not started | |
| Dimension expressions | Equations in dimensions | 3 | not started | `width / 2` style |
| Automatic Dimensions and Constraints | Fully Define Sketch | 3 | not started | |
| Coincident | Coincident | 3 | not started | |
| — | Midpoint | 3 | not started | Inventor uses coincident to a midpoint |
| Collinear | Collinear | 3 | not started | |
| Concentric | Concentric, Coradial | 3 | not started | |
| Fix | Fix | 3 | not started | |
| Parallel | Parallel | 3 | not started | |
| Perpendicular | Perpendicular | 3 | not started | |
| Horizontal / Vertical | Horizontal / Vertical | 3 | not started | |
| Tangent | Tangent | 3 | not started | |
| Smooth (G2) | Equal Curvature | 3 | not started | |
| Symmetric | Symmetric | 3 | not started | |
| Equal | Equal | 3 | not started | |
| — | Merge, Pierce | 6 | not started | Pierce ties a sketch point to a 3D curve |
| Constraint inference | Automatic relations | 3 | not started | |
| Show / Delete Constraints | Display/Delete Relations | 3 | not started | |
| Relax Mode | Drag with SketchXpert | 3 | not started | |
| Degrees of freedom, fully constrained colour | Under/fully/over defined colours | 3 | not started | |
| Over-constrained prompt | SketchXpert | 3 | not started | |

### Blocks and tools

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Sketch Blocks | Blocks | 10 | not started | Skeleton and layout workflows |
| Share Sketch | Shared sketch | 6 | not started | |
| Sketch Doctor | Check Sketch for Feature, Repair Sketch | 3 | not started | |
| Profile selection with nested loops | Contour Select | 3 | not started | |
| Slice Graphics | — | 4 | not started | |
| Layout part | Layout sketch | 7 | not started | |

## 3D sketch and curves

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| 3D Sketch | 3D Sketch | 6 | not started | |
| 3D Line with auto-bend | 3D Line | 6 | not started | |
| 3D spline | 3D Spline | 6 | not started | |
| Helical Curve | Helix and Spiral | 6 | not started | |
| Equation Curve (3D) | Equation Driven Curve (3D) | 6 | not started | |
| Bend | Sketch Fillet (3D) | 6 | not started | |
| Project to Surface | Projected Curve | 10 | not started | |
| Intersection Curve | Intersection Curve | 6 | not started | |
| Silhouette Curve | Silhouette Entities | 10 | not started | |
| Include Geometry | Convert Entities (3D) | 6 | not started | |
| 3D constraints and dimensions | 3D relations and dimensions | 6 | not started | |
| — | Composite Curve | 6 | not started | |
| — | Split Line | 6 | not started | Splits faces by a curve |
| Curve on face | Face Curves | 10 | not started | |
| 3D Transform | Triad move | 6 | not started | |

## Part modeling

### Create

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Extrude: distance, symmetric | Extruded Boss: Blind, Mid Plane | 1 | in progress | New solid only |
| Extrude: join / cut / intersect / new solid | Extruded Boss / Cut, Merge result | 2 | not started | Needs booleans |
| Extrude: two directions, asymmetric | Direction 1 / Direction 2 | 3 | not started | |
| Extrude: to, to next, through all, between | Up to Vertex/Surface/Body, Up to Next, Through All | 3 | not started | |
| — | Offset from Surface end condition, Thin Feature | 6 | not started | |
| Extrude taper | Draft while extruding | 6 | not started | |
| Revolve | Revolved Boss / Cut | 3 | not started | |
| Sweep (path, guide rail, guide surface) | Swept Boss / Cut (guide curves) | 6 | not started | |
| Sweep taper and twist | Sweep twist along path | 6 | not started | |
| — | Swept Cut with solid tool body | 10 | not started | Machining tool paths |
| Loft (rails, center line, area loft) | Lofted Boss / Cut | 6 | not started | |
| Loft point mapping, conditions | Connectors, start/end constraints | 6 | not started | |
| — | Boundary Boss / Cut | 10 | not started | |
| Coil | — | 6 | not started | SolidWorks sweeps along a helix instead |
| Rib | Rib | 6 | not started | |
| Emboss | Wrap (emboss, deboss, scribe) | 6 | not started | |
| Decal | Decals | 10 | not started | |
| Derive | Insert Part, Mirror Part | 7 | not started | Link-in with scale, mirror, body selection |
| Import (base feature) | Imported feature | 2 | not started | See File I/O |
| — | FeatureWorks (feature recognition on imports) | 11 | not started | |
| Primitives (box, cylinder, sphere, torus) | — | 6 | not started | |
| — | Dome | 6 | not started | |

### Modify

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Hole (simple, counterbore, countersink, spotface) | Hole Wizard | 3 | not started | |
| Hole: tapped, taper tapped, clearance | Hole Wizard: tap, pipe tap, legacy | 3 | not started | Thread data tables |
| Hole placement | Hole Wizard positions | 3 | not started | |
| — | Advanced Hole (stacked near/far elements) | 6 | not started | |
| — | Hole Series (across assembly parts) | 7 | not started | |
| Fillet: constant | Fillet: constant size | 3 | not started | |
| Fillet: variable | Fillet: variable size | 6 | not started | |
| Fillet: setbacks | Fillet: setback | 6 | not started | |
| Fillet: face | Fillet: face | 6 | not started | |
| Fillet: full round | Fillet: full round | 6 | not started | |
| Fillet: G2 / inverted conic | Fillet: curvature continuous, conic | 6 | not started | |
| — | FilletXpert | 6 | not started | |
| Chamfer (distance, angle, two distances) | Chamfer (angle-distance, distance-distance, vertex) | 3 | not started | |
| Partial chamfer | Offset face, face-face chamfer | 6 | not started | |
| Shell | Shell | 6 | not started | |
| Draft (fixed edge, fixed plane, parting line) | Draft (neutral plane, parting line, step draft) | 6 | not started | |
| — | DraftXpert | 6 | not started | |
| Thread | Cosmetic Thread, Thread | 6 | not started | |
| — | Stud Wizard | 6 | not started | |
| Split (face, trim solid, split solid) | Split, Split Line | 6 | not started | |
| Combine | Combine | 6 | not started | |
| — | Intersect (regions from bodies and surfaces) | 10 | not started | |
| Thicken/Offset | Thicken | 10 | not started | |
| Direct Edit (move, resize, rotate, scale) | Move Face, Instant3D | 10 | not started | |
| — | Scale (feature) | 6 | not started | Inventor scales via Derive or Direct Edit |
| Delete Face | Delete Face (delete and patch/fill) | 10 | not started | |
| Replace Face | Replace Face | 10 | not started | |
| Copy Object | Move/Copy Bodies (copy) | 10 | not started | |
| Move Bodies | Move/Copy Bodies | 6 | not started | |
| — | Delete/Keep Body | 6 | not started | |
| Bend Part | Flex (bending, twisting, tapering, stretching) | 10 | not started | |
| — | Deform | 10 | not started | |
| — | Indent | 10 | not started | |
| — | Cavity | 11 | not started | Mold cavities from a part |
| Simplify | Defeature | 7 | not started | |
| Make Components | Save Bodies | 7 | not started | Multi-body master workflow |
| iFeatures | Library Features | 10 | not started | |

### Work features (reference geometry)

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Work Plane (all 12 methods) | Plane (all constraint combinations) | 6 | not started | Offset, angle, midplane, three points, tangent, normal to curve |
| Work Axis (all methods) | Axis | 6 | not started | Edge, cylinder, two points, two planes, normal through point |
| Work Point (all methods) | Point | 6 | not started | |
| Grounded Work Point | — | 6 | not started | |
| UCS | Coordinate System | 6 | not started | |
| — | Center of Mass (reference point) | 6 | not started | |
| — | Bounding Box | 6 | not started | Updates with the part; used for stock size |
| iMates | Mate References | 7 | not started | |

### Pattern

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Rectangular Pattern | Linear Pattern | 6 | not started | |
| Circular Pattern | Circular Pattern | 6 | not started | |
| Sketch Driven Pattern | Sketch Driven Pattern | 6 | not started | |
| Pattern along path | Curve Driven Pattern | 6 | not started | |
| Mirror | Mirror | 6 | not started | Features and bodies |
| — | Table Driven Pattern | 6 | not started | |
| — | Fill Pattern | 6 | not started | Perforation layouts |
| — | Variable Pattern | 10 | not started | Each instance gets its own dimension values |
| Pattern options: suppress occurrences, adjust | Instances to skip, vary sketch | 6 | not started | |

### Part-level tools

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Material | Edit Material | 6 | not started | Density drives mass properties |
| Appearance | Appearances | 4 | not started | Per part, body, feature, face |
| Physical properties | Mass Properties | 6 | not started | Mass, volume, area, COG, inertia |
| Region Properties | Section Properties | 3 | not started | |
| Measure | Measure | 4 | not started | Distance, angle, area, loop, minimum distance |
| Section views | Section View | 4 | not started | |
| Make Part | Insert into New Part | 7 | not started | |
| Mark | — | 10 | not started | *(low priority)* |
| Point cloud attach | ScanTo3D, point cloud | 11 | not started | *(low priority)* |
| Mesh feature, mesh to B-rep | Mesh BREP bodies, convert to body | 10 | not started | |

## Parameters, properties and automation

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| User Parameters | Global Variables | 1 | in progress | Name, value, comment stored; nothing references them yet |
| Model parameters (d0, d1…) | Dimension names (D1@Sketch1) | 3 | not started | |
| Equations in parameters, units | Equations | 3 | not started | |
| Text and True/False parameters | — | 10 | not started | |
| Multi-value lists | — | 10 | not started | |
| Key / Export parameters | Link to custom property | 6 | not started | |
| Parameter tolerances | Dimension tolerances | 8 | not started | |
| Linked parameters (Excel) | Linked values, external equations file | 10 | not started | |
| iProperties | Custom Properties, Summary Information | 6 | not started | Feed title blocks and parts lists |
| — | Property Tab Builder | 10 | not started | Custom property entry forms |
| Material and appearance libraries | Material database, appearance library | 6 | not started | |
| Style and Standard Editor | Drafting standard (document properties) | 8 | not started | |
| iLogic rules | — | 10 | not started | Scripted rules; pick a language such as Rhai or Lua |
| iLogic event triggers | — | 10 | not started | |
| iLogic forms | DriveWorksXpress | 10 | not started | |
| iLogic external rules | — | 10 | not started | |
| iLogic component and feature control | Configuration-specific suppression | 10 | not started | |
| iParts | Design Tables (part configurations) | 10 | not started | |
| iAssemblies | Design Tables (assembly configurations) | 10 | not started | |
| iFeature tables | Library feature configurations | 10 | not started | |
| Inventor API | SolidWorks API | 10 | not started | Rust API plus scripting bindings |
| VBA macros | VBA macros, macro recorder | 11 | not started | *(low priority)* |
| Add-ins | Add-ins | 11 | not started | |
| Task Scheduler | Task Scheduler | 11 | not started | Batch jobs |

## Sheet metal

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Sheet Metal Defaults, Styles / Rules | Sheet Metal parameters, Gauge Tables | 9 | not started | Thickness, bend radius, relief, K-factor |
| Unfold rules (K-factor, bend table, equation) | Bend Allowance, Bend Deduction, K-factor, Bend Table | 9 | not started | |
| Face | Base Flange/Tab | 9 | not started | |
| Contour Flange | Base Flange (open sketch), Miter Flange | 9 | not started | |
| Contour Roll | — | 9 | not started | |
| Lofted Flange | Lofted Bend | 9 | not started | |
| Flange | Edge Flange | 9 | not started | |
| Hem | Hem | 9 | not started | |
| Fold | Sketched Bend | 9 | not started | |
| Bend | — | 9 | not started | Bend between two faces |
| — | Jog | 9 | not started | |
| — | Cross Break | 9 | not started | |
| — | Swept Flange | 9 | not started | |
| Corner Seam | Closed Corner | 9 | not started | |
| — | Welded Corner | 9 | not started | |
| Corner Round, Corner Chamfer | Break Corner / Corner Trim | 9 | not started | |
| Corner relief options | Corner Relief | 9 | not started | |
| Cut (across bend) | Extruded Cut (normal cut), Unfold/Fold cut | 9 | not started | |
| Punch Tool | Forming Tools | 9 | not started | |
| — | Sheet Metal Vent | 9 | not started | |
| — | Tab and Slot | 9 | not started | |
| — | Sheet Metal Gusset | 9 | not started | |
| Rip | Rip | 9 | not started | |
| Unfold / Refold | Unfold / Fold | 9 | not started | |
| Convert to Sheet Metal | Convert to Sheet Metal, Insert Bends | 9 | not started | |
| Flat Pattern | Flatten | 9 | not started | |
| Flat pattern extents, bend order | Sheet metal bounding box, bend notes | 9 | not started | |
| Multi-body sheet metal | Multi-body sheet metal | 9 | not started | |
| A-side definition | Fixed face | 9 | not started | |
| Cosmetic centerlines, bend lines | Bend lines | 9 | not started | |
| Export flat pattern (DXF/DWG) | Export to DXF/DWG | 9 | not started | Layer mapping, bend lines |

## Surfacing and freeform

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Extrude / Revolve / Sweep / Loft (surface output) | Extruded / Revolved / Swept / Lofted Surface | 10 | not started | |
| — | Boundary Surface | 10 | not started | |
| Patch (boundary patch) | Filled Surface | 10 | not started | G0/G1/G2 conditions |
| — | Planar Surface | 10 | not started | |
| Stitch | Knit Surface | 10 | not started | |
| Sculpt | Thicken (cut) | 10 | not started | |
| Trim Surface | Trim Surface | 10 | not started | |
| Extend Surface | Extend Surface | 10 | not started | |
| — | Untrim Surface | 10 | not started | |
| Ruled Surface | Ruled Surface | 10 | not started | |
| Thicken/Offset (surface) | Offset Surface | 10 | not started | |
| — | Radiate Surface | 10 | not started | |
| — | Mid-Surface | 10 | not started | |
| Unwrap | Surface Flatten | 10 | not started | |
| Split with surface | Cut With Surface | 10 | not started | |
| Repair Bodies | Import Diagnostics | 10 | not started | Cleans up bad imports |
| Fit Mesh Face | — | 10 | not started | |
| Freeform: box, plane, cylinder, sphere, torus, quadball | — | 10 | not started | T-spline modeling |
| Freeform: face, extrude, revolve, sweep, loft | — | 10 | not started | |
| Freeform: edit form, insert edge/point, subdivide | — | 10 | not started | |
| Freeform: bridge, merge, weld/unweld, crease | — | 10 | not started | |
| Freeform: thicken, match edge, align, flatten, symmetry | — | 10 | not started | |
| Freeform: convert to and from B-rep | — | 10 | not started | |
| — | Freeform (push/pull face deformation) | 10 | not started | Not T-splines; deforms an existing face |

## Plastic part and mold

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Grill | Vent | 10 | not started | |
| Boss | Mounting Boss | 10 | not started | |
| Rest | — | 10 | not started | |
| Snap Fit | Snap Hook, Snap Hook Groove | 10 | not started | |
| Rule Fillet | — | 10 | not started | |
| Lip | Lip/Groove | 10 | not started | |
| Shrinkage allowance | Scale (mold) | 10 | not started | |
| Mold Design (core/cavity, parting surfaces) | Mold Tools (parting lines, shut-off surfaces, parting surface, tooling split, core) | 11 | not started | *(low priority)* |
| Mold base, runners, gates | — | 11 | not started | *(low priority)* |
| Moldflow fill analysis | SolidWorks Plastics | 11 | not started | *(low priority)* |

## Assemblies

### Components

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Assembly (.iam) | Assembly (.sldasm) | 7 | not started | Extend `.smp` or add an assembly format |
| Place Component | Insert Components | 7 | not started | |
| Place imported CAD | Insert STEP component | 7 | not started | |
| Create Component (in place) | New Part (in context) | 7 | not started | |
| Ground | Fix / Float | 7 | not started | |
| Pattern Component | Linear, Circular, Pattern Driven, Sketch Driven, Curve Driven Component Pattern | 7 | not started | |
| — | Chain Component Pattern | 7 | not started | |
| Mirror Components | Mirror Components | 7 | not started | |
| Copy Components | Copy with Mates | 7 | not started | |
| Replace Component | Replace Components | 7 | not started | |
| Demote / Promote | Form New Subassembly / Dissolve | 7 | not started | |
| Flexible subassemblies | Flexible subassemblies | 7 | not started | |
| Edit in place | Edit Part (in context) | 7 | not started | |
| Adaptive parts and features | External references (in-context) | 7 | not started | |
| Virtual components | Virtual components | 7 | not started | |
| Reference components | Envelopes | 7 | not started | |
| Shrinkwrap | Defeature (assembly) | 7 | not started | |
| Substitutes (model states) | Simplified configurations | 10 | not started | |
| — | SpeedPak | 7 | not started | Lightweight subassembly representation |
| Skeletal modeling | Top-down layout / master model | 7 | not started | |
| — | Treehouse (structure planning) | 11 | not started | *(low priority)* |

### Constraints and joints (mates)

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Assembly constraint solver | Mate solver | 7 | not started | |
| Mate / Flush | Coincident, Parallel, Distance | 7 | not started | |
| Angle | Angle | 7 | not started | |
| — | Perpendicular | 7 | not started | Inventor uses a 90° angle |
| Tangent | Tangent | 7 | not started | |
| Insert | Concentric + Coincident | 7 | not started | |
| — | Lock | 7 | not started | |
| Symmetry | Symmetric | 7 | not started | |
| — | Width | 7 | not started | |
| — | Profile Center | 7 | not started | |
| — | Path Mate | 7 | not started | |
| — | Linear/Linear Coupler | 7 | not started | |
| Rotation motion | Gear | 7 | not started | |
| Rotation-Translation | Rack and Pinion, Screw | 7 | not started | |
| Transitional | Cam | 7 | not started | |
| — | Hinge, Slot, Universal Joint | 7 | not started | |
| Constraint Set (UCS to UCS) | — | 7 | not started | |
| Constraint limits | Limit Distance / Limit Angle | 7 | not started | |
| Joints: rigid, rotational, slider, cylindrical, planar, ball | — | 7 | not started | |
| Composite iMates | — | 7 | not started | |
| Alt-drag constraint | SmartMates | 7 | not started | |
| Assemble command | Mate while inserting | 7 | not started | |
| Drive constraint | Motion Study (motor) | 7 | not started | With collision detection |
| Contact Solver | Physical Dynamics | 7 | not started | |
| Sick constraints, isolate errors | MateXpert, mate errors | 7 | not started | |
| Degrees of freedom display | Under-defined markers | 7 | not started | |
| Free Move / Free Rotate | Move / Rotate Component | 7 | not started | |
| Positional Representations | Mate Controller | 7 | not started | |

### Productivity and inspection

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Analyze Interference | Interference Detection | 7 | not started | |
| — | Clearance Verification | 7 | not started | |
| — | Hole Alignment | 7 | not started | |
| Bill of Materials | Bill of Materials (from drawings or assembly) | 7 | not started | Structured, parts only |
| BOM structure: phantom, reference, purchased, inseparable | Child display, exclude from BOM | 7 | not started | |
| Design Views (assembly) | Display States | 7 | not started | |
| Selection filters, select by size, internal | Select Internal, Select Toolbox, Advanced Select | 7 | not started | |
| Isolate | Isolate | 7 | not started | |
| Express Mode | Large Assembly Mode, Large Design Review, Lightweight | 7 | not started | |
| Assembly features (cut, hole, fillet, chamfer, sweep, revolve) | Assembly features | 7 | not started | |
| Weld preparation | — | 10 | not started | |
| Center of gravity display | Center of Mass | 7 | not started | |
| Component suppression | Suppress components | 7 | not started | |
| Resolve Link | Find references | 7 | not started | |
| — | Assembly Visualization | 7 | not started | Sort and colour components by property |
| — | AssemblyXpert | 7 | not started | Performance report |
| — | Performance Evaluation | 7 | not started | Rebuild times per feature/component |
| Exploded views (in Presentations) | Exploded View (in assembly), Explode Line Sketch | 7 | not started | |

## Weldments and frames

Inventor builds these in assemblies; SolidWorks builds them as multi-body
parts. Same tools, different document model.

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Weldment assembly | Weldment (part) | 10 | not started | |
| Preparations | — | 10 | not started | |
| Cosmetic Weld | Weld Bead (cosmetic) | 10 | not started | |
| Fillet Weld | Fillet Bead | 10 | not started | |
| Groove Weld | Weld Bead | 10 | not started | |
| Weld symbols (3D) | Weld symbols | 10 | not started | |
| Machining features | Post-weld features | 10 | not started | |
| Frame Generator: Insert Frame | Structural Member | 10 | not started | |
| Frame member profiles | Weldment profiles library | 10 | not started | |
| Change frame member | Edit structural member | 10 | not started | |
| Miter, Notch, Trim to Frame, Trim/Extend | Trim/Extend, corner treatments | 10 | not started | |
| Lengthen / Shorten | Structural member offsets | 10 | not started | |
| — | End Cap | 10 | not started | |
| — | Gusset | 10 | not started | |
| Frame member info, cut lists | Cut List, cut list properties | 10 | not started | |
| Weld calculator | — | 11 | not started | *(low priority)* |
| Beam and Column Calculator, Frame Analysis | Beam elements in Simulation | 11 | not started | *(low priority)* |

## Standard parts and design accelerators

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Content Center (ISO, DIN, ANSI, JIS, GB…) | Toolbox | 11 | not started | Fasteners, profiles, bearings |
| Content Center Editor | Toolbox Settings | 11 | not started | |
| Bolted Connection | Smart Fasteners | 11 | not started | |
| — | Design Library (drag-and-drop parts, features, annotations) | 10 | not started | |
| Shaft generator | — | 11 | not started | |
| Spur, bevel, worm gears | Toolbox gears (static) | 11 | not started | Inventor generates and calculates |
| Bearing generator and calculator | Toolbox bearings, bearing calculator | 11 | not started | |
| V-belts, synchronous belts | Belt/Chain | 11 | not started | |
| Roller chains | Belt/Chain | 11 | not started | |
| Cam generators | — | 11 | not started | |
| Keys, splines | Toolbox keys | 11 | not started | |
| Clevis, cotter, safety pins | Toolbox pins | 11 | not started | |
| O-rings | Toolbox O-rings | 11 | not started | |
| Spring generators | — | 11 | not started | |
| Machinery's Handbook calculators | Toolbox calculators (beam, bearing) | 11 | not started | Tolerances, fits, screws |
| Joint and brake calculators, power screw | — | 11 | not started | *(low priority)* |

## Drawings

### Views

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Drawing (.idw / .dwg) | Drawing (.slddrw) | 8 | not started | |
| Sheets, borders, title blocks | Sheets, Sheet Formats | 8 | not started | |
| Base View | Model View, Standard 3 View | 8 | not started | |
| Projected View | Projected View | 8 | not started | First and third angle |
| Auxiliary View | Auxiliary View | 8 | not started | |
| Section View (full, half, offset, aligned) | Section View, Aligned Section | 8 | not started | |
| — | Removed Section | 8 | not started | |
| Detail View | Detail View | 8 | not started | |
| Overlay View | Alternate Position View | 8 | not started | |
| Draft View | Empty View | 8 | not started | |
| — | Relative View | 8 | not started | Orients a view by two faces |
| — | Predefined View | 8 | not started | |
| Break | Break View | 8 | not started | |
| Break Out | Broken-out Section | 8 | not started | |
| Slice | — | 8 | not started | |
| Crop | Crop View | 8 | not started | |
| — | 3D Drawing View | 8 | not started | |
| View alignment | Alignment | 8 | not started | |
| View styles: hidden, shaded, raster/precise | Display style, draft quality | 8 | not started | |
| Model state / design view in view | Configuration / display state in view | 8 | not started | |
| Hatching | Area Hatch/Fill | 8 | not started | Per material, per part |
| Exclude components from section | Section scope | 8 | not started | |
| Sketches on sheets and views | Sketch in drawing | 8 | not started | |
| Flat pattern view | Flat pattern view | 9 | not started | |
| — | Detailing Mode | 8 | not started | Opens large drawings without loading models |

### Annotate

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Dimension | Smart Dimension | 8 | not started | |
| Baseline, Ordinate, Chain dimension sets | Baseline, Ordinate, Chain Dimension | 8 | not started | |
| Retrieve Dimensions | Model Items | 8 | not started | |
| Arrange Dimensions | Auto Arrange Dimensions | 8 | not started | |
| — | Autodimension | 8 | not started | |
| Tolerances and fits | Tolerance/Precision, fits | 8 | not started | |
| Hole and Thread Notes | Hole Callout | 8 | not started | |
| Chamfer Note | Chamfer Dimension | 8 | not started | |
| Bend notes, punch notes | Bend notes | 9 | not started | |
| Center Mark, Centerline, Bisector, Centered Pattern | Center Mark, Centerline | 8 | not started | |
| Automated Centerlines | Auto insert center marks | 8 | not started | |
| Surface Texture | Surface Finish | 8 | not started | |
| Welding symbols, caterpillar, end fill | Weld Symbol, weld beads | 10 | not started | |
| Feature Control Frame | Geometric Tolerance | 8 | not started | |
| Datum Identifier, Datum Target | Datum Feature, Datum Target | 8 | not started | |
| Feature Identifier | — | 8 | not started | |
| Sketched Symbols | Blocks | 8 | not started | |
| Text, Leader Text | Note | 8 | not started | |
| Revision Cloud | Revision Cloud | 8 | not started | |
| Balloon, Auto Balloon | Balloon, Auto Balloon, Stacked Balloon | 8 | not started | |
| — | Magnetic Line | 8 | not started | |
| Parts List | Bill of Materials table | 8 | not started | |
| Revision Table, revision tags | Revision Table | 8 | not started | |
| General Table | General Table | 8 | not started | |
| Hole Table | Hole Table | 8 | not started | |
| Bend Table, Punch Table | Bend Table, Punch Table | 9 | not started | |
| Weld bead table, cut list table | Weldment Cut List, Weld Table | 10 | not started | |
| Layers | Layers | 8 | not started | |
| Drafting standards (ANSI, ISO, DIN, JIS, GB, BSI) | Drafting standards | 8 | not started | |
| Prompted entries, property fields | Linked notes to properties | 8 | not started | |
| Drawing Resources | Sheet format, block libraries | 8 | not started | |
| Templates | Templates | 8 | not started | |
| Associative update, orphaned annotations | Dangling annotations | 8 | not started | |

### Output and management

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Print | Print | 8 | not started | |
| Export PDF | Save as PDF | 8 | not started | |
| Export DWG / DXF | Save as DWG / DXF | 8 | not started | |
| Export images | Save as image | 8 | not started | |
| Move / copy views between sheets | Move / copy views | 8 | not started | |
| Replace Model Reference | Replace Model | 8 | not started | |
| — | Design Checker | 8 | not started | Checks drawings against company standards |
| — | DrawCompare / Compare Documents | 8 | not started | |
| Shared Views | eDrawings, markup | 11 | not started | *(low priority)* |

## Model-based definition

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| 3D Annotations | DimXpert, MBD 3D PMI | 8 | not started | Goes out through STEP AP242 semantic PMI |
| Tolerance Advisor | DimXpert manager, auto-dimension scheme | 8 | not started | |
| Tolerance Analysis (stack-up) | TolAnalyst | 11 | not started | *(low priority)* |
| — | 3D PDF publishing (MBD) | 11 | not started | *(low priority)* |

## Presentations and animation

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Presentation (.ipn) | — | 11 | not started | SolidWorks does this in the assembly |
| Storyboards, timeline | Motion Study (Animation) | 11 | not started | |
| Tweak Components | Explode steps | 11 | not started | |
| Snapshot Views | Exploded view configurations | 11 | not started | |
| Auto Explode | Auto-space explode | 11 | not started | |
| Camera actions, opacity | Camera views, appearance keys | 11 | not started | |
| Publish video | Save animation (video) | 11 | not started | |
| Publish raster images | Save as image | 11 | not started | |

## Inspection, analysis and simulation

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Zebra Analysis | Zebra Stripes | 10 | not started | |
| Draft Analysis | Draft Analysis | 10 | not started | |
| Curvature analysis, curvature comb | Curvature, Curvature Combs | 10 | not started | |
| Surface Analysis (Gaussian) | — | 10 | not started | |
| Cross Section Analysis | — | 10 | not started | |
| — | Thickness Analysis | 10 | not started | |
| — | Undercut Analysis | 10 | not started | |
| — | Deviation Analysis | 10 | not started | |
| — | Symmetry Check | 10 | not started | |
| — | Geometry Analysis (short edges, sliver faces) | 10 | not started | |
| Model integrity check | Check entity | 2 | not started | Validates the kernel after rebuild and on import |
| — | Compare Documents, Compare Geometry | 10 | not started | |
| — | Feature Statistics | 6 | not started | Rebuild time per feature |
| Stress Analysis (linear static) | Simulation (SimulationXpress, static) | 11 | not started | |
| Modal analysis | Frequency study | 11 | not started | |
| Parametric study | Design Study | 11 | not started | |
| Contacts | Contact sets | 11 | not started | |
| Mesh controls, convergence | Mesh controls, adaptive meshing | 11 | not started | |
| — | Thermal, buckling, fatigue, drop test studies | 11 | not started | *(low priority)* |
| Dynamic Simulation | Motion Analysis | 11 | not started | |
| Generative Shape Design | Topology Study | 11 | not started | *(low priority)* |
| — | Flow Simulation (CFD) | 11 | not started | *(low priority)* |
| — | Costing | 11 | not started | Manufacturing cost estimates |
| — | Sustainability | 11 | not started | *(low priority)* |
| — | DFMXpress | 11 | not started | Design-for-manufacture checks |

## Tube, pipe and electrical routing

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Rigid piping | Routing: pipes | 11 | not started | |
| Bent tubing | Routing: tubes | 11 | not started | |
| Flexible hose | Routing: flexible tubes | 11 | not started | |
| Fittings, connections | Routing library components, connection points | 11 | not started | |
| Pipe styles | Routing templates | 11 | not started | |
| ISOGEN output | Piping isometrics | 11 | not started | *(low priority)* |
| Wires, cables, segments | Electrical routing, wires, cables | 11 | not started | |
| Ribbon cables | Ribbon cables | 11 | not started | |
| Electrical part authoring | Routing Library Manager | 11 | not started | |
| Nailboard drawings | Flattened route drawings | 11 | not started | |
| Harness reports, wire list import | From-to lists | 11 | not started | |

## File I/O and interoperability

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Native part format | Native part format | 1 | done | `.smp`, see [smp-format.md](smp-format.md) |
| STEP export: geometry | STEP export | 1 | in progress | AP214 schema via truck |
| STEP export: AP242 | STEP AP242 | 2 | not started | |
| STEP export: colours, names | STEP with appearances | 6 | not started | |
| STEP export: assemblies | STEP assembly structure | 7 | not started | |
| STEP export: PMI | STEP 242 with 3D PMI | 8 | not started | |
| STEP import: parts | STEP import | 2 | not started | |
| STEP import: assemblies | STEP import (assembly) | 7 | not started | |
| AnyCAD reference (linked import) | 3D Interconnect | 7 | not started | Updates when the source file changes |
| IGES | IGES | 10 | not started | |
| Parasolid (x_t) | Parasolid (x_t) | 10 | not started | Licensed format; check feasibility |
| ACIS (sat) | ACIS (sat) | 10 | not started | |
| JT | JT | 11 | not started | |
| STL | STL | 6 | not started | 3D printing |
| OBJ | — | 6 | not started | |
| 3MF | 3MF | 6 | not started | |
| DWG/DXF import | DXF/DWG import | 6 | not started | |
| PDF, 3D PDF | PDF, 3D PDF | 8 | not started | |
| Open other CAD (SolidWorks, Creo, NX, CATIA) | Open other CAD (Inventor, Creo, NX, CATIA) | 11 | not started | Proprietary; via STEP in practice |
| Revit (RFA), IFC export | IFC export | 11 | not started | *(low priority)* |
| FBX, glTF | glTF, FBX (via Visualize) | 11 | not started | |
| Point cloud (RCP, E57) | ScanTo3D | 11 | not started | *(low priority)* |
| Image export | Save as image | 4 | not started | |
| Templates | Templates | 6 | not started | |
| Batch migration | Task Scheduler conversion | 11 | not started | |
| — | — | 1 | in progress | Command-line rebuild and export (`smcli`); neither product has one |

## UI, viewport and navigation

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Application window | Application window | 4 | not started | Wayland-native (winit + wgpu) |
| — | — | 4 | not started | Follows the OS and Omarchy theme live; neither product has this |
| Ribbon | CommandManager | 4 | not started | |
| Model browser search and filter | Design tree filter | 4 | not started | |
| Property panels | PropertyManager | 4 | not started | |
| Mini-toolbar, in-canvas manipulators | Instant3D handles, triad | 4 | not started | |
| Marking Menu | Mouse Gestures, Shortcut Bar | 4 | not started | |
| — | Breadcrumbs, context toolbars | 4 | not started | |
| Command aliases (E, R, H, F, D, L…) | Keyboard shortcuts | 4 | not started | Default to Inventor's aliases |
| Customize shortcuts and ribbon | Customize | 4 | not started | |
| ViewCube | View Selector cube | 4 | not started | |
| Navigation bar (pan, zoom, orbit, look at) | Heads-up View Toolbar | 4 | not started | |
| Mouse presets | Mouse settings | 4 | not started | |
| 3Dconnexion SpaceMouse | 3Dconnexion SpaceMouse | 4 | not started | libspnav on Linux |
| Home View, previous/next | Previous View | 4 | not started | |
| Orthographic / perspective | Perspective | 4 | not started | |
| Visual styles | Display styles | 4 | not started | Shaded, edges, wireframe, hidden lines |
| Shadows, reflections, ambient occlusion | RealView, ambient occlusion | 4 | not started | |
| Ray tracing | PhotoView 360 / Visualize | 11 | not started | *(low priority)* |
| Selection priority, Select Other | Selection filters, Select Other | 4 | not started | |
| Window / crossing selection | Box / lasso selection | 4 | not started | |
| — | Magnifying Glass | 4 | not started | |
| Preselection highlight | Dynamic highlight | 4 | not started | |
| Heads-up display (input on canvas) | Instant3D, on-canvas dimension input | 4 | not started | |
| Grid, snap | Grid, snaps | 4 | not started | |
| Tabbed documents, multiple windows | Multiple windows, tabs | 4 | not started | |
| Application Options | System Options | 4 | not started | |
| Status bar, prompts | Status bar | 4 | not started | |
| Command search | Search commands | 4 | not started | |
| Home page with recent files | Welcome dialog | 4 | not started | |
| Command repeat | Repeat last command | 4 | not started | |
| Drag-and-drop file open | Drag-and-drop file open | 4 | not started | |
| — | Task Pane (design library, file explorer, appearances) | 4 | not started | |
| Large model graphics (LOD, culling) | Level of detail, graphics performance | 4 | not started | |
| — | — | 4 | not started | HiDPI and fractional scaling on Wayland |
| Localization | Localization | 11 | not started | *(low priority)* |
| Touch / tablet | Touch mode | 11 | not started | *(low priority)* |

## Data management and version control

This section follows Onshape, since that is the model to copy. Everything
here runs on local files: history and versions live on disk next to the
part, nothing needs a server or an account, and a document stays usable
offline forever. Git and git LFS are fine to build on, but the user should
never need to know git to branch or merge a design.

| Onshape | Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|---|
| Document (part studios, assemblies, drawings and files as tabs) | Project folder | — | 7 | not started | One local folder or container per design |
| No Save button: every change persisted | — | Auto-recover | 5 | not started | Write each feature edit to disk as it happens |
| History panel (every change, restorable) | OldVersions folder | Backup copies | 5 | not started | Full edit history per document, never lost |
| Restore to any point in history | Restore old version | Restore backup | 5 | not started | |
| Versions (named, immutable snapshots) | Vault versions | PDM versions | 5 | not started | |
| Branches (workspaces) | — | — | 5 | not started | Try design alternatives without copying files |
| Merge branches | — | — | 5 | not started | Feature-tree merge; needs stable topological naming (step 2) |
| Merge conflict resolution on features and parameters | — | — | 5 | not started | |
| Version graph view | — | — | 5 | not started | |
| Compare (feature tree, parameters and geometry between versions) | — | Compare Documents, Compare Geometry | 5 | not started | `.smp` feature tree as diffable text |
| Undo across sessions via history | — | — | 5 | not started | |
| Edit attribution (who changed what, when) | Vault history | PDM history | 5 | not started | Author from local identity (e.g. git config) |
| References pinned to a version of another document | — | — | 7 | not started | Derive or place at a version, not "latest file on disk" |
| Update to latest version notification | Out-of-date references | Update references | 7 | not started | |
| Linked documents, where used | Where Used | Where Used | 7 | not started | |
| Rename and move without breaking references | Design Assistant | Rename with references (SolidWorks Explorer) | 7 | not started | Stable IDs instead of paths |
| Copy / make a copy of a document | Copy Design | Pack and Go with renaming | 7 | not started | |
| — | Project file (.ipj): workspace, libraries, search paths | File Locations, reference search paths | 7 | not started | Local documents should mostly remove the need |
| Release management (release candidates, approval) | Vault lifecycle | PDM workflow | 7 | not started | Local approval; single user by default |
| Release states (pending, released, obsolete) | Lifecycle states | Workflow states | 7 | not started | |
| Revision scheme (A, B, C…) and revision history | Revisions | Revisions | 7 | not started | |
| Part numbering scheme | Vault numbering | PDM serial numbers | 7 | not started | |
| Folders, labels, search across documents | Vault search | PDM search | 7 | not started | Local index of all documents |
| Thumbnails in document browser | Thumbnails | Thumbnails | 4 | not started | freedesktop thumbnailer on Linux |
| Comments and action items on documents | — | — | 11 | not started | *(low priority)* |
| Export / archive a document with its history | Pack and Go | Pack and Go | 7 | not started | Single file to hand to someone else |
| Sync and backup to another machine | — | — | 11 | not started | Local-first: git remote or folder sync, never required |
| Real-time multi-user editing | — | — | 11 | not started | *(low priority)*: out of scope while single-user and local |
| Sharing and permissions | Vault permissions | PDM permissions | 11 | not started | *(low priority)*: only matters for shared drives or LANs |

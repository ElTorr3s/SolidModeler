# Feature parity: Inventor and SolidWorks

This is the checklist for the long-term goal in the [roadmap](roadmap.md):
feature parity with Autodesk Inventor, plus the SolidWorks features worth
having. It is grouped by environment and then by tool group, the way both
products lay out their commands.

Each feature appears once. When both products have it under different names,
the row gives both. A `—` in a column means that product has no equivalent,
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
| 5 | *Proposed:* rest of part modeling (work features, patterns, sweeps, lofts, shell, properties, materials) |
| 6 | *Proposed:* assemblies |
| 7 | *Proposed:* drawings and model-based definition |
| 8 | *Proposed:* sheet metal |
| 9 | *Proposed:* advanced modeling: surfacing, freeform, plastic part, rules and automation, table-driven families, weldments and frames |
| 10 | *Proposed:* add-ins and analysis: standard parts libraries, design accelerators, simulation, routing, presentations |

Steps 5 to 10 are a suggested order only; the roadmap itself lists steps 1
to 4.

**Status** is one of:

- `not started`
- `in progress`: some of it works (the notes say what)
- `done`: matches the reference behaviour for normal professional use

*(low priority)* marks items that are rarely used in practice or tied to the
vendor's cloud services. They stay listed so nothing gets forgotten.

## Summary

<!-- summary:start -->
| Environment | Items | In both | Inventor only | SolidWorks only | Ours only | Done | In progress |
|---|---|---|---|---|---|---|---|
| Core document and history | 25 | 21 | 0 | 4 | 0 | 0 | 5 |
| Sketch (2D) | 69 | 59 | 2 | 8 | 0 | 0 | 1 |
| 3D sketch and curves | 15 | 13 | 0 | 2 | 0 | 0 | 0 |
| Part modeling | 86 | 61 | 4 | 21 | 0 | 0 | 1 |
| Parameters, properties and automation | 23 | 17 | 5 | 1 | 0 | 0 | 1 |
| Sheet metal | 31 | 22 | 2 | 7 | 0 | 0 | 0 |
| Surfacing and freeform | 24 | 11 | 7 | 6 | 0 | 0 | 0 |
| Plastic part and mold | 10 | 7 | 3 | 0 | 0 | 0 | 0 |
| Assemblies | 68 | 49 | 4 | 15 | 0 | 0 | 0 |
| Weldments and frames | 17 | 13 | 2 | 2 | 0 | 0 | 0 |
| Standard parts and design accelerators | 16 | 11 | 4 | 1 | 0 | 0 | 0 |
| Drawings | 67 | 56 | 2 | 9 | 0 | 0 | 0 |
| Model-based definition | 4 | 3 | 0 | 1 | 0 | 0 | 0 |
| Presentations and animation | 8 | 7 | 1 | 0 | 0 | 0 | 0 |
| Inspection, analysis and simulation | 25 | 11 | 2 | 12 | 0 | 0 | 0 |
| Tube, pipe and electrical routing | 11 | 11 | 0 | 0 | 0 | 0 | 0 |
| File I/O and interoperability | 30 | 28 | 1 | 0 | 1 | 1 | 2 |
| UI, viewport and navigation | 37 | 32 | 0 | 3 | 2 | 0 | 0 |
| Data management and projects | 8 | 5 | 1 | 1 | 1 | 0 | 0 |
| **Total** | **574** | **437** | **40** | **93** | **4** | **1** | **10** |
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
| — | FeatureXpert | 5 | not started | Auto-fixes failing fillets and drafts |
| — | Freeze bar | 5 | not started | Locks features above the bar from rebuilding |
| Stable topological naming | Stable face/edge IDs | 2 | not started | Not a command; needed so references survive edits |
| Parent/child relationships | Parent/Child | 3 | not started | |
| Browser folders | Folders | 5 | not started | |
| Rename nodes | Rename | 3 | not started | |
| — | Comments on features | 5 | not started | |
| Undo / redo | Undo / redo | 4 | not started | |
| Solid Bodies folder (multi-body) | Solid Bodies folder (multi-body) | 5 | not started | |
| Surface Bodies folder | Surface Bodies folder | 9 | not started | |
| Visibility per feature/body/work feature | Hide/show | 4 | not started | |
| Model States | Configurations | 9 | not started | Variations of one part in one file |
| Design Views | Display States, Named Views | 5 | not started | Saved camera, visibility, appearance |
| Document Settings | Document Properties | 3 | not started | Length units only so far |
| — | Sensors | 9 | not started | Alerts when mass, dimension, interference cross limits |

## Sketch (2D)

### Create

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Start 2D Sketch on plane | Sketch on plane | 1 | in progress | Origin planes with offset; closed polylines only |
| Sketch on face / work plane | Sketch on face / plane | 3 | not started | |
| Line (with arc gesture) | Line, Centerline, Midpoint Line | 3 | not started | |
| Spline: control vertex | Style Spline | 3 | not started | |
| Spline: interpolation | Spline | 3 | not started | |
| — | Spline on Surface | 9 | not started | |
| Equation Curve | Equation Driven Curve | 5 | not started | |
| Bridge Curve | — | 5 | not started | |
| — | Conic | 5 | not started | |
| — | Parabola | 5 | not started | |
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
| Text, Geometry Text | Text (on curve) | 5 | not started | |
| Construction, Centerline style | For construction | 3 | not started | |
| Project Geometry | Convert Entities | 3 | not started | |
| Project Cut Edges | Intersection Curve | 3 | not started | |
| Import AutoCAD into sketch | DXF/DWG import to sketch | 5 | not started | |
| Insert Image | Sketch Picture | 5 | not started | |
| Insert points from Excel | Curve Through XYZ Points | 5 | not started | *(low priority)* |
| — | Autotrace (picture to sketch) | 9 | not started | *(low priority)* |
| — | Sketch Ink (pen input) | 10 | not started | *(low priority)* |

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
| — | Merge, Pierce | 5 | not started | Pierce ties a sketch point to a 3D curve |
| Constraint inference | Automatic relations | 3 | not started | |
| Show / Delete Constraints | Display/Delete Relations | 3 | not started | |
| Relax Mode | Drag with SketchXpert | 3 | not started | |
| Degrees of freedom, fully constrained colour | Under/fully/over defined colours | 3 | not started | |
| Over-constrained prompt | SketchXpert | 3 | not started | |

### Blocks and tools

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Sketch Blocks | Blocks | 9 | not started | Skeleton and layout workflows |
| Share Sketch | Shared sketch | 5 | not started | |
| Sketch Doctor | Check Sketch for Feature, Repair Sketch | 3 | not started | |
| Profile selection with nested loops | Contour Select | 3 | not started | |
| Slice Graphics | — | 4 | not started | |
| Layout part | Layout sketch | 6 | not started | |

## 3D sketch and curves

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| 3D Sketch | 3D Sketch | 5 | not started | |
| 3D Line with auto-bend | 3D Line | 5 | not started | |
| 3D spline | 3D Spline | 5 | not started | |
| Helical Curve | Helix and Spiral | 5 | not started | |
| Equation Curve (3D) | Equation Driven Curve (3D) | 5 | not started | |
| Bend | Sketch Fillet (3D) | 5 | not started | |
| Project to Surface | Projected Curve | 9 | not started | |
| Intersection Curve | Intersection Curve | 5 | not started | |
| Silhouette Curve | Silhouette Entities | 9 | not started | |
| Include Geometry | Convert Entities (3D) | 5 | not started | |
| 3D constraints and dimensions | 3D relations and dimensions | 5 | not started | |
| — | Composite Curve | 5 | not started | |
| — | Split Line | 5 | not started | Splits faces by a curve |
| Curve on face | Face Curves | 9 | not started | |
| 3D Transform | Triad move | 5 | not started | |

## Part modeling

### Create

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Extrude: distance, symmetric | Extruded Boss: Blind, Mid Plane | 1 | in progress | New solid only |
| Extrude: join / cut / intersect / new solid | Extruded Boss / Cut, Merge result | 2 | not started | Needs booleans |
| Extrude: two directions, asymmetric | Direction 1 / Direction 2 | 3 | not started | |
| Extrude: to, to next, through all, between | Up to Vertex/Surface/Body, Up to Next, Through All | 3 | not started | |
| — | Offset from Surface end condition, Thin Feature | 5 | not started | |
| Extrude taper | Draft while extruding | 5 | not started | |
| Revolve | Revolved Boss / Cut | 3 | not started | |
| Sweep (path, guide rail, guide surface) | Swept Boss / Cut (guide curves) | 5 | not started | |
| Sweep taper and twist | Sweep twist along path | 5 | not started | |
| — | Swept Cut with solid tool body | 9 | not started | Machining tool paths |
| Loft (rails, center line, area loft) | Lofted Boss / Cut | 5 | not started | |
| Loft point mapping, conditions | Connectors, start/end constraints | 5 | not started | |
| — | Boundary Boss / Cut | 9 | not started | |
| Coil | — | 5 | not started | SolidWorks sweeps along a helix instead |
| Rib | Rib | 5 | not started | |
| Emboss | Wrap (emboss, deboss, scribe) | 5 | not started | |
| Decal | Decals | 9 | not started | |
| Derive | Insert Part, Mirror Part | 6 | not started | Link-in with scale, mirror, body selection |
| Import (base feature) | Imported feature | 2 | not started | See File I/O |
| — | FeatureWorks (feature recognition on imports) | 10 | not started | |
| Primitives (box, cylinder, sphere, torus) | — | 5 | not started | |
| — | Dome | 5 | not started | |

### Modify

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Hole (simple, counterbore, countersink, spotface) | Hole Wizard | 3 | not started | |
| Hole: tapped, taper tapped, clearance | Hole Wizard: tap, pipe tap, legacy | 3 | not started | Thread data tables |
| Hole placement | Hole Wizard positions | 3 | not started | |
| — | Advanced Hole (stacked near/far elements) | 5 | not started | |
| — | Hole Series (across assembly parts) | 6 | not started | |
| Fillet: constant | Fillet: constant size | 3 | not started | |
| Fillet: variable | Fillet: variable size | 5 | not started | |
| Fillet: setbacks | Fillet: setback | 5 | not started | |
| Fillet: face | Fillet: face | 5 | not started | |
| Fillet: full round | Fillet: full round | 5 | not started | |
| Fillet: G2 / inverted conic | Fillet: curvature continuous, conic | 5 | not started | |
| — | FilletXpert | 5 | not started | |
| Chamfer (distance, angle, two distances) | Chamfer (angle-distance, distance-distance, vertex) | 3 | not started | |
| Partial chamfer | Offset face, face-face chamfer | 5 | not started | |
| Shell | Shell | 5 | not started | |
| Draft (fixed edge, fixed plane, parting line) | Draft (neutral plane, parting line, step draft) | 5 | not started | |
| — | DraftXpert | 5 | not started | |
| Thread | Cosmetic Thread, Thread | 5 | not started | |
| — | Stud Wizard | 5 | not started | |
| Split (face, trim solid, split solid) | Split, Split Line | 5 | not started | |
| Combine | Combine | 5 | not started | |
| — | Intersect (regions from bodies and surfaces) | 9 | not started | |
| Thicken/Offset | Thicken | 9 | not started | |
| Direct Edit (move, resize, rotate, scale) | Move Face, Instant3D | 9 | not started | |
| — | Scale (feature) | 5 | not started | Inventor scales via Derive or Direct Edit |
| Delete Face | Delete Face (delete and patch/fill) | 9 | not started | |
| Replace Face | Replace Face | 9 | not started | |
| Copy Object | Move/Copy Bodies (copy) | 9 | not started | |
| Move Bodies | Move/Copy Bodies | 5 | not started | |
| — | Delete/Keep Body | 5 | not started | |
| Bend Part | Flex (bending, twisting, tapering, stretching) | 9 | not started | |
| — | Deform | 9 | not started | |
| — | Indent | 9 | not started | |
| — | Cavity | 10 | not started | Mold cavities from a part |
| Simplify | Defeature | 6 | not started | |
| Make Components | Save Bodies | 6 | not started | Multi-body master workflow |
| iFeatures | Library Features | 9 | not started | |

### Work features (reference geometry)

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Work Plane (all 12 methods) | Plane (all constraint combinations) | 5 | not started | Offset, angle, midplane, three points, tangent, normal to curve |
| Work Axis (all methods) | Axis | 5 | not started | Edge, cylinder, two points, two planes, normal through point |
| Work Point (all methods) | Point | 5 | not started | |
| Grounded Work Point | — | 5 | not started | |
| UCS | Coordinate System | 5 | not started | |
| — | Center of Mass (reference point) | 5 | not started | |
| — | Bounding Box | 5 | not started | Updates with the part; used for stock size |
| iMates | Mate References | 6 | not started | |

### Pattern

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Rectangular Pattern | Linear Pattern | 5 | not started | |
| Circular Pattern | Circular Pattern | 5 | not started | |
| Sketch Driven Pattern | Sketch Driven Pattern | 5 | not started | |
| Pattern along path | Curve Driven Pattern | 5 | not started | |
| Mirror | Mirror | 5 | not started | Features and bodies |
| — | Table Driven Pattern | 5 | not started | |
| — | Fill Pattern | 5 | not started | Perforation layouts |
| — | Variable Pattern | 9 | not started | Each instance gets its own dimension values |
| Pattern options: suppress occurrences, adjust | Instances to skip, vary sketch | 5 | not started | |

### Part-level tools

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Material | Edit Material | 5 | not started | Density drives mass properties |
| Appearance | Appearances | 4 | not started | Per part, body, feature, face |
| Physical properties | Mass Properties | 5 | not started | Mass, volume, area, COG, inertia |
| Region Properties | Section Properties | 3 | not started | |
| Measure | Measure | 4 | not started | Distance, angle, area, loop, minimum distance |
| Section views | Section View | 4 | not started | |
| Make Part | Insert into New Part | 6 | not started | |
| Mark | — | 9 | not started | *(low priority)* |
| Point cloud attach | ScanTo3D, point cloud | 10 | not started | *(low priority)* |
| Mesh feature, mesh to B-rep | Mesh BREP bodies, convert to body | 9 | not started | |

## Parameters, properties and automation

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| User Parameters | Global Variables | 1 | in progress | Name, value, comment stored; nothing references them yet |
| Model parameters (d0, d1…) | Dimension names (D1@Sketch1) | 3 | not started | |
| Equations in parameters, units | Equations | 3 | not started | |
| Text and True/False parameters | — | 9 | not started | |
| Multi-value lists | — | 9 | not started | |
| Key / Export parameters | Link to custom property | 5 | not started | |
| Parameter tolerances | Dimension tolerances | 7 | not started | |
| Linked parameters (Excel) | Linked values, external equations file | 9 | not started | |
| iProperties | Custom Properties, Summary Information | 5 | not started | Feed title blocks and parts lists |
| — | Property Tab Builder | 9 | not started | Custom property entry forms |
| Material and appearance libraries | Material database, appearance library | 5 | not started | |
| Style and Standard Editor | Drafting standard (document properties) | 7 | not started | |
| iLogic rules | — | 9 | not started | Scripted rules; pick a language such as Rhai or Lua |
| iLogic event triggers | — | 9 | not started | |
| iLogic forms | DriveWorksXpress | 9 | not started | |
| iLogic external rules | — | 9 | not started | |
| iLogic component and feature control | Configuration-specific suppression | 9 | not started | |
| iParts | Design Tables (part configurations) | 9 | not started | |
| iAssemblies | Design Tables (assembly configurations) | 9 | not started | |
| iFeature tables | Library feature configurations | 9 | not started | |
| Inventor API | SolidWorks API | 9 | not started | Rust API plus scripting bindings |
| VBA macros | VBA macros, macro recorder | 10 | not started | *(low priority)* |
| Add-ins | Add-ins | 10 | not started | |
| Task Scheduler | Task Scheduler | 10 | not started | Batch jobs |

## Sheet metal

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Sheet Metal Defaults, Styles / Rules | Sheet Metal parameters, Gauge Tables | 8 | not started | Thickness, bend radius, relief, K-factor |
| Unfold rules (K-factor, bend table, equation) | Bend Allowance, Bend Deduction, K-factor, Bend Table | 8 | not started | |
| Face | Base Flange/Tab | 8 | not started | |
| Contour Flange | Base Flange (open sketch), Miter Flange | 8 | not started | |
| Contour Roll | — | 8 | not started | |
| Lofted Flange | Lofted Bend | 8 | not started | |
| Flange | Edge Flange | 8 | not started | |
| Hem | Hem | 8 | not started | |
| Fold | Sketched Bend | 8 | not started | |
| Bend | — | 8 | not started | Bend between two faces |
| — | Jog | 8 | not started | |
| — | Cross Break | 8 | not started | |
| — | Swept Flange | 8 | not started | |
| Corner Seam | Closed Corner | 8 | not started | |
| — | Welded Corner | 8 | not started | |
| Corner Round, Corner Chamfer | Break Corner / Corner Trim | 8 | not started | |
| Corner relief options | Corner Relief | 8 | not started | |
| Cut (across bend) | Extruded Cut (normal cut), Unfold/Fold cut | 8 | not started | |
| Punch Tool | Forming Tools | 8 | not started | |
| — | Sheet Metal Vent | 8 | not started | |
| — | Tab and Slot | 8 | not started | |
| — | Sheet Metal Gusset | 8 | not started | |
| Rip | Rip | 8 | not started | |
| Unfold / Refold | Unfold / Fold | 8 | not started | |
| Convert to Sheet Metal | Convert to Sheet Metal, Insert Bends | 8 | not started | |
| Flat Pattern | Flatten | 8 | not started | |
| Flat pattern extents, bend order | Sheet metal bounding box, bend notes | 8 | not started | |
| Multi-body sheet metal | Multi-body sheet metal | 8 | not started | |
| A-side definition | Fixed face | 8 | not started | |
| Cosmetic centerlines, bend lines | Bend lines | 8 | not started | |
| Export flat pattern (DXF/DWG) | Export to DXF/DWG | 8 | not started | Layer mapping, bend lines |

## Surfacing and freeform

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Extrude / Revolve / Sweep / Loft (surface output) | Extruded / Revolved / Swept / Lofted Surface | 9 | not started | |
| — | Boundary Surface | 9 | not started | |
| Patch (boundary patch) | Filled Surface | 9 | not started | G0/G1/G2 conditions |
| — | Planar Surface | 9 | not started | |
| Stitch | Knit Surface | 9 | not started | |
| Sculpt | Thicken (cut) | 9 | not started | |
| Trim Surface | Trim Surface | 9 | not started | |
| Extend Surface | Extend Surface | 9 | not started | |
| — | Untrim Surface | 9 | not started | |
| Ruled Surface | Ruled Surface | 9 | not started | |
| Thicken/Offset (surface) | Offset Surface | 9 | not started | |
| — | Radiate Surface | 9 | not started | |
| — | Mid-Surface | 9 | not started | |
| Unwrap | Surface Flatten | 9 | not started | |
| Split with surface | Cut With Surface | 9 | not started | |
| Repair Bodies | Import Diagnostics | 9 | not started | Cleans up bad imports |
| Fit Mesh Face | — | 9 | not started | |
| Freeform: box, plane, cylinder, sphere, torus, quadball | — | 9 | not started | T-spline modeling |
| Freeform: face, extrude, revolve, sweep, loft | — | 9 | not started | |
| Freeform: edit form, insert edge/point, subdivide | — | 9 | not started | |
| Freeform: bridge, merge, weld/unweld, crease | — | 9 | not started | |
| Freeform: thicken, match edge, align, flatten, symmetry | — | 9 | not started | |
| Freeform: convert to and from B-rep | — | 9 | not started | |
| — | Freeform (push/pull face deformation) | 9 | not started | Not T-splines; deforms an existing face |

## Plastic part and mold

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Grill | Vent | 9 | not started | |
| Boss | Mounting Boss | 9 | not started | |
| Rest | — | 9 | not started | |
| Snap Fit | Snap Hook, Snap Hook Groove | 9 | not started | |
| Rule Fillet | — | 9 | not started | |
| Lip | Lip/Groove | 9 | not started | |
| Shrinkage allowance | Scale (mold) | 9 | not started | |
| Mold Design (core/cavity, parting surfaces) | Mold Tools (parting lines, shut-off surfaces, parting surface, tooling split, core) | 10 | not started | *(low priority)* |
| Mold base, runners, gates | — | 10 | not started | *(low priority)* |
| Moldflow fill analysis | SolidWorks Plastics | 10 | not started | *(low priority)* |

## Assemblies

### Components

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Assembly (.iam) | Assembly (.sldasm) | 6 | not started | Extend `.smp` or add an assembly format |
| Place Component | Insert Components | 6 | not started | |
| Place imported CAD | Insert STEP component | 6 | not started | |
| Create Component (in place) | New Part (in context) | 6 | not started | |
| Ground | Fix / Float | 6 | not started | |
| Pattern Component | Linear, Circular, Pattern Driven, Sketch Driven, Curve Driven Component Pattern | 6 | not started | |
| — | Chain Component Pattern | 6 | not started | |
| Mirror Components | Mirror Components | 6 | not started | |
| Copy Components | Copy with Mates | 6 | not started | |
| Replace Component | Replace Components | 6 | not started | |
| Demote / Promote | Form New Subassembly / Dissolve | 6 | not started | |
| Flexible subassemblies | Flexible subassemblies | 6 | not started | |
| Edit in place | Edit Part (in context) | 6 | not started | |
| Adaptive parts and features | External references (in-context) | 6 | not started | |
| Virtual components | Virtual components | 6 | not started | |
| Reference components | Envelopes | 6 | not started | |
| Shrinkwrap | Defeature (assembly) | 6 | not started | |
| Substitutes (model states) | Simplified configurations | 9 | not started | |
| — | SpeedPak | 6 | not started | Lightweight subassembly representation |
| Skeletal modeling | Top-down layout / master model | 6 | not started | |
| — | Treehouse (structure planning) | 10 | not started | *(low priority)* |

### Constraints and joints (mates)

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Assembly constraint solver | Mate solver | 6 | not started | |
| Mate / Flush | Coincident, Parallel, Distance | 6 | not started | |
| Angle | Angle | 6 | not started | |
| — | Perpendicular | 6 | not started | Inventor uses a 90° angle |
| Tangent | Tangent | 6 | not started | |
| Insert | Concentric + Coincident | 6 | not started | |
| — | Lock | 6 | not started | |
| Symmetry | Symmetric | 6 | not started | |
| — | Width | 6 | not started | |
| — | Profile Center | 6 | not started | |
| — | Path Mate | 6 | not started | |
| — | Linear/Linear Coupler | 6 | not started | |
| Rotation motion | Gear | 6 | not started | |
| Rotation-Translation | Rack and Pinion, Screw | 6 | not started | |
| Transitional | Cam | 6 | not started | |
| — | Hinge, Slot, Universal Joint | 6 | not started | |
| Constraint Set (UCS to UCS) | — | 6 | not started | |
| Constraint limits | Limit Distance / Limit Angle | 6 | not started | |
| Joints: rigid, rotational, slider, cylindrical, planar, ball | — | 6 | not started | |
| Composite iMates | — | 6 | not started | |
| Alt-drag constraint | SmartMates | 6 | not started | |
| Assemble command | Mate while inserting | 6 | not started | |
| Drive constraint | Motion Study (motor) | 6 | not started | With collision detection |
| Contact Solver | Physical Dynamics | 6 | not started | |
| Sick constraints, isolate errors | MateXpert, mate errors | 6 | not started | |
| Degrees of freedom display | Under-defined markers | 6 | not started | |
| Free Move / Free Rotate | Move / Rotate Component | 6 | not started | |
| Positional Representations | Mate Controller | 6 | not started | |

### Productivity and inspection

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Analyze Interference | Interference Detection | 6 | not started | |
| — | Clearance Verification | 6 | not started | |
| — | Hole Alignment | 6 | not started | |
| Bill of Materials | Bill of Materials (from drawings or assembly) | 6 | not started | Structured, parts only |
| BOM structure: phantom, reference, purchased, inseparable | Child display, exclude from BOM | 6 | not started | |
| Design Views (assembly) | Display States | 6 | not started | |
| Selection filters, select by size, internal | Select Internal, Select Toolbox, Advanced Select | 6 | not started | |
| Isolate | Isolate | 6 | not started | |
| Express Mode | Large Assembly Mode, Large Design Review, Lightweight | 6 | not started | |
| Assembly features (cut, hole, fillet, chamfer, sweep, revolve) | Assembly features | 6 | not started | |
| Weld preparation | — | 9 | not started | |
| Center of gravity display | Center of Mass | 6 | not started | |
| Component suppression | Suppress components | 6 | not started | |
| Copy Design | Pack and Go with renaming | 6 | not started | |
| Resolve Link | Find references | 6 | not started | |
| — | Assembly Visualization | 6 | not started | Sort and colour components by property |
| — | AssemblyXpert | 6 | not started | Performance report |
| — | Performance Evaluation | 6 | not started | Rebuild times per feature/component |
| Exploded views (in Presentations) | Exploded View (in assembly), Explode Line Sketch | 6 | not started | |

## Weldments and frames

Inventor builds these in assemblies; SolidWorks builds them as multi-body
parts. Same tools, different document model.

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Weldment assembly | Weldment (part) | 9 | not started | |
| Preparations | — | 9 | not started | |
| Cosmetic Weld | Weld Bead (cosmetic) | 9 | not started | |
| Fillet Weld | Fillet Bead | 9 | not started | |
| Groove Weld | Weld Bead | 9 | not started | |
| Weld symbols (3D) | Weld symbols | 9 | not started | |
| Machining features | Post-weld features | 9 | not started | |
| Frame Generator: Insert Frame | Structural Member | 9 | not started | |
| Frame member profiles | Weldment profiles library | 9 | not started | |
| Change frame member | Edit structural member | 9 | not started | |
| Miter, Notch, Trim to Frame, Trim/Extend | Trim/Extend, corner treatments | 9 | not started | |
| Lengthen / Shorten | Structural member offsets | 9 | not started | |
| — | End Cap | 9 | not started | |
| — | Gusset | 9 | not started | |
| Frame member info, cut lists | Cut List, cut list properties | 9 | not started | |
| Weld calculator | — | 10 | not started | *(low priority)* |
| Beam and Column Calculator, Frame Analysis | Beam elements in Simulation | 10 | not started | *(low priority)* |

## Standard parts and design accelerators

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Content Center (ISO, DIN, ANSI, JIS, GB…) | Toolbox | 10 | not started | Fasteners, profiles, bearings |
| Content Center Editor | Toolbox Settings | 10 | not started | |
| Bolted Connection | Smart Fasteners | 10 | not started | |
| — | Design Library (drag-and-drop parts, features, annotations) | 9 | not started | |
| Shaft generator | — | 10 | not started | |
| Spur, bevel, worm gears | Toolbox gears (static) | 10 | not started | Inventor generates and calculates |
| Bearing generator and calculator | Toolbox bearings, bearing calculator | 10 | not started | |
| V-belts, synchronous belts | Belt/Chain | 10 | not started | |
| Roller chains | Belt/Chain | 10 | not started | |
| Cam generators | — | 10 | not started | |
| Keys, splines | Toolbox keys | 10 | not started | |
| Clevis, cotter, safety pins | Toolbox pins | 10 | not started | |
| O-rings | Toolbox O-rings | 10 | not started | |
| Spring generators | — | 10 | not started | |
| Machinery's Handbook calculators | Toolbox calculators (beam, bearing) | 10 | not started | Tolerances, fits, screws |
| Joint and brake calculators, power screw | — | 10 | not started | *(low priority)* |

## Drawings

### Views

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Drawing (.idw / .dwg) | Drawing (.slddrw) | 7 | not started | |
| Sheets, borders, title blocks | Sheets, Sheet Formats | 7 | not started | |
| Base View | Model View, Standard 3 View | 7 | not started | |
| Projected View | Projected View | 7 | not started | First and third angle |
| Auxiliary View | Auxiliary View | 7 | not started | |
| Section View (full, half, offset, aligned) | Section View, Aligned Section | 7 | not started | |
| — | Removed Section | 7 | not started | |
| Detail View | Detail View | 7 | not started | |
| Overlay View | Alternate Position View | 7 | not started | |
| Draft View | Empty View | 7 | not started | |
| — | Relative View | 7 | not started | Orients a view by two faces |
| — | Predefined View | 7 | not started | |
| Break | Break View | 7 | not started | |
| Break Out | Broken-out Section | 7 | not started | |
| Slice | — | 7 | not started | |
| Crop | Crop View | 7 | not started | |
| — | 3D Drawing View | 7 | not started | |
| View alignment | Alignment | 7 | not started | |
| View styles: hidden, shaded, raster/precise | Display style, draft quality | 7 | not started | |
| Model state / design view in view | Configuration / display state in view | 7 | not started | |
| Hatching | Area Hatch/Fill | 7 | not started | Per material, per part |
| Exclude components from section | Section scope | 7 | not started | |
| Sketches on sheets and views | Sketch in drawing | 7 | not started | |
| Flat pattern view | Flat pattern view | 8 | not started | |
| — | Detailing Mode | 7 | not started | Opens large drawings without loading models |

### Annotate

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Dimension | Smart Dimension | 7 | not started | |
| Baseline, Ordinate, Chain dimension sets | Baseline, Ordinate, Chain Dimension | 7 | not started | |
| Retrieve Dimensions | Model Items | 7 | not started | |
| Arrange Dimensions | Auto Arrange Dimensions | 7 | not started | |
| — | Autodimension | 7 | not started | |
| Tolerances and fits | Tolerance/Precision, fits | 7 | not started | |
| Hole and Thread Notes | Hole Callout | 7 | not started | |
| Chamfer Note | Chamfer Dimension | 7 | not started | |
| Bend notes, punch notes | Bend notes | 8 | not started | |
| Center Mark, Centerline, Bisector, Centered Pattern | Center Mark, Centerline | 7 | not started | |
| Automated Centerlines | Auto insert center marks | 7 | not started | |
| Surface Texture | Surface Finish | 7 | not started | |
| Welding symbols, caterpillar, end fill | Weld Symbol, weld beads | 9 | not started | |
| Feature Control Frame | Geometric Tolerance | 7 | not started | |
| Datum Identifier, Datum Target | Datum Feature, Datum Target | 7 | not started | |
| Feature Identifier | — | 7 | not started | |
| Sketched Symbols | Blocks | 7 | not started | |
| Text, Leader Text | Note | 7 | not started | |
| Revision Cloud | Revision Cloud | 7 | not started | |
| Balloon, Auto Balloon | Balloon, Auto Balloon, Stacked Balloon | 7 | not started | |
| — | Magnetic Line | 7 | not started | |
| Parts List | Bill of Materials table | 7 | not started | |
| Revision Table, revision tags | Revision Table | 7 | not started | |
| General Table | General Table | 7 | not started | |
| Hole Table | Hole Table | 7 | not started | |
| Bend Table, Punch Table | Bend Table, Punch Table | 8 | not started | |
| Weld bead table, cut list table | Weldment Cut List, Weld Table | 9 | not started | |
| Layers | Layers | 7 | not started | |
| Drafting standards (ANSI, ISO, DIN, JIS, GB, BSI) | Drafting standards | 7 | not started | |
| Prompted entries, property fields | Linked notes to properties | 7 | not started | |
| Drawing Resources | Sheet format, block libraries | 7 | not started | |
| Templates | Templates | 7 | not started | |
| Associative update, orphaned annotations | Dangling annotations | 7 | not started | |

### Output and management

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Print | Print | 7 | not started | |
| Export PDF | Save as PDF | 7 | not started | |
| Export DWG / DXF | Save as DWG / DXF | 7 | not started | |
| Export images | Save as image | 7 | not started | |
| Move / copy views between sheets | Move / copy views | 7 | not started | |
| Replace Model Reference | Replace Model | 7 | not started | |
| — | Design Checker | 7 | not started | Checks drawings against company standards |
| — | DrawCompare / Compare Documents | 7 | not started | |
| Shared Views | eDrawings, markup | 10 | not started | *(low priority)* |

## Model-based definition

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| 3D Annotations | DimXpert, MBD 3D PMI | 7 | not started | Goes out through STEP AP242 semantic PMI |
| Tolerance Advisor | DimXpert manager, auto-dimension scheme | 7 | not started | |
| Tolerance Analysis (stack-up) | TolAnalyst | 10 | not started | *(low priority)* |
| — | 3D PDF publishing (MBD) | 10 | not started | *(low priority)* |

## Presentations and animation

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Presentation (.ipn) | — | 10 | not started | SolidWorks does this in the assembly |
| Storyboards, timeline | Motion Study (Animation) | 10 | not started | |
| Tweak Components | Explode steps | 10 | not started | |
| Snapshot Views | Exploded view configurations | 10 | not started | |
| Auto Explode | Auto-space explode | 10 | not started | |
| Camera actions, opacity | Camera views, appearance keys | 10 | not started | |
| Publish video | Save animation (video) | 10 | not started | |
| Publish raster images | Save as image | 10 | not started | |

## Inspection, analysis and simulation

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Zebra Analysis | Zebra Stripes | 9 | not started | |
| Draft Analysis | Draft Analysis | 9 | not started | |
| Curvature analysis, curvature comb | Curvature, Curvature Combs | 9 | not started | |
| Surface Analysis (Gaussian) | — | 9 | not started | |
| Cross Section Analysis | — | 9 | not started | |
| — | Thickness Analysis | 9 | not started | |
| — | Undercut Analysis | 9 | not started | |
| — | Deviation Analysis | 9 | not started | |
| — | Symmetry Check | 9 | not started | |
| — | Geometry Analysis (short edges, sliver faces) | 9 | not started | |
| Model integrity check | Check entity | 2 | not started | Validates the kernel after rebuild and on import |
| — | Compare Documents, Compare Geometry | 9 | not started | |
| — | Feature Statistics | 5 | not started | Rebuild time per feature |
| Stress Analysis (linear static) | Simulation (SimulationXpress, static) | 10 | not started | |
| Modal analysis | Frequency study | 10 | not started | |
| Parametric study | Design Study | 10 | not started | |
| Contacts | Contact sets | 10 | not started | |
| Mesh controls, convergence | Mesh controls, adaptive meshing | 10 | not started | |
| — | Thermal, buckling, fatigue, drop test studies | 10 | not started | *(low priority)* |
| Dynamic Simulation | Motion Analysis | 10 | not started | |
| Generative Shape Design | Topology Study | 10 | not started | *(low priority)* |
| — | Flow Simulation (CFD) | 10 | not started | *(low priority)* |
| — | Costing | 10 | not started | Manufacturing cost estimates |
| — | Sustainability | 10 | not started | *(low priority)* |
| — | DFMXpress | 10 | not started | Design-for-manufacture checks |

## Tube, pipe and electrical routing

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Rigid piping | Routing: pipes | 10 | not started | |
| Bent tubing | Routing: tubes | 10 | not started | |
| Flexible hose | Routing: flexible tubes | 10 | not started | |
| Fittings, connections | Routing library components, connection points | 10 | not started | |
| Pipe styles | Routing templates | 10 | not started | |
| ISOGEN output | Piping isometrics | 10 | not started | *(low priority)* |
| Wires, cables, segments | Electrical routing, wires, cables | 10 | not started | |
| Ribbon cables | Ribbon cables | 10 | not started | |
| Electrical part authoring | Routing Library Manager | 10 | not started | |
| Nailboard drawings | Flattened route drawings | 10 | not started | |
| Harness reports, wire list import | From-to lists | 10 | not started | |

## File I/O and interoperability

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Native part format | Native part format | 1 | done | `.smp`, see [smp-format.md](smp-format.md) |
| STEP export: geometry | STEP export | 1 | in progress | AP214 schema via truck |
| STEP export: AP242 | STEP AP242 | 2 | not started | |
| STEP export: colours, names | STEP with appearances | 5 | not started | |
| STEP export: assemblies | STEP assembly structure | 6 | not started | |
| STEP export: PMI | STEP 242 with 3D PMI | 7 | not started | |
| STEP import: parts | STEP import | 2 | not started | |
| STEP import: assemblies | STEP import (assembly) | 6 | not started | |
| AnyCAD reference (linked import) | 3D Interconnect | 6 | not started | Updates when the source file changes |
| IGES | IGES | 9 | not started | |
| Parasolid (x_t) | Parasolid (x_t) | 9 | not started | Licensed format; check feasibility |
| ACIS (sat) | ACIS (sat) | 9 | not started | |
| JT | JT | 10 | not started | |
| STL | STL | 5 | not started | 3D printing |
| OBJ | — | 5 | not started | |
| 3MF | 3MF | 5 | not started | |
| DWG/DXF import | DXF/DWG import | 5 | not started | |
| PDF, 3D PDF | PDF, 3D PDF | 7 | not started | |
| Open other CAD (SolidWorks, Creo, NX, CATIA) | Open other CAD (Inventor, Creo, NX, CATIA) | 10 | not started | Proprietary; via STEP in practice |
| Revit (RFA), IFC export | IFC export | 10 | not started | *(low priority)* |
| FBX, glTF | glTF, FBX (via Visualize) | 10 | not started | |
| Point cloud (RCP, E57) | ScanTo3D | 10 | not started | *(low priority)* |
| Image export | Save as image | 4 | not started | |
| Pack and Go | Pack and Go | 6 | not started | |
| Templates | Templates | 5 | not started | |
| OldVersions folder | Backup copies | 4 | not started | |
| Autosave / recovery | Auto-recover | 4 | not started | |
| File thumbnails | File thumbnails | 4 | not started | freedesktop thumbnailer on Linux |
| Batch migration | Task Scheduler conversion | 10 | not started | |
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
| Ray tracing | PhotoView 360 / Visualize | 10 | not started | *(low priority)* |
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
| Localization | Localization | 10 | not started | *(low priority)* |
| Touch / tablet | Touch mode | 10 | not started | *(low priority)* |

## Data management and projects

| Inventor | SolidWorks | Step | Status | Notes |
|---|---|---|---|---|
| Project file (.ipj) | — | 6 | not started | Workspace, libraries, search paths |
| — | File Locations (system options) | 6 | not started | |
| Library paths | Reference search paths | 6 | not started | |
| Design Assistant (rename, replace with references) | Rename with references (SolidWorks Explorer) | 6 | not started | |
| Where Used | Where Used | 6 | not started | |
| Vault | PDM | 10 | not started | Could build on git and git LFS |
| Lifecycle states, revisions | Workflow states, revisions | 10 | not started | |
| — | — | 5 | not started | Git-friendly diffs of `.smp`; neither product has this |

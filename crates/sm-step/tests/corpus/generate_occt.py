#!/usr/bin/env python3
"""Regenerates the OpenCascade-written part of the STEP import corpus.

OpenCascade is an independent STEP writer, so these files check the importer
against geometry and conventions it did not produce itself. Each model is
written to `occt/<name>.step`; `expected/<name>.json` records what
OpenCascade reads back (per-solid volume and bounding box, in millimetres),
which the Rust tests use as the oracle.

    python3 -m venv venv && venv/bin/pip install cadquery-ocp==7.8.1.1.post1
    venv/bin/python generate_occt.py

Also run with `--oracle FILE...` to write expected JSON for STEP files from
other sources (see README.md).
"""

import json
import math
import os
import sys

from OCP.BRep import BRep_Builder
from OCP.BRepAlgoAPI import BRepAlgoAPI_Cut, BRepAlgoAPI_Fuse
from OCP.BRepBndLib import BRepBndLib
from OCP.BRepBuilderAPI import (
    BRepBuilderAPI_MakeEdge,
    BRepBuilderAPI_MakeFace,
    BRepBuilderAPI_MakeSolid,
    BRepBuilderAPI_MakeWire,
    BRepBuilderAPI_Transform,
)
from OCP.BRepFilletAPI import BRepFilletAPI_MakeFillet
from OCP.BRepGProp import BRepGProp
from OCP.BRepOffsetAPI import BRepOffsetAPI_ThruSections
from OCP.BRepPrimAPI import (
    BRepPrimAPI_MakeBox,
    BRepPrimAPI_MakeCone,
    BRepPrimAPI_MakeCylinder,
    BRepPrimAPI_MakePrism,
    BRepPrimAPI_MakeRevol,
    BRepPrimAPI_MakeSphere,
    BRepPrimAPI_MakeTorus,
)
from OCP.Bnd import Bnd_Box
from OCP.GC import GC_MakeEllipse
from OCP.Geom import Geom_BSplineCurve
from OCP.GProp import GProp_GProps
from OCP.IFSelect import IFSelect_RetDone
from OCP.Interface import Interface_Static
from OCP.STEPCAFControl import STEPCAFControl_Reader, STEPCAFControl_Writer
from OCP.STEPControl import STEPControl_AsIs, STEPControl_Reader, STEPControl_Writer
from OCP.TColStd import TColStd_Array1OfInteger, TColStd_Array1OfReal
from OCP.TColgp import TColgp_Array1OfPnt
from OCP.TCollection import TCollection_ExtendedString
from OCP.TDataStd import TDataStd_Name
from OCP.TDocStd import TDocStd_Document
from OCP.TopAbs import TopAbs_SOLID
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS, TopoDS_Shell
from OCP.XCAFDoc import XCAFDoc_DocumentTool
from OCP.gp import gp_Ax1, gp_Ax2, gp_Circ, gp_Dir, gp_Elips, gp_Pnt, gp_Trsf, gp_Vec

HERE = os.path.dirname(os.path.abspath(__file__))


def ax2(origin=(0, 0, 0), normal=(0, 0, 1), xdir=None):
    if xdir is None:
        return gp_Ax2(gp_Pnt(*origin), gp_Dir(*normal))
    return gp_Ax2(gp_Pnt(*origin), gp_Dir(*normal), gp_Dir(*xdir))


def bspline(points, degree=3):
    """A clamped, uniform B-spline curve through `points` as control points."""
    n = len(points)
    poles = TColgp_Array1OfPnt(1, n)
    for i, p in enumerate(points, 1):
        poles.SetValue(i, gp_Pnt(*p))
    inner = n - degree - 1
    knots = TColStd_Array1OfReal(1, inner + 2)
    mults = TColStd_Array1OfInteger(1, inner + 2)
    for i in range(inner + 2):
        knots.SetValue(i + 1, float(i))
        mults.SetValue(i + 1, degree + 1 if i in (0, inner + 1) else 1)
    return Geom_BSplineCurve(poles, knots, mults, degree)


def wire_of(*edges):
    mk = BRepBuilderAPI_MakeWire()
    for e in edges:
        mk.Add(e)
    return mk.Wire()


def models():
    box = BRepPrimAPI_MakeBox(10.0, 20.0, 30.0).Shape()
    yield "box", box

    yield "cylinder", BRepPrimAPI_MakeCylinder(ax2((1, 2, 3), (0, 1, 1)), 5.0, 12.0).Shape()
    yield "frustum", BRepPrimAPI_MakeCone(ax2(), 6.0, 2.0, 8.0).Shape()
    yield "cone_apex", BRepPrimAPI_MakeCone(ax2((0, 0, 5), (1, 0, 0)), 4.0, 0.0, 9.0).Shape()
    yield "sphere", BRepPrimAPI_MakeSphere(gp_Pnt(3, -2, 1), 7.5).Shape()
    yield "torus", BRepPrimAPI_MakeTorus(ax2((0, 0, 0), (0, 0, 1)), 10.0, 3.0).Shape()
    yield "partial_torus", BRepPrimAPI_MakeTorus(ax2(), 10.0, 3.0, math.radians(200)).Shape()

    # A plate with through holes and a blind counterbore: planar faces with
    # circular inner loops and seamed cylinders.
    plate = BRepPrimAPI_MakeBox(60.0, 40.0, 8.0).Shape()
    for x, y, r in [(10, 10, 3), (50, 10, 3), (50, 30, 4.5), (10, 30, 2)]:
        hole = BRepPrimAPI_MakeCylinder(ax2((x, y, -1)), r, 10.0).Shape()
        plate = BRepAlgoAPI_Cut(plate, hole).Shape()
    bore = BRepPrimAPI_MakeCylinder(ax2((30, 20, 5)), 8.0, 5.0).Shape()
    plate = BRepAlgoAPI_Cut(plate, bore).Shape()
    yield "plate_with_holes", plate

    # Fillets give cylinders, tori and blended corner patches.
    from OCP.TopAbs import TopAbs_EDGE

    base = BRepPrimAPI_MakeBox(20.0, 15.0, 10.0).Shape()
    fillet = BRepFilletAPI_MakeFillet(base)
    exp = TopExp_Explorer(base, TopAbs_EDGE)
    while exp.More():
        fillet.Add(2.0, TopoDS.Edge_s(exp.Current()))
        exp.Next()
    yield "filleted_box", fillet.Shape()

    # Loft between a circle and an ellipse: a closed (periodic) B-spline
    # surface with a seam.
    loft = BRepOffsetAPI_ThruSections(True, False)
    loft.AddWire(wire_of(BRepBuilderAPI_MakeEdge(gp_Circ(ax2(), 5.0)).Edge()))
    loft.AddWire(
        wire_of(BRepBuilderAPI_MakeEdge(gp_Elips(ax2((0, 0, 10)), 8.0, 3.0)).Edge())
    )
    loft.AddWire(wire_of(BRepBuilderAPI_MakeEdge(gp_Circ(ax2((1, 0, 18)), 4.0)).Edge()))
    loft.Build()
    yield "loft", loft.Shape()

    # Revolving a spline profile gives SURFACE_OF_REVOLUTION.
    pts = [(0, 0, 0), (4, 0, 0), (6, 0, 3), (3, 0, 6), (5, 0, 9), (0, 0, 10)]
    profile = BRepBuilderAPI_MakeEdge(bspline(pts)).Edge()
    axis_edge = BRepBuilderAPI_MakeEdge(gp_Pnt(0, 0, 10), gp_Pnt(0, 0, 0)).Edge()
    face = BRepBuilderAPI_MakeFace(wire_of(profile, axis_edge)).Face()
    yield "revolved_spline", BRepPrimAPI_MakeRevol(face, gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1))).Shape()

    # Extruding a spline and an ellipse gives SURFACE_OF_LINEAR_EXTRUSION.
    spline = bspline([(0, 0, 0), (5, 3, 0), (10, -3, 0), (15, 0, 0)])
    e1 = BRepBuilderAPI_MakeEdge(spline).Edge()
    e2 = BRepBuilderAPI_MakeEdge(gp_Pnt(15, 0, 0), gp_Pnt(15, 8, 0)).Edge()
    e3 = BRepBuilderAPI_MakeEdge(gp_Pnt(15, 8, 0), gp_Pnt(0, 8, 0)).Edge()
    e4 = BRepBuilderAPI_MakeEdge(gp_Pnt(0, 8, 0), gp_Pnt(0, 0, 0)).Edge()
    face = BRepBuilderAPI_MakeFace(wire_of(e1, e2, e3, e4)).Face()
    yield "extruded_spline", BRepPrimAPI_MakePrism(face, gp_Vec(0, 0, 6)).Shape()
    ellipse = GC_MakeEllipse(gp_Elips(ax2(), 7.0, 4.0)).Value()
    face = BRepBuilderAPI_MakeFace(wire_of(BRepBuilderAPI_MakeEdge(ellipse).Edge())).Face()
    yield "elliptic_prism", BRepPrimAPI_MakePrism(face, gp_Vec(2, 0, 9)).Shape()

    # A hollow ball: BREP_WITH_VOIDS.
    outer = BRepPrimAPI_MakeSphere(10.0).Shell()
    inner = BRepPrimAPI_MakeSphere(6.0).Shell()
    inner.Reverse()
    mk = BRepBuilderAPI_MakeSolid()
    mk.Add(outer)
    mk.Add(TopoDS.Shell_s(inner))
    yield "hollow_sphere", mk.Solid()

    # Two solids in one file.
    cyl = BRepPrimAPI_MakeCylinder(ax2((30, 0, 0)), 3.0, 10.0).Shape()
    yield "two_bodies", compound(box, cyl)


def compound(*shapes):
    from OCP.TopoDS import TopoDS_Compound

    c = TopoDS_Compound()
    b = BRep_Builder()
    b.MakeCompound(c)
    for s in shapes:
        b.Add(c, s)
    return c


def write_shape(shape, path, unit="MM", schema="AP242DIS"):
    Interface_Static.SetCVal_s("write.step.schema", schema)
    Interface_Static.SetCVal_s("write.step.unit", unit)
    Interface_Static.SetCVal_s("write.step.product.name", os.path.basename(path))
    w = STEPControl_Writer()
    w.Transfer(shape, STEPControl_AsIs)
    assert w.Write(path) == IFSelect_RetDone, path


def write_assembly(path):
    """A two-level assembly: a bracket (box) with two instances of a pin
    (cylinder) placed by transforms, one of them in a sub-assembly."""
    doc = TDocStd_Document(TCollection_ExtendedString("XmlOcaf"))
    tool = XCAFDoc_DocumentTool.ShapeTool_s(doc.Main())

    def named(label, name):
        TDataStd_Name.Set_s(label, TCollection_ExtendedString(name))

    from OCP.TopLoc import TopLoc_Location
    from OCP.TopoDS import TopoDS_Compound

    def empty_assembly(name):
        c = TopoDS_Compound()
        BRep_Builder().MakeCompound(c)
        label = tool.AddShape(c, True)
        named(label, name)
        return label

    bracket = tool.AddShape(BRepPrimAPI_MakeBox(40.0, 10.0, 5.0).Shape(), False)
    named(bracket, "bracket")
    pin = tool.AddShape(BRepPrimAPI_MakeCylinder(2.0, 15.0).Shape(), False)
    named(pin, "pin")

    def loc(dx, dy, dz, angle_deg=0.0, axis=(0, 0, 1)):
        t = gp_Trsf()
        t.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(*axis)), math.radians(angle_deg))
        m = gp_Trsf()
        m.SetTranslation(gp_Vec(dx, dy, dz))
        return TopLoc_Location(m.Multiplied(t))

    sub = empty_assembly("pin_pair")
    tool.AddComponent(sub, pin, loc(0, 0, 0))
    tool.AddComponent(sub, pin, loc(10, 0, 0, 90, (1, 0, 0)))
    top = empty_assembly("top")
    tool.AddComponent(top, bracket, loc(0, 0, 0))
    tool.AddComponent(top, sub, loc(5, 5, 5, 30, (0, 0, 1)))
    tool.AddComponent(top, pin, loc(35, 5, -10))
    tool.UpdateAssemblies()

    Interface_Static.SetCVal_s("write.step.schema", "AP242DIS")
    Interface_Static.SetCVal_s("write.step.unit", "MM")
    w = STEPCAFControl_Writer()
    w.SetNameMode(True)
    w.Transfer(doc, STEPControl_AsIs)
    assert w.Write(path) == IFSelect_RetDone


def oracle(step_path):
    """Reads a STEP file with OpenCascade and returns per-solid volume and
    bounding box, in millimetres, sorted for stable comparison."""
    Interface_Static.SetCVal_s("xstep.cascade.unit", "MM")
    r = STEPControl_Reader()
    assert r.ReadFile(step_path) == IFSelect_RetDone, step_path
    r.TransferRoots()
    shape = r.OneShape()
    solids = []
    exp = TopExp_Explorer(shape, TopAbs_SOLID)
    while exp.More():
        s = exp.Current()
        props = GProp_GProps()
        BRepGProp.VolumeProperties_s(s, props)
        box = Bnd_Box()
        BRepBndLib.AddOptimal_s(s, box, False, False)
        xmin, ymin, zmin, xmax, ymax, zmax = box.Get()
        faces = 0
        from OCP.TopAbs import TopAbs_FACE

        fexp = TopExp_Explorer(s, TopAbs_FACE)
        while fexp.More():
            faces += 1
            fexp.Next()
        solids.append(
            {
                "volume": round(props.Mass(), 6),
                "bbox": [round(v, 6) for v in (xmin, ymin, zmin, xmax, ymax, zmax)],
                "faces": faces,
            }
        )
        exp.Next()
    solids.sort(key=lambda s: (s["bbox"], s["volume"]))
    return {"solids": solids}


def write_oracle(step_path, name):
    out = os.path.join(HERE, "expected", name + ".json")
    with open(out, "w") as f:
        json.dump(oracle(step_path), f, indent=1)
        f.write("\n")


def main():
    if sys.argv[1:2] == ["--oracle"]:
        for path in sys.argv[2:]:
            write_oracle(path, os.path.splitext(os.path.basename(path))[0])
        return
    os.makedirs(os.path.join(HERE, "occt"), exist_ok=True)
    os.makedirs(os.path.join(HERE, "expected"), exist_ok=True)
    for name, shape in models():
        path = os.path.join(HERE, "occt", name + ".step")
        write_shape(shape, path)
        write_oracle(path, name)
    # Same box in inches and as AP214, for unit and schema handling.
    box = BRepPrimAPI_MakeBox(10.0, 20.0, 30.0).Shape()
    for name, unit, schema in [("box_inch", "INCH", "AP242DIS"), ("box_ap214", "MM", "AP214IS"), ("box_ap203", "M", "AP203")]:
        path = os.path.join(HERE, "occt", name + ".step")
        write_shape(box, path, unit=unit, schema=schema)
        write_oracle(path, name)
    path = os.path.join(HERE, "occt", "assembly.step")
    write_assembly(path)
    write_oracle(path, "assembly")


if __name__ == "__main__":
    main()

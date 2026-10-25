#include "sm-occt/cpp/shim.h"
#include "sm-occt/src/ffi.rs.h"

#include <BOPAlgo_Alerts.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepGProp.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepTools_History.hxx>
#include <BRep_Tool.hxx>
#include <BinTools.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Interface_Static.hxx>
#include <Poly_Triangulation.hxx>
#include <Precision.hxx>
#include <Quantity_Color.hxx>
#include <STEPCAFControl_Controller.hxx>
#include <STEPCAFControl_Reader.hxx>
#include <STEPCAFControl_Writer.hxx>
#include <STEPControl_Reader.hxx>
#include <ShapeUpgrade_UnifySameDomain.hxx>
#include <Standard_Version.hxx>
#include <TDF_ChildIterator.hxx>
#include <TDF_LabelSequence.hxx>
#include <TDataStd_Name.hxx>
#include <TDocStd_Document.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopLoc_Location.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Wire.hxx>
#include <XCAFApp_Application.hxx>
#include <XCAFDoc_ColorTool.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>
#include <gp_Ax1.hxx>
#include <gp_Pln.hxx>
#include <gp_Vec.hxx>

#include <cmath>
#include <sstream>
#include <stdexcept>

namespace smocct {

namespace {

// Sub-shape kinds as seen from Rust (SubKind).
constexpr uint8_t KIND_VERTEX = 0;
constexpr uint8_t KIND_EDGE = 1;
constexpr uint8_t KIND_FACE = 2;
constexpr uint8_t KIND_SOLID = 3;

// History relations as seen from Rust (Relation).
constexpr uint32_t REL_KEPT = 0;
constexpr uint32_t REL_MODIFIED = 1;
constexpr uint32_t REL_GENERATED = 2;
constexpr uint32_t REL_DELETED = 3;

TopAbs_ShapeEnum to_occ(uint8_t kind) {
  switch (kind) {
  case KIND_VERTEX:
    return TopAbs_VERTEX;
  case KIND_EDGE:
    return TopAbs_EDGE;
  case KIND_FACE:
    return TopAbs_FACE;
  case KIND_SOLID:
    return TopAbs_SOLID;
  }
  throw std::invalid_argument("unknown sub-shape kind");
}

int from_occ(TopAbs_ShapeEnum t) {
  switch (t) {
  case TopAbs_VERTEX:
    return KIND_VERTEX;
  case TopAbs_EDGE:
    return KIND_EDGE;
  case TopAbs_FACE:
    return KIND_FACE;
  case TopAbs_SOLID:
    return KIND_SOLID;
  default:
    return -1;
  }
}

void require(bool ok, const char *what) {
  if (!ok)
    throw std::runtime_error(what);
}

void require_shape(const TopoDS_Shape &s, const char *what) {
  if (s.IsNull())
    throw std::runtime_error(std::string(what) + ": empty shape");
}

void require_finite(std::initializer_list<double> values) {
  for (double v : values)
    if (!std::isfinite(v))
      throw std::invalid_argument("non-finite input");
}

TopTools_IndexedMapOfShape map_of(const TopoDS_Shape &s, uint8_t kind) {
  TopTools_IndexedMapOfShape m;
  TopExp::MapShapes(s, to_occ(kind), m);
  return m;
}

// Sub-shape `index` (0-based) of `kind` in `s`'s indexed map.
TopoDS_Shape sub_of(const TopoDS_Shape &s, uint8_t kind, uint32_t index) {
  TopTools_IndexedMapOfShape m = map_of(s, kind);
  if (index >= static_cast<uint32_t>(m.Extent()))
    throw std::out_of_range("sub-shape index out of range");
  return m(static_cast<int>(index) + 1);
}

// Records, for every vertex, edge and face of every argument, whether it
// was kept, modified, generated from or deleted, against the result's own
// indexed maps. Works for any algorithm with the IsDeleted / Modified /
// Generated protocol.
template <class Algo>
void fill_history(OpResult &r, const std::vector<TopoDS_Shape> &args, Algo &algo) {
  TopTools_IndexedMapOfShape result_maps[3];
  for (uint8_t k = 0; k < 3; ++k)
    result_maps[k] = map_of(r.shape, k);
  auto find = [&](const TopoDS_Shape &s, uint32_t &kind, uint32_t &index) {
    int k = from_occ(s.ShapeType());
    if (k < 0 || k > 2)
      return false;
    int i = result_maps[k].FindIndex(s);
    if (i <= 0)
      return false;
    kind = static_cast<uint32_t>(k);
    index = static_cast<uint32_t>(i - 1);
    return true;
  };
  for (uint32_t a = 0; a < args.size(); ++a) {
    if (args[a].IsNull())
      continue;
    for (uint8_t k = 0; k < 3; ++k) {
      TopTools_IndexedMapOfShape m = map_of(args[a], k);
      for (int i = 1; i <= m.Extent(); ++i) {
        const TopoDS_Shape &s = m(i);
        const uint32_t idx = static_cast<uint32_t>(i - 1);
        const bool deleted = algo.IsDeleted(s);
        if (deleted)
          r.history.push_back({a, k, idx, REL_DELETED, k, 0});
        TopTools_ListOfShape modified = algo.Modified(s);
        uint32_t rk, ri;
        for (const TopoDS_Shape &m2 : modified)
          if (find(m2, rk, ri))
            r.history.push_back({a, k, idx, REL_MODIFIED, rk, ri});
        TopTools_ListOfShape generated = algo.Generated(s);
        for (const TopoDS_Shape &g : generated)
          if (find(g, rk, ri))
            r.history.push_back({a, k, idx, REL_GENERATED, rk, ri});
        if (!deleted && modified.IsEmpty() && find(s, rk, ri))
          r.history.push_back({a, k, idx, REL_KEPT, rk, ri});
      }
    }
  }
}

// BRepTools_History names deletion IsRemoved.
struct HistoryAdapter {
  const Handle(BRepTools_History) & h;
  bool IsDeleted(const TopoDS_Shape &s) const { return !h.IsNull() && h->IsRemoved(s); }
  TopTools_ListOfShape Modified(const TopoDS_Shape &s) const {
    return h.IsNull() ? TopTools_ListOfShape() : h->Modified(s);
  }
  TopTools_ListOfShape Generated(const TopoDS_Shape &s) const {
    return h.IsNull() ? TopTools_ListOfShape() : h->Generated(s);
  }
};

std::unique_ptr<OpResult> finish(TopoDS_Shape shape, const char *what) {
  require_shape(shape, what);
  auto r = std::make_unique<OpResult>();
  r->shape = std::move(shape);
  return r;
}

TopTools_ListOfShape subs_list(const TopoDS_Shape &s, uint8_t kind,
                               rust::Slice<const uint32_t> indices) {
  TopTools_IndexedMapOfShape m = map_of(s, kind);
  TopTools_ListOfShape out;
  for (uint32_t i : indices) {
    if (i >= static_cast<uint32_t>(m.Extent()))
      throw std::out_of_range("sub-shape index out of range");
    out.Append(m(static_cast<int>(i) + 1));
  }
  return out;
}

std::string alerts(const BRepAlgoAPI_BooleanOperation &algo) {
  std::ostringstream out;
  algo.DumpErrors(out);
  std::string text = out.str();
  return text.empty() ? std::string("boolean operation failed") : "boolean operation failed: " + text;
}

// STEP translation uses process-wide static parameters; Rust serialises
// calls with a mutex.
void step_static_params() {
  STEPCAFControl_Controller::Init();
  Interface_Static::SetCVal("xstep.cascade.unit", "MM");
  Interface_Static::SetCVal("write.step.unit", "MM");
  Interface_Static::SetCVal("write.step.schema", "AP242DIS");
  Interface_Static::SetIVal("write.surfacecurve.mode", 1);
}

Handle(TDocStd_Document) new_xcaf_document() {
  Handle(TDocStd_Document) doc;
  XCAFApp_Application::GetApplication()->NewDocument("MDTV-XCAF", doc);
  require(!doc.IsNull(), "could not create an XCAF document");
  return doc;
}

} // namespace

rust::String occt_version() { return rust::String(OCC_VERSION_COMPLETE); }

std::unique_ptr<Shape> shape_clone(const Shape &s) { return std::make_unique<Shape>(s.shape); }

bool shape_is_null(const Shape &s) { return s.shape.IsNull(); }

uint8_t shape_type(const Shape &s) {
  require_shape(s.shape, "shape_type");
  return static_cast<uint8_t>(s.shape.ShapeType());
}

uint32_t shape_count(const Shape &s, uint8_t kind) {
  if (s.shape.IsNull())
    return 0;
  return static_cast<uint32_t>(map_of(s.shape, kind).Extent());
}

bool shape_same(const Shape &s, uint8_t kind, uint32_t i, const Shape &t, uint8_t tkind, uint32_t j) {
  return sub_of(s.shape, kind, i).IsSame(sub_of(t.shape, tkind, j));
}

std::unique_ptr<Shape> shape_sub(const Shape &s, uint8_t kind, uint32_t index) {
  return std::make_unique<Shape>(sub_of(s.shape, kind, index));
}

std::unique_ptr<Shape> make_box(double dx, double dy, double dz) {
  require_finite({dx, dy, dz});
  require(dx > Precision::Confusion() && dy > Precision::Confusion() && dz > Precision::Confusion(),
          "box sizes must be positive");
  return std::make_unique<Shape>(BRepPrimAPI_MakeBox(dx, dy, dz).Shape());
}

std::unique_ptr<Shape> make_cylinder(double r, double h) {
  require_finite({r, h});
  require(r > Precision::Confusion() && h > Precision::Confusion(),
          "cylinder sizes must be positive");
  return std::make_unique<Shape>(BRepPrimAPI_MakeCylinder(r, h).Shape());
}

std::unique_ptr<OpResult> polygon_face(rust::Slice<const double> points,
                                       rust::Slice<const uint32_t> loop_lens) {
  require(!loop_lens.empty(), "polygon face needs at least one loop");
  size_t total = 0;
  for (uint32_t n : loop_lens) {
    require(n >= 3, "every loop needs at least 3 points");
    total += n;
  }
  require(points.size() == total * 3, "point buffer does not match loop lengths");
  for (double v : points)
    require(std::isfinite(v), "non-finite point coordinate");

  std::vector<std::vector<gp_Pnt>> loops;
  size_t at = 0;
  for (uint32_t n : loop_lens) {
    std::vector<gp_Pnt> loop;
    for (uint32_t i = 0; i < n; ++i, ++at)
      loop.emplace_back(points[3 * at], points[3 * at + 1], points[3 * at + 2]);
    for (uint32_t i = 0; i < n; ++i)
      require(loop[i].Distance(loop[(i + 1) % n]) > Precision::Confusion(),
              "loop has repeated consecutive points");
    loops.push_back(std::move(loop));
  }

  // Newell normal: the outer loop is counter-clockwise about it.
  auto newell = [](const std::vector<gp_Pnt> &l) {
    gp_XYZ n(0, 0, 0);
    for (size_t i = 0; i < l.size(); ++i) {
      const gp_Pnt &a = l[i], &b = l[(i + 1) % l.size()];
      n += gp_XYZ((a.Y() - b.Y()) * (a.Z() + b.Z()), (a.Z() - b.Z()) * (a.X() + b.X()),
                  (a.X() - b.X()) * (a.Y() + b.Y()));
    }
    return n;
  };
  gp_XYZ normal = newell(loops[0]);
  require(normal.Modulus() > Precision::Confusion(), "outer loop is degenerate");
  gp_Pln plane(loops[0][0], gp_Dir(normal));
  for (const auto &l : loops)
    for (const gp_Pnt &p : l)
      require(plane.Distance(p) <= 1e-6 * (1.0 + p.XYZ().Modulus()), "loops are not planar");

  std::vector<TopoDS_Wire> wires;
  // segment_edges[l][k] is the edge from point k to point k+1 of loop l.
  std::vector<std::vector<TopoDS_Edge>> segment_edges;
  for (size_t li = 0; li < loops.size(); ++li) {
    BRepBuilderAPI_MakePolygon poly;
    std::vector<TopoDS_Edge> seg;
    for (size_t k = 0; k < loops[li].size(); ++k) {
      poly.Add(loops[li][k]);
      if (k > 0) {
        require(poly.Added(), "could not add a polygon point");
        seg.push_back(poly.Edge());
      }
    }
    poly.Close();
    require(poly.IsDone(), "could not build a polygon loop");
    seg.push_back(poly.Edge());
    TopoDS_Wire w = poly.Wire();
    if (li > 0 && newell(loops[li]).Dot(normal) > 0)
      w.Reverse(); // holes run clockwise
    wires.push_back(w);
    segment_edges.push_back(std::move(seg));
  }
  BRepBuilderAPI_MakeFace mk(plane, wires[0], Standard_True);
  for (size_t li = 1; li < wires.size(); ++li)
    mk.Add(wires[li]);
  require(mk.IsDone(), "could not build a planar face");
  auto r = finish(mk.Face(), "polygon_face");
  require(BRepCheck_Analyzer(r->shape).IsValid(), "polygon face is invalid (self-intersecting loops?)");

  // Edge index of each segment, in input order.
  TopTools_IndexedMapOfShape edges = map_of(r->shape, KIND_EDGE);
  for (const auto &seg : segment_edges)
    for (const TopoDS_Edge &e : seg) {
      int i = edges.FindIndex(e);
      require(i > 0, "polygon edge lost while building the face");
      r->extra.push_back(static_cast<uint32_t>(i - 1));
    }
  return r;
}

std::unique_ptr<OpResult> prism(const Shape &profile, double dx, double dy, double dz) {
  require_shape(profile.shape, "prism profile");
  require_finite({dx, dy, dz});
  gp_Vec v(dx, dy, dz);
  require(v.Magnitude() > Precision::Confusion(), "prism vector is zero");
  BRepPrimAPI_MakePrism mk(profile.shape, v, Standard_False, Standard_True);
  require(mk.IsDone(), "prism failed");
  auto r = finish(mk.Shape(), "prism");
  fill_history(*r, {profile.shape}, mk);
  r->first = mk.FirstShape();
  r->last = mk.LastShape();
  return r;
}

std::unique_ptr<OpResult> revolve(const Shape &profile, double ox, double oy, double oz, double dx,
                                  double dy, double dz, double angle) {
  require_shape(profile.shape, "revolve profile");
  require_finite({ox, oy, oz, dx, dy, dz, angle});
  gp_Vec d(dx, dy, dz);
  require(d.Magnitude() > Precision::Confusion(), "revolve axis is zero");
  require(std::abs(angle) > Precision::Angular(), "revolve angle is zero");
  gp_Ax1 axis(gp_Pnt(ox, oy, oz), gp_Dir(d));
  const bool full = std::abs(angle) >= 2 * M_PI - Precision::Angular();
  std::unique_ptr<BRepPrimAPI_MakeRevol> mk =
      full ? std::make_unique<BRepPrimAPI_MakeRevol>(profile.shape, axis, Standard_False)
           : std::make_unique<BRepPrimAPI_MakeRevol>(profile.shape, axis, angle, Standard_False);
  require(mk->IsDone(), "revolve failed");
  auto r = finish(mk->Shape(), "revolve");
  fill_history(*r, {profile.shape}, *mk);
  if (!full) {
    r->first = mk->FirstShape();
    r->last = mk->LastShape();
  }
  return r;
}

std::unique_ptr<OpResult> boolean_op(uint8_t op, const Shape &a, const Shape &b) {
  require_shape(a.shape, "boolean argument");
  require_shape(b.shape, "boolean tool");
  std::unique_ptr<BRepAlgoAPI_BooleanOperation> mk;
  switch (op) {
  case 0:
    mk = std::make_unique<BRepAlgoAPI_Fuse>(a.shape, b.shape);
    break;
  case 1:
    mk = std::make_unique<BRepAlgoAPI_Cut>(a.shape, b.shape);
    break;
  case 2:
    mk = std::make_unique<BRepAlgoAPI_Common>(a.shape, b.shape);
    break;
  default:
    throw std::invalid_argument("unknown boolean operation");
  }
  if (mk->HasErrors() || !mk->IsDone())
    throw std::runtime_error(alerts(*mk));
  auto r = finish(mk->Shape(), "boolean");
  fill_history(*r, {a.shape, b.shape}, *mk);
  return r;
}

std::unique_ptr<OpResult> fillet(const Shape &s, rust::Slice<const uint32_t> edges, double radius) {
  require_shape(s.shape, "fillet");
  require_finite({radius});
  require(radius > Precision::Confusion(), "fillet radius must be positive");
  require(!edges.empty(), "fillet needs at least one edge");
  BRepFilletAPI_MakeFillet mk(s.shape);
  for (const TopoDS_Shape &e : subs_list(s.shape, KIND_EDGE, edges))
    mk.Add(radius, TopoDS::Edge(e));
  mk.Build();
  require(mk.IsDone(), "fillet failed (radius too large for the neighbouring faces?)");
  auto r = finish(mk.Shape(), "fillet");
  fill_history(*r, {s.shape}, mk);
  return r;
}

std::unique_ptr<OpResult> chamfer(const Shape &s, rust::Slice<const uint32_t> edges, double distance) {
  require_shape(s.shape, "chamfer");
  require_finite({distance});
  require(distance > Precision::Confusion(), "chamfer distance must be positive");
  require(!edges.empty(), "chamfer needs at least one edge");
  BRepFilletAPI_MakeChamfer mk(s.shape);
  for (const TopoDS_Shape &e : subs_list(s.shape, KIND_EDGE, edges))
    mk.Add(distance, TopoDS::Edge(e));
  mk.Build();
  require(mk.IsDone(), "chamfer failed (distance too large?)");
  auto r = finish(mk.Shape(), "chamfer");
  fill_history(*r, {s.shape}, mk);
  return r;
}

std::unique_ptr<OpResult> shell(const Shape &s, rust::Slice<const uint32_t> faces, double thickness,
                                double tolerance) {
  require_shape(s.shape, "shell");
  require_finite({thickness, tolerance});
  require(std::abs(thickness) > Precision::Confusion(), "shell thickness must be non-zero");
  require(tolerance > 0, "shell tolerance must be positive");
  BRepOffsetAPI_MakeThickSolid mk;
  // Positive thickness keeps the outside and hollows inwards, like
  // Inventor's Shell with the default inside direction.
  mk.MakeThickSolidByJoin(s.shape, subs_list(s.shape, KIND_FACE, faces), -thickness, tolerance);
  mk.Build();
  require(mk.IsDone(), "shell failed");
  auto r = finish(mk.Shape(), "shell");
  fill_history(*r, {s.shape}, mk);
  return r;
}

std::unique_ptr<OpResult> draft(const Shape &s, rust::Slice<const uint32_t> faces, double dx, double dy,
                                double dz, double angle, double px, double py, double pz, double nx,
                                double ny, double nz) {
  require_shape(s.shape, "draft");
  require_finite({dx, dy, dz, angle, px, py, pz, nx, ny, nz});
  require(!faces.empty(), "draft needs at least one face");
  require(gp_Vec(dx, dy, dz).Magnitude() > Precision::Confusion(), "draft direction is zero");
  require(gp_Vec(nx, ny, nz).Magnitude() > Precision::Confusion(), "neutral plane normal is zero");
  require(std::abs(angle) < M_PI / 2, "draft angle must be below 90 degrees");
  BRepOffsetAPI_DraftAngle mk(s.shape);
  gp_Pln neutral(gp_Pnt(px, py, pz), gp_Dir(nx, ny, nz));
  for (const TopoDS_Shape &f : subs_list(s.shape, KIND_FACE, faces)) {
    mk.Add(TopoDS::Face(f), gp_Dir(dx, dy, dz), angle, neutral);
    require(mk.AddDone(), "draft cannot be applied to one of the faces");
  }
  mk.Build();
  require(mk.IsDone(), "draft failed");
  auto r = finish(mk.Shape(), "draft");
  fill_history(*r, {s.shape}, mk);
  return r;
}

std::unique_ptr<OpResult> unify_same_domain(const Shape &s) {
  require_shape(s.shape, "unify");
  ShapeUpgrade_UnifySameDomain u(s.shape, Standard_True, Standard_True, Standard_False);
  u.Build();
  auto r = finish(u.Shape(), "unify");
  HistoryAdapter h{u.History()};
  fill_history(*r, {s.shape}, h);
  return r;
}

std::unique_ptr<Shape> op_shape(const OpResult &r) { return std::make_unique<Shape>(r.shape); }

rust::Vec<HistEntry> op_history(const OpResult &r) {
  rust::Vec<HistEntry> out;
  out.reserve(r.history.size());
  for (const auto &e : r.history)
    out.push_back(HistEntry{static_cast<uint8_t>(e[0]), static_cast<uint8_t>(e[1]), e[2],
                            static_cast<uint8_t>(e[3]), static_cast<uint8_t>(e[4]), e[5]});
  return out;
}

std::unique_ptr<Shape> op_first(const OpResult &r) { return std::make_unique<Shape>(r.first); }
std::unique_ptr<Shape> op_last(const OpResult &r) { return std::make_unique<Shape>(r.last); }

rust::Vec<uint32_t> op_extra(const OpResult &r) {
  rust::Vec<uint32_t> out;
  for (uint32_t v : r.extra)
    out.push_back(v);
  return out;
}

bool check_valid(const Shape &s) {
  require_shape(s.shape, "check_valid");
  return BRepCheck_Analyzer(s.shape).IsValid();
}

MassProps mass_props(const Shape &s) {
  require_shape(s.shape, "mass_props");
  GProp_GProps vol, area;
  BRepGProp::VolumeProperties(s.shape, vol);
  BRepGProp::SurfaceProperties(s.shape, area);
  gp_Pnt c = std::abs(vol.Mass()) > Precision::Confusion() ? vol.CentreOfMass() : area.CentreOfMass();
  return MassProps{vol.Mass(), area.Mass(), c.X(), c.Y(), c.Z()};
}

BoundingBox bounding_box(const Shape &s) {
  require_shape(s.shape, "bounding_box");
  Bnd_Box box;
  BRepBndLib::AddOptimal(s.shape, box, Standard_False, Standard_False);
  require(!box.IsVoid(), "shape has no extent");
  double x0, y0, z0, x1, y1, z1;
  box.Get(x0, y0, z0, x1, y1, z1);
  return BoundingBox{x0, y0, z0, x1, y1, z1};
}

FaceInfo face_info(const Shape &s, uint32_t index) {
  TopoDS_Face f = TopoDS::Face(sub_of(s.shape, KIND_FACE, index));
  BRepAdaptor_Surface surf(f);
  FaceInfo info{};
  info.kind = static_cast<uint8_t>(surf.GetType());
  if (surf.GetType() == GeomAbs_Plane) {
    gp_Pln pln = surf.Plane();
    gp_Dir n = pln.Axis().Direction();
    if (f.Orientation() == TopAbs_REVERSED)
      n.Reverse();
    info.ox = pln.Location().X();
    info.oy = pln.Location().Y();
    info.oz = pln.Location().Z();
    info.nx = n.X();
    info.ny = n.Y();
    info.nz = n.Z();
  }
  Bnd_Box box;
  BRepBndLib::AddOptimal(f, box, Standard_False, Standard_False);
  if (!box.IsVoid())
    box.Get(info.xmin, info.ymin, info.zmin, info.xmax, info.ymax, info.zmax);
  GProp_GProps area;
  BRepGProp::SurfaceProperties(f, area);
  info.area = area.Mass();
  return info;
}

rust::Vec<double> edge_ends(const Shape &s, uint32_t index) {
  TopoDS_Edge e = TopoDS::Edge(sub_of(s.shape, KIND_EDGE, index));
  TopoDS_Vertex a, b;
  TopExp::Vertices(e, a, b, Standard_True);
  require(!a.IsNull() && !b.IsNull(), "edge has no end vertices");
  gp_Pnt p = BRep_Tool::Pnt(a), q = BRep_Tool::Pnt(b);
  rust::Vec<double> out;
  for (double v : {p.X(), p.Y(), p.Z(), q.X(), q.Y(), q.Z()})
    out.push_back(v);
  return out;
}

MeshData tessellate(const Shape &s, double linear, double angular) {
  require_shape(s.shape, "tessellate");
  require_finite({linear, angular});
  require(linear > 0 && angular > 0, "tessellation tolerances must be positive");
  // Mesh a copy so the caller's shape (which may be shared) is not changed.
  TopoDS_Shape copy = s.shape;
  BRepMesh_IncrementalMesh mesher(copy, linear, Standard_False, angular, Standard_False);
  require(mesher.IsDone(), "tessellation failed");

  MeshData out;
  TopTools_IndexedMapOfShape faces = map_of(copy, KIND_FACE);
  out.face_ranges.push_back(0);
  for (int fi = 1; fi <= faces.Extent(); ++fi) {
    TopoDS_Face f = TopoDS::Face(faces(fi));
    TopLoc_Location loc;
    Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(f, loc);
    if (!tri.IsNull() && tri->NbTriangles() > 0) {
      const bool reversed = f.Orientation() == TopAbs_REVERSED;
      const gp_Trsf trsf = loc.Transformation();
      const uint32_t base = static_cast<uint32_t>(out.positions.size() / 3);
      BRepAdaptor_Surface surf(f, Standard_False);
      std::vector<gp_XYZ> nodes;
      for (int i = 1; i <= tri->NbNodes(); ++i) {
        gp_Pnt p = tri->Node(i).Transformed(trsf);
        out.positions.push_back(p.X());
        out.positions.push_back(p.Y());
        out.positions.push_back(p.Z());
        nodes.push_back(p.XYZ());
      }
      std::vector<gp_XYZ> normals(nodes.size(), gp_XYZ(0, 0, 0));
      std::vector<bool> exact(nodes.size(), false);
      if (tri->HasUVNodes()) {
        for (int i = 1; i <= tri->NbNodes(); ++i) {
          gp_Pnt2d uv = tri->UVNode(i);
          gp_Pnt p;
          gp_Vec du, dv;
          surf.D1(uv.X(), uv.Y(), p, du, dv);
          gp_Vec n = du.Crossed(dv);
          if (n.Magnitude() > 1e-12) {
            n.Normalize();
            n.Transform(trsf);
            if (reversed)
              n.Reverse();
            normals[i - 1] = n.XYZ();
            exact[i - 1] = true;
          }
        }
      }
      for (int t = 1; t <= tri->NbTriangles(); ++t) {
        int a, b, c;
        tri->Triangle(t).Get(a, b, c);
        if (reversed)
          std::swap(b, c);
        out.indices.push_back(base + a - 1);
        out.indices.push_back(base + b - 1);
        out.indices.push_back(base + c - 1);
        // Degenerate points (poles, apexes) take the mean of their triangles.
        gp_XYZ fn = (nodes[b - 1] - nodes[a - 1]).Crossed(nodes[c - 1] - nodes[a - 1]);
        for (int v : {a, b, c})
          if (!exact[v - 1])
            normals[v - 1] += fn;
      }
      for (const gp_XYZ &n : normals) {
        double m = n.Modulus();
        gp_XYZ u = m > 1e-30 ? n / m : gp_XYZ(0, 0, 1);
        out.normals.push_back(u.X());
        out.normals.push_back(u.Y());
        out.normals.push_back(u.Z());
      }
    }
    out.face_ranges.push_back(static_cast<uint32_t>(out.indices.size() / 3));
  }
  return out;
}

rust::Vec<uint8_t> to_brep(const Shape &s) {
  require_shape(s.shape, "to_brep");
  std::ostringstream out(std::ios::binary);
  BinTools::Write(s.shape, out, Standard_False, Standard_False, BinTools_FormatVersion_CURRENT);
  require(out.good(), "could not serialise shape");
  const std::string bytes = out.str();
  rust::Vec<uint8_t> v;
  v.reserve(bytes.size());
  for (char c : bytes)
    v.push_back(static_cast<uint8_t>(c));
  return v;
}

std::unique_ptr<Shape> from_brep(rust::Slice<const uint8_t> bytes) {
  std::istringstream in(std::string(reinterpret_cast<const char *>(bytes.data()), bytes.size()),
                        std::ios::binary);
  TopoDS_Shape shape;
  BinTools::Read(shape, in);
  require_shape(shape, "from_brep");
  return std::make_unique<Shape>(shape);
}

namespace {

struct StepCollector {
  Handle(XCAFDoc_ShapeTool) shapes;
  Handle(XCAFDoc_ColorTool) colors;
  uint32_t max_bodies;
  rust::Vec<StepBody> out;

  static std::string name_of(const TDF_Label &label) {
    Handle(TDataStd_Name) name;
    if (label.FindAttribute(TDataStd_Name::GetID(), name)) {
      const TCollection_ExtendedString &ext = name->Get();
      std::string utf8(static_cast<size_t>(ext.LengthOfCString()) + 1, '\0');
      Standard_PCharacter buf = &utf8[0];
      const int n = ext.ToUTF8CString(buf);
      utf8.resize(static_cast<size_t>(n > 0 ? n : 0));
      return utf8;
    }
    return std::string();
  }

  bool color_of(const TDF_Label &label, const TopoDS_Shape &shape, Quantity_Color &c) const {
    for (XCAFDoc_ColorType t : {XCAFDoc_ColorSurf, XCAFDoc_ColorGen})
      if (colors->GetColor(label, t, c) || colors->GetColor(shape, t, c))
        return true;
    // Many exporters colour faces rather than the solid; use the first
    // coloured face.
    for (TopExp_Explorer ex(shape, TopAbs_FACE); ex.More(); ex.Next())
      for (XCAFDoc_ColorType t : {XCAFDoc_ColorSurf, XCAFDoc_ColorGen})
        if (colors->GetColor(ex.Current(), t, c))
          return true;
    return false;
  }

  void emit(const TopoDS_Shape &solid, const std::string &name, bool has_color, const Quantity_Color &c) {
    if (out.size() >= max_bodies)
      throw std::runtime_error("too many bodies in the STEP file");
    StepBody body;
    body.name = rust::String(name);
    body.has_color = has_color;
    double r = 0, g = 0, b = 0;
    if (has_color)
      c.Values(r, g, b, Quantity_TOC_sRGB);
    body.r = r;
    body.g = g;
    body.b = b;
    body.brep = to_brep(Shape(solid));
    out.push_back(std::move(body));
  }

  void visit(const TDF_Label &label, const TopLoc_Location &loc, const std::string &instance_name,
             int depth) {
    if (depth > 64)
      throw std::runtime_error("assembly nested too deeply");
    TDF_Label target = label;
    if (shapes->IsReference(label))
      shapes->GetReferredShape(label, target);
    if (shapes->IsAssembly(target)) {
      TDF_LabelSequence comps;
      shapes->GetComponents(target, comps, Standard_False);
      for (int i = 1; i <= comps.Length(); ++i) {
        const TDF_Label &comp = comps.Value(i);
        visit(comp, loc * shapes->GetLocation(comp), name_of(comp), depth + 1);
      }
      return;
    }
    TopoDS_Shape shape = shapes->GetShape(target);
    if (shape.IsNull())
      return;
    std::string name = name_of(target);
    if (name.empty())
      name = instance_name;
    Quantity_Color c;
    bool has_color = color_of(target, shape, c);
    TopoDS_Shape placed = shape.Moved(loc);
    for (TopExp_Explorer ex(placed, TopAbs_SOLID); ex.More(); ex.Next())
      emit(ex.Current(), name, has_color, c);
  }
};

} // namespace

rust::Vec<StepBody> read_step(rust::Slice<const uint8_t> bytes, uint32_t max_bodies) {
  Handle(TDocStd_Document) doc = new_xcaf_document();
  step_static_params();
  STEPCAFControl_Reader reader;
  reader.SetNameMode(Standard_True);
  reader.SetColorMode(Standard_True);
  reader.SetLayerMode(Standard_False);
  reader.SetPropsMode(Standard_False);
  std::istringstream in(std::string(reinterpret_cast<const char *>(bytes.data()), bytes.size()),
                        std::ios::binary);
  require(reader.ChangeReader().ReadStream("input.step", in) == IFSelect_RetDone,
          "OpenCascade could not read the STEP file");
  require(reader.Transfer(doc), "OpenCascade could not transfer the STEP file");

  StepCollector c{XCAFDoc_DocumentTool::ShapeTool(doc->Main()),
                  XCAFDoc_DocumentTool::ColorTool(doc->Main()), max_bodies, {}};
  TDF_LabelSequence roots;
  c.shapes->GetFreeShapes(roots);
  for (int i = 1; i <= roots.Length(); ++i)
    c.visit(roots.Value(i), TopLoc_Location(), std::string(), 0);
  XCAFApp_Application::GetApplication()->Close(doc);
  return std::move(c.out);
}

struct StepWriter::Impl {
  Handle(TDocStd_Document) doc;
};

StepWriter::StepWriter() : impl(std::make_unique<Impl>()) {}
StepWriter::~StepWriter() {
  if (impl && !impl->doc.IsNull()) {
    try {
      XCAFApp_Application::GetApplication()->Close(impl->doc);
    } catch (...) {
    }
  }
}

std::unique_ptr<StepWriter> step_writer_new() {
  auto w = std::make_unique<StepWriter>();
  w->impl->doc = new_xcaf_document();
  return w;
}

void step_writer_add(StepWriter &w, const Shape &s, rust::Str name, bool has_color, double r, double g,
                     double b) {
  require_shape(s.shape, "step_writer_add");
  Handle(XCAFDoc_ShapeTool) shapes = XCAFDoc_DocumentTool::ShapeTool(w.impl->doc->Main());
  TDF_Label label = shapes->AddShape(s.shape, Standard_False);
  require(!label.IsNull(), "could not add shape to the STEP document");
  TDataStd_Name::Set(label, TCollection_ExtendedString(std::string(name).c_str(), Standard_True));
  if (has_color) {
    require_finite({r, g, b});
    XCAFDoc_DocumentTool::ColorTool(w.impl->doc->Main())
        ->SetColor(label, Quantity_Color(r, g, b, Quantity_TOC_sRGB), XCAFDoc_ColorSurf);
  }
}

void step_writer_write(StepWriter &w, rust::Str path) {
  // The schema is read when the writer's model is created.
  step_static_params();
  STEPCAFControl_Writer writer;
  writer.SetNameMode(Standard_True);
  writer.SetColorMode(Standard_True);
  require(writer.Transfer(w.impl->doc, STEPControl_AsIs), "could not transfer shapes to STEP");
  require(writer.Write(std::string(path).c_str()) == IFSelect_RetDone, "could not write the STEP file");
}

} // namespace smocct

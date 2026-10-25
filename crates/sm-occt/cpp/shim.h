// Thin C++ layer between Rust and OpenCascade.
//
// Rules: only opaque handles (Shape, OpResult, StepWriter) cross into Rust,
// and every function that touches OpenCascade is declared returning Result
// on the Rust side, so the trycatch override below turns any
// Standard_Failure or std::exception into a Rust error instead of
// unwinding through Rust frames.

#pragma once

#include "rust/cxx.h"

#include <Standard_Failure.hxx>
#include <TopoDS_Shape.hxx>

#include <array>
#include <cstdint>
#include <exception>
#include <memory>
#include <string>
#include <vector>

namespace rust {
namespace behavior {
template <typename Try, typename Fail>
static void trycatch(Try &&func, Fail &&fail) noexcept try {
  func();
} catch (const Standard_Failure &e) {
  const char *msg = e.GetMessageString();
  std::string text = std::string(e.DynamicType()->Name()) +
                     ((msg && *msg) ? std::string(": ") + msg : std::string());
  fail(text);
} catch (const std::exception &e) {
  fail(e.what());
} catch (...) {
  fail("unknown C++ exception");
}
} // namespace behavior
} // namespace rust

namespace smocct {

struct HistEntry;
struct MassProps;
struct BoundingBox;
struct FaceInfo;
struct MeshData;
struct StepBody;

class Shape {
public:
  explicit Shape(TopoDS_Shape s) : shape(std::move(s)) {}
  TopoDS_Shape shape;
};

// The outcome of a modelling operation: the result shape, its history
// relative to the arguments, and per-operation extras.
class OpResult {
public:
  TopoDS_Shape shape;
  // (arg, kind, index, relation, result kind, result index); see HistEntry.
  std::vector<std::array<uint32_t, 6>> history;
  TopoDS_Shape first; // prism / revolve start cap, may be null
  TopoDS_Shape last;  // end cap, may be null
  std::vector<uint32_t> extra;
};

// Collects named, coloured shapes into an XCAF document for STEP export.
class StepWriter {
public:
  StepWriter();
  ~StepWriter();
  struct Impl;
  std::unique_ptr<Impl> impl;
};

rust::String occt_version();

std::unique_ptr<Shape> shape_clone(const Shape &s);
bool shape_is_null(const Shape &s);
uint8_t shape_type(const Shape &s);
uint32_t shape_count(const Shape &s, uint8_t kind);
bool shape_same(const Shape &s, uint8_t kind, uint32_t i, const Shape &t, uint8_t tkind, uint32_t j);
std::unique_ptr<Shape> shape_sub(const Shape &s, uint8_t kind, uint32_t index);

std::unique_ptr<Shape> make_box(double dx, double dy, double dz);
std::unique_ptr<Shape> make_cylinder(double r, double h);
std::unique_ptr<OpResult> polygon_face(rust::Slice<const double> points,
                                       rust::Slice<const uint32_t> loop_lens);
std::unique_ptr<OpResult> prism(const Shape &profile, double dx, double dy, double dz);
std::unique_ptr<OpResult> revolve(const Shape &profile, double ox, double oy, double oz,
                                  double dx, double dy, double dz, double angle);
std::unique_ptr<OpResult> boolean_op(uint8_t op, const Shape &a, const Shape &b);
std::unique_ptr<OpResult> fillet(const Shape &s, rust::Slice<const uint32_t> edges, double radius);
std::unique_ptr<OpResult> chamfer(const Shape &s, rust::Slice<const uint32_t> edges, double distance);
std::unique_ptr<OpResult> shell(const Shape &s, rust::Slice<const uint32_t> faces, double thickness,
                                double tolerance);
std::unique_ptr<OpResult> draft(const Shape &s, rust::Slice<const uint32_t> faces, double dx, double dy,
                                double dz, double angle, double px, double py, double pz, double nx,
                                double ny, double nz);
std::unique_ptr<OpResult> unify_same_domain(const Shape &s);

std::unique_ptr<Shape> op_shape(const OpResult &r);
rust::Vec<HistEntry> op_history(const OpResult &r);
std::unique_ptr<Shape> op_first(const OpResult &r);
std::unique_ptr<Shape> op_last(const OpResult &r);
rust::Vec<uint32_t> op_extra(const OpResult &r);

bool check_valid(const Shape &s);
MassProps mass_props(const Shape &s);
BoundingBox bounding_box(const Shape &s);
FaceInfo face_info(const Shape &s, uint32_t index);
rust::Vec<double> edge_ends(const Shape &s, uint32_t index);
MeshData tessellate(const Shape &s, double linear, double angular);

rust::Vec<uint8_t> to_brep(const Shape &s);
std::unique_ptr<Shape> from_brep(rust::Slice<const uint8_t> bytes);

rust::Vec<StepBody> read_step(rust::Slice<const uint8_t> bytes, uint32_t max_bodies);
std::unique_ptr<StepWriter> step_writer_new();
void step_writer_add(StepWriter &w, const Shape &s, rust::Str name, bool has_color, double r, double g,
                     double b);
void step_writer_write(StepWriter &w, rust::Str path);

} // namespace smocct

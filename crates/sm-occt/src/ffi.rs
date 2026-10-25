//! The raw cxx bridge. Everything here is wrapped by safe types in the
//! crate root; nothing outside this crate sees these items.

#[cxx::bridge(namespace = "smocct")]
pub(crate) mod ffi {
    /// One history record: argument `arg`'s sub-shape (`kind`, `index`)
    /// relates to the result's sub-shape (`result_kind`, `result_index`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct HistEntry {
        arg: u8,
        kind: u8,
        index: u32,
        relation: u8,
        result_kind: u8,
        result_index: u32,
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct MassProps {
        volume: f64,
        area: f64,
        cx: f64,
        cy: f64,
        cz: f64,
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct BoundingBox {
        xmin: f64,
        ymin: f64,
        zmin: f64,
        xmax: f64,
        ymax: f64,
        zmax: f64,
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct FaceInfo {
        kind: u8,
        ox: f64,
        oy: f64,
        oz: f64,
        nx: f64,
        ny: f64,
        nz: f64,
        xmin: f64,
        ymin: f64,
        zmin: f64,
        xmax: f64,
        ymax: f64,
        zmax: f64,
        area: f64,
    }

    #[derive(Debug, Clone, Default)]
    struct MeshData {
        positions: Vec<f64>,
        normals: Vec<f64>,
        indices: Vec<u32>,
        face_ranges: Vec<u32>,
    }

    #[derive(Debug, Clone)]
    struct StepBody {
        name: String,
        has_color: bool,
        r: f64,
        g: f64,
        b: f64,
        brep: Vec<u8>,
    }

    unsafe extern "C++" {
        include!("sm-occt/cpp/shim.h");

        type Shape;
        type OpResult;
        type StepWriter;

        fn occt_version() -> String;

        fn shape_clone(s: &Shape) -> UniquePtr<Shape>;
        fn shape_is_null(s: &Shape) -> bool;
        fn shape_type(s: &Shape) -> Result<u8>;
        fn shape_count(s: &Shape, kind: u8) -> Result<u32>;
        fn shape_same(s: &Shape, kind: u8, i: u32, t: &Shape, tkind: u8, j: u32) -> Result<bool>;
        fn shape_sub(s: &Shape, kind: u8, index: u32) -> Result<UniquePtr<Shape>>;

        fn make_box(dx: f64, dy: f64, dz: f64) -> Result<UniquePtr<Shape>>;
        fn make_cylinder(r: f64, h: f64) -> Result<UniquePtr<Shape>>;
        fn polygon_face(points: &[f64], loop_lens: &[u32]) -> Result<UniquePtr<OpResult>>;
        fn prism(profile: &Shape, dx: f64, dy: f64, dz: f64) -> Result<UniquePtr<OpResult>>;
        #[allow(clippy::too_many_arguments)]
        fn revolve(
            profile: &Shape,
            ox: f64,
            oy: f64,
            oz: f64,
            dx: f64,
            dy: f64,
            dz: f64,
            angle: f64,
        ) -> Result<UniquePtr<OpResult>>;
        fn boolean_op(op: u8, a: &Shape, b: &Shape) -> Result<UniquePtr<OpResult>>;
        fn fillet(s: &Shape, edges: &[u32], radius: f64) -> Result<UniquePtr<OpResult>>;
        fn chamfer(s: &Shape, edges: &[u32], distance: f64) -> Result<UniquePtr<OpResult>>;
        fn shell(
            s: &Shape,
            faces: &[u32],
            thickness: f64,
            tolerance: f64,
        ) -> Result<UniquePtr<OpResult>>;
        #[allow(clippy::too_many_arguments)]
        fn draft(
            s: &Shape,
            faces: &[u32],
            dx: f64,
            dy: f64,
            dz: f64,
            angle: f64,
            px: f64,
            py: f64,
            pz: f64,
            nx: f64,
            ny: f64,
            nz: f64,
        ) -> Result<UniquePtr<OpResult>>;
        fn unify_same_domain(s: &Shape) -> Result<UniquePtr<OpResult>>;

        fn op_shape(r: &OpResult) -> UniquePtr<Shape>;
        fn op_history(r: &OpResult) -> Vec<HistEntry>;
        fn op_first(r: &OpResult) -> UniquePtr<Shape>;
        fn op_last(r: &OpResult) -> UniquePtr<Shape>;
        fn op_extra(r: &OpResult) -> Vec<u32>;

        fn check_valid(s: &Shape) -> Result<bool>;
        fn mass_props(s: &Shape) -> Result<MassProps>;
        fn bounding_box(s: &Shape) -> Result<BoundingBox>;
        fn face_info(s: &Shape, index: u32) -> Result<FaceInfo>;
        fn edge_ends(s: &Shape, index: u32) -> Result<Vec<f64>>;
        fn tessellate(s: &Shape, linear: f64, angular: f64) -> Result<MeshData>;

        fn to_brep(s: &Shape) -> Result<Vec<u8>>;
        fn from_brep(bytes: &[u8]) -> Result<UniquePtr<Shape>>;

        fn read_step(bytes: &[u8], max_bodies: u32) -> Result<Vec<StepBody>>;
        fn step_writer_new() -> Result<UniquePtr<StepWriter>>;
        fn step_writer_add(
            w: Pin<&mut StepWriter>,
            s: &Shape,
            name: &str,
            has_color: bool,
            r: f64,
            g: f64,
            b: f64,
        ) -> Result<()>;
        fn step_writer_write(w: Pin<&mut StepWriter>, path: &str) -> Result<()>;
    }
}

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(shape) = sm_occt::Shape::load_brep(data) {
        // What loads must also be usable.
        let _ = shape.shape_type();
        let _ = shape.faces();
    }
});

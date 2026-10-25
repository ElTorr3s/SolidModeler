//! Property tests for the rebuild. Documents in files come from anyone, so a
//! rebuild of any valid document must return a model or an error, never
//! panic, and every body it returns must be a closed solid.

use proptest::prelude::*;
use sm_kernel::profile::{regions, signed_area};
use sm_testkit::strategies::{buildable_document, document, profile};
use truck_topology::shell::ShellCondition;

fn all_closed(model: &sm_kernel::Model) -> bool {
    model.bodies.iter().all(|b| {
        b.solid
            .boundaries()
            .iter()
            .all(|s| s.shell_condition() == ShellCondition::Closed)
    })
}

proptest! {
    // Booleans are slow in debug builds, so the default is modest; CI's
    // release run raises it through PROPTEST_CASES.
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(64)
    ))]

    /// Any sketch geometry, however broken, gives regions or an error, and
    /// regions are always oriented: outer loops counter-clockwise, holes
    /// clockwise.
    #[test]
    fn regions_are_oriented(loops in prop::collection::vec(profile(), 0..4)) {
        if let Ok(regions) = regions(&loops) {
            for r in regions {
                prop_assert!(signed_area(&r.outer) > 0.0);
                for h in &r.holes {
                    prop_assert!(signed_area(h) < 0.0);
                }
            }
        }
    }

    /// Rebuilding a document with arbitrary sketches never panics.
    #[test]
    fn rebuild_never_panics(doc in document()) {
        let _ = sm_kernel::rebuild(&doc);
    }

    /// Rebuilds of mostly-buildable documents only ever produce closed solids.
    #[test]
    fn rebuilt_bodies_are_closed(doc in buildable_document()) {
        if let Ok(model) = sm_kernel::rebuild(&doc) {
            prop_assert!(all_closed(&model));
        }
    }
}

//! Property tests for the document model.

use proptest::prelude::*;
use sm_document::PartDocument;
use sm_testkit::strategies::document;

proptest! {
    /// Documents built through the public API always validate.
    #[test]
    fn built_documents_validate(doc in document()) {
        prop_assert_eq!(doc.validate(), Ok(()));
    }

    /// `document.json` must be lossless: every value, including every bit of
    /// every coordinate, survives a round trip.
    #[test]
    fn json_round_trip_is_exact(doc in document()) {
        let json = serde_json::to_string_pretty(&doc).unwrap();
        let back: PartDocument = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(back, doc);
    }

    /// `active_features` never yields more than the history holds, and never
    /// anything suppressed.
    #[test]
    fn active_features_are_a_filtered_prefix(doc in document()) {
        let active: Vec<_> = doc.active_features().collect();
        prop_assert!(active.len() <= doc.features.len());
        prop_assert!(active.iter().all(|f| !f.suppressed));
        let end = doc
            .end_of_part
            .and_then(|id| doc.features.iter().position(|f| f.id == id))
            .unwrap_or(doc.features.len());
        prop_assert!(active
            .iter()
            .all(|f| doc.features[..end].iter().any(|g| g.id == f.id)));
    }

    /// New ids never collide with existing ones, even after edits that
    /// remove or reorder features.
    #[test]
    fn added_ids_are_fresh(mut doc in document(), drop in any::<prop::sample::Index>()) {
        if !doc.features.is_empty() {
            let i = drop.index(doc.features.len());
            doc.features.remove(i);
            doc.end_of_part = None;
        }
        let ids: Vec<_> = doc.features.iter().map(|f| f.id).collect();
        let new = doc.add_feature(
            "New",
            sm_document::FeatureKind::Sketch(sm_document::Sketch {
                plane: sm_document::OriginPlane::XY,
                offset: 0.0,
                profiles: vec![],
            }),
        );
        prop_assert!(!ids.contains(&new));
    }
}

use include_dir::{Dir, include_dir};

static PRESETS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../presets");

pub fn load_presets() -> Vec<(String, String)> {
    let mut files: Vec<_> = PRESETS_DIR
        .files()
        .filter(|f| f.path().extension().and_then(|e| e.to_str()) == Some("toml"))
        .collect();
    files.sort_by_key(|f| f.path());
    files
        .into_iter()
        .filter_map(|f| {
            let label = f.path().display().to_string();
            Some((label, f.contents_utf8()?.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use lsystem_core::{
        AnyCompiledGeneration, BoundsAccumulator, BoundsAccumulator2D, BoundsAccumulator3D, D2, D3,
        DEFAULT_TEMPLATE_SEGMENT_BUDGET, Dimensions, GenerationConfig, PreparedGeneration,
    };

    use super::load_presets;
    use crate::{ConfigDefaults, ConfigDocument, ConfigSource};

    #[test]
    fn load_presets_valid_non_empty_and_sorted() {
        let presets = load_presets();
        assert!(!presets.is_empty());
        let mut prev_label: Option<&str> = None;
        for (label, text) in &presets {
            assert!(
                label.ends_with(".toml"),
                "label should end with .toml: {label}"
            );
            assert!(!text.is_empty(), "preset {label} should not be empty");
            if let Some(prev) = prev_label {
                assert!(
                    prev <= label.as_str(),
                    "presets not sorted: {prev} > {label}"
                );
            }
            prev_label = Some(label.as_str());
        }
    }

    fn assert_stamped_2d_bounds(label: &str, generation: lsystem_core::CompiledGeneration<D2>) {
        let prepared = generation
            .plan_templates(DEFAULT_TEMPLATE_SEGMENT_BUDGET)
            .prepare();
        assert!(
            matches!(prepared, PreparedGeneration::Stamped(_)),
            "{label}: fixture must select stamped generation"
        );
        let mut exact = BoundsAccumulator2D::default();
        let candidates = prepared
            .segments()
            .drain(|[start, end]| exact.include_segment(start, end))
            .expect("non-empty stamped scene");
        let exact = exact.finish().expect("non-empty stamped scene");
        assert!(
            candidates.min.x <= exact.min.x
                && candidates.min.y <= exact.min.y
                && candidates.max.x >= exact.max.x
                && candidates.max.y >= exact.max.y,
            "{label}: candidate rectangle must contain exact bounds"
        );
    }

    fn assert_stamped_3d_bounds(label: &str, generation: lsystem_core::CompiledGeneration<D3>) {
        let prepared = generation
            .plan_templates(DEFAULT_TEMPLATE_SEGMENT_BUDGET)
            .prepare();
        assert!(
            matches!(prepared, PreparedGeneration::Stamped(_)),
            "{label}: fixture must select stamped generation"
        );
        let mut exact = BoundsAccumulator3D::default();
        let mut endpoints = Vec::new();
        let candidates = prepared
            .segments()
            .drain(|[start, end]| {
                for point in [start, end] {
                    exact.include(point);
                    endpoints.push(point);
                }
            })
            .expect("non-empty stamped scene");
        for point in endpoints {
            let horizontal_distance =
                (point.x - candidates.center_xz.x).hypot(point.z - candidates.center_xz.y);
            assert!(
                horizontal_distance <= candidates.radius
                    && point.y >= candidates.min_y
                    && point.y <= candidates.max_y,
                "{label}: candidate cylinder must contain every endpoint"
            );
        }
        exact.finish().expect("non-empty stamped scene");
    }

    fn assert_stamped_bounds(label: &str, generation: &GenerationConfig) {
        match generation.compile() {
            AnyCompiledGeneration::TwoD(generation) => assert_stamped_2d_bounds(label, generation),
            AnyCompiledGeneration::ThreeD(generation) => {
                assert_stamped_3d_bounds(label, generation)
            }
        }
    }

    #[test]
    fn stamped_summary_bounds_contain_preset_and_seeded_grammar_geometry() {
        for (label, text) in load_presets() {
            let source = ConfigSource::parse(&text).expect("bundled TOML parses");
            let document = ConfigDocument::try_from(source).expect("bundled preset validates");
            let config = document
                .editor_config()
                .resolve(ConfigDefaults::embedded(), u16::MAX);
            assert_stamped_bounds(&label, &config.generation);
        }

        // A single diagonal local template is deliberately awkward for AABB
        // corners after a rigid placement: phantom corners can add horizontal
        // radius even when the true geometry is nearly axis-aligned.
        let thin_diagonal_3d = GenerationConfig::new(
            Dimensions::ThreeD,
            "A".to_string(),
            2,
            45.0,
            1.0,
            0.0,
            BTreeMap::from([('A', "+&FFFF".to_string())]),
        )
        .expect("thin diagonal grammar is balanced");
        assert_stamped_bounds("thin-diagonal-3d", &thin_diagonal_3d);

        // The 2D analog, awkward for the 2D directional support table in the
        // same way: four collinear segments at 45 degrees, so the local AABB
        // is a square whose off-diagonal corners are phantoms.
        let thin_diagonal_2d = GenerationConfig::new(
            Dimensions::TwoD,
            "A".to_string(),
            2,
            45.0,
            1.0,
            0.0,
            BTreeMap::from([('A', "+FFFF".to_string())]),
        )
        .expect("thin diagonal grammar is balanced");
        assert_stamped_bounds("thin-diagonal-2d", &thin_diagonal_2d);

        // The seeded 2D grammars below turn by `22.5 + n` degrees for integer
        // `n`, which is never a multiple of 90, so their stamps are placed at
        // non-axis-aligned rotations and exercise the support table's sector
        // lookup rather than its exact generator directions. Sector coverage
        // itself is asserted in `lsystem-core`'s template tests, where stamp
        // rotations are reachable.
        let mut seed = 0x5EED_CAFE_u64;
        for index in 0..16 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let two_d_rhs = ["F+AF", "F-AF", "F[+A]F", "F[-A]F"][((seed >> 33) as usize) % 4];
            let two_d = GenerationConfig::new(
                Dimensions::TwoD,
                "A".to_string(),
                4,
                22.5 + ((seed >> 8) % 90) as f32,
                1.0,
                0.0,
                BTreeMap::from([('A', two_d_rhs.to_string())]),
            )
            .expect("seeded 2D grammar is balanced");
            assert_stamped_bounds(&format!("seeded-2d-{index}"), &two_d);

            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let three_d_rhs = [r"F+AF", r"F&AF", r"F/AF", r"F[^A]F"][((seed >> 33) as usize) % 4];
            let three_d = GenerationConfig::new(
                Dimensions::ThreeD,
                "A".to_string(),
                4,
                18.0 + ((seed >> 8) % 90) as f32,
                1.0,
                0.0,
                BTreeMap::from([('A', three_d_rhs.to_string())]),
            )
            .expect("seeded 3D grammar is balanced");
            assert_stamped_bounds(&format!("seeded-3d-{index}"), &three_d);
        }
    }
}

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
    use super::load_presets;

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
}

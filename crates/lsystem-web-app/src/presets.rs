use include_dir::{Dir, include_dir};
use lsystem_core::Config;

static PRESETS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../presets");

pub(crate) fn load_presets() -> Vec<(String, String)> {
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

pub(crate) fn max_iterations_for_config(config: &Config) -> u32 {
    let generation = &config.generation;
    let max_seg = lsystem_renderer::line_renderer::max_segments_for_line_color(
        generation.dimensions,
        &config.colors.line,
    );
    lsystem_core::max_safe_iterations(&generation.axiom, &generation.rules, max_seg)
}

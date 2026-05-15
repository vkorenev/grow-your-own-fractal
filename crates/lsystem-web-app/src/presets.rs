use include_dir::{Dir, include_dir};
use lsystem_core::Config;

static PRESETS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../presets");

pub(crate) fn load_presets() -> Vec<(String, &'static str)> {
    let mut files: Vec<_> = PRESETS_DIR
        .files()
        .filter(|f| f.path().extension().and_then(|e| e.to_str()) == Some("toml"))
        .collect();
    files.sort_by_key(|f| f.path());
    files
        .into_iter()
        .filter_map(|f| {
            let content = f.contents_utf8()?;
            let name = match Config::parse(content) {
                Ok(config) => config.name,
                Err(err) => {
                    log::error!("Bundled preset {:?} failed to parse: {err}", f.path());
                    return None;
                }
            };
            Some((name, content))
        })
        .collect()
}

pub(crate) fn max_iterations_for_config(config: &Config) -> u32 {
    let max_seg = if config.dimensions == 3 {
        lsystem_renderer::line_renderer::MAX_SEGMENTS_3D
    } else {
        lsystem_renderer::line_renderer::MAX_SEGMENTS
    };
    lsystem_core::max_safe_iterations(&config.generation.axiom, &config.generation.rules, max_seg)
}

pub(crate) fn effective_config(
    config: Option<Config>,
    iterations: u32,
    angle: f32,
) -> Option<Config> {
    config.map(|mut config| {
        config.generation.iterations = iterations;
        config.generation.angle = angle;
        config
    })
}

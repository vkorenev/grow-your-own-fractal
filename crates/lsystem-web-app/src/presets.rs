use include_dir::{Dir, include_dir};
use lsystem_core::Config;

static PRESETS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../presets");

pub(crate) struct AppliedConfig {
    pub(crate) config: Config,
    pub(crate) max_iterations: u32,
}

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
            let name = Config::parse(content).ok()?.name;
            Some((name, content))
        })
        .collect()
}

pub(crate) fn apply_toml(text: &str) -> Result<AppliedConfig, lsystem_core::ConfigError> {
    let config = Config::parse(text)?;
    let max_iterations = lsystem_core::max_safe_iterations(
        &config.axiom,
        &config.rules,
        lsystem_renderer::line_renderer::MAX_SEGMENTS,
    );
    Ok(AppliedConfig {
        config,
        max_iterations,
    })
}

pub(crate) fn effective_config(
    config: Option<Config>,
    iterations: u32,
    angle: f32,
) -> Option<Config> {
    config.map(|mut config| {
        config.iterations = iterations;
        config.angle = angle;
        config
    })
}

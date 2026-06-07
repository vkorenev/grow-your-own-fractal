use lsystem_core::EditorGenerationConfig;

pub(crate) fn max_iterations_for_editor_config(generation: &EditorGenerationConfig) -> u32 {
    let max_seg = lsystem_renderer::line_renderer::max_segments_for_line_color(
        generation.dimensions,
        generation.axiom.contains('[') || generation.rules.values().any(|rhs| rhs.contains('[')),
    );
    lsystem_core::max_safe_iterations(&generation.axiom, &generation.rules, max_seg)
}

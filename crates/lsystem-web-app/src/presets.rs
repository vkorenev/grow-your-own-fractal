use lsystem_core::EditorGenerationConfig;

pub(crate) fn max_iterations_for_editor_config(generation: &EditorGenerationConfig) -> u32 {
    let max_seg = lsystem_renderer::line_renderer::max_segments_for_line_color(
        generation.dimensions,
        generation.has_stack_directives(),
    );
    lsystem_core::max_safe_iterations(&generation.axiom, &generation.rules, max_seg)
}

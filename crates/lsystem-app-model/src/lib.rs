pub(crate) mod animation;
pub(crate) mod color;
pub(crate) mod config_workspace;
pub(crate) mod presets;
pub(crate) mod util;

pub use animation::{
    HUE_ROTATION_MAX_SPEED_DEGREES_PER_SECOND, HUE_ROTATION_MIN_SPEED_DEGREES_PER_SECOND,
    HueRotation, HueRotationDirection, advance_hue_rotation_phase_degrees,
};
pub use color::{
    ColorControlMemory, LineColorMode, line_color_for_controls, line_color_for_render,
    selected_line_color_mode,
};
pub use config_workspace::{
    CleanMut, ConfigEntry, ConfigWorkspace, ConfigWorkspaceError, DirtyMut, EntryViewMut,
};
pub use presets::load_presets;
pub use util::sanitize_filename;

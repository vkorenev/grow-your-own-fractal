pub(crate) mod animation;
pub(crate) mod color;
pub(crate) mod config_workspace;
pub(crate) mod presets;
pub(crate) mod util;

pub use animation::{
    HSV_MOVEMENT_MAX_SPEED_DEGREES_PER_SECOND, HSV_MOVEMENT_MIN_SPEED_DEGREES_PER_SECOND,
    HsvMovement, HsvMovementDirection, advance_hsv_phase_degrees,
};
pub use color::{ColorControlMemory, LineColorMode};
pub use config_workspace::{
    CleanMut, ConfigEntry, ConfigWorkspace, ConfigWorkspaceError, DirtyMut, EntryViewMut,
};
pub use presets::load_presets;
pub use util::{hex_to_rgb, rgb_to_hex, sanitize_filename};

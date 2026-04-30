pub struct InputState {
    pub cursor_pos: [f32; 2],
    drag_active: bool,
    last_drag_pos: [f32; 2],
}

impl InputState {
    pub fn new() -> Self {
        Self {
            cursor_pos: [0.0; 2],
            drag_active: false,
            last_drag_pos: [0.0; 2],
        }
    }

    /// Returns the screen-space drag delta `[dx, dy]` if a drag is in progress.
    pub fn on_cursor_moved(&mut self, x: f32, y: f32) -> Option<[f32; 2]> {
        self.cursor_pos = [x, y];
        if self.drag_active {
            let delta = [x - self.last_drag_pos[0], y - self.last_drag_pos[1]];
            self.last_drag_pos = [x, y];
            Some(delta)
        } else {
            None
        }
    }

    pub fn on_left_button(&mut self, pressed: bool) {
        if pressed {
            self.last_drag_pos = self.cursor_pos;
        }
        self.drag_active = pressed;
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

use glam::Vec2;

/// Line-segment geometry produced by a turtle interpreter.
pub enum Geometry {
    D2 { segments: Vec<[Vec2; 2]> },
}

impl Geometry {
    pub fn segment_count(&self) -> usize {
        let Geometry::D2 { segments } = self;
        segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segment_count() == 0
    }
}

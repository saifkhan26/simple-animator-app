//! Layer = stack of cell exposures across the timeline.
//!
//! `exposures[frame]` decides what shows on that frame for this layer:
//!   * `Some(cell_id)` = a key — the cell with that id appears.
//!   * `None`          = hold — the previous non-None entry continues to show.
//!
//! Drawing on a frame that resolves to a held cell modifies the shared cell
//! (animator convention). To break a hold, the user inserts a new key via the
//! X-sheet panel.

pub type CellId = usize;

pub struct Layer {
    pub name: String,
    pub opacity: f32,
    pub visible: bool,
    pub locked: bool,
    /// If true, this layer renders at a fixed dim opacity and is excluded
    /// from final export — used as a light-table / reference layer.
    pub reference: bool,
    pub exposures: Vec<Option<CellId>>,
}

impl Layer {
    pub fn new(name: impl Into<String>, frames: usize) -> Self {
        Self {
            name: name.into(),
            opacity: 1.0,
            visible: true,
            locked: false,
            reference: false,
            exposures: vec![None; frames.max(1)],
        }
    }

    /// Returns the resolved CellId showing on `frame`, walking back through
    /// holds.
    pub fn resolve(&self, frame: usize) -> Option<CellId> {
        if self.exposures.is_empty() {
            return None;
        }
        let f = frame.min(self.exposures.len() - 1);
        for i in (0..=f).rev() {
            if let Some(id) = self.exposures[i] {
                return Some(id);
            }
        }
        None
    }

    pub fn set_key(&mut self, frame: usize, cell: CellId) {
        if frame < self.exposures.len() {
            self.exposures[frame] = Some(cell);
        }
    }

    pub fn hold(&mut self, frame: usize) {
        if frame < self.exposures.len() {
            self.exposures[frame] = None;
        }
    }
}

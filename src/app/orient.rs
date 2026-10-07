//! Mirroring and turning the art itself — the selection, the drawing, or
//! every drawing on a layer. The view flip only changes how the canvas is
//! shown; these change the pixels, losslessly (see [`crate::doc::orient`]).
//!
//! A child of `app` for the same reason as `select`: it works on
//! `AppState`'s private state directly.

use std::sync::Arc;

use super::AppState;
use crate::doc::orient::Orient;

impl AppState {
    /// Flip or turn what the selection says to. A float, or a selection with
    /// edges, has its pixels turned in place about its own centre. Otherwise
    /// — nothing selected, or Select All — the current drawing is, about the
    /// cell's centre, or every drawing on the layer with the Lasso's "All
    /// frames" on.
    pub fn orient_art(&mut self, o: Orient) {
        let bounded = self.sel_mask.as_ref().is_some_and(|m| m.outside == 0 && !m.is_all());
        if self.selection.is_some() || bounded {
            self.orient_selection(o);
            return;
        }
        let li = self.project.current_layer;
        self.orient_drawings(li, o, self.sel_all_frames);
    }

    /// Turn the current drawing of layer `li` over, or with `every` each of
    /// its drawings, about the cell's centre: the canvas centre for a layer
    /// that hasn't been moved, so a flipped walk cycle stays registered. One
    /// undo step. A quarter turn of a non-square drawing swaps its width and
    /// height rather than cropping it, so turning back is exact; turning a
    /// whole layer swaps the size its new drawings get too.
    pub fn orient_drawings(&mut self, li: usize, o: Orient, every: bool) {
        if every {
            self.orient_layers(&[li], o);
            return;
        }
        let Some(layer) = self.project.layers.get(li) else {
            return;
        };
        if !self.project.paintable(li) || o.is_identity() {
            return;
        }
        let cells: Vec<_> = layer.resolve(self.project.current_frame).into_iter().collect();
        self.orient_cells(cells, &[], o);
    }

    /// Every drawing on each of `layers` — a group's layers, say — turned
    /// over as [`AppState::orient_drawings`] does a whole layer, in one undo
    /// step. Locked and reference layers are left alone.
    pub fn orient_layers(&mut self, layers: &[usize], o: Orient) {
        if o.is_identity() {
            return;
        }
        let layers: Vec<usize> = layers.iter().copied().filter(|&li| self.project.paintable(li)).collect();
        let mut cells = Vec::new();
        for &li in &layers {
            for (c, _) in self.layer_drawings(li) {
                if !cells.contains(&c) {
                    cells.push(c);
                }
            }
        }
        let resized = if o.swaps_axes() { layers } else { Vec::new() };
        self.orient_cells(cells, &resized, o);
    }

    /// Turn `cells` over, and swap the cell size `resized` layers give their
    /// new drawings. One undo step.
    fn orient_cells(&mut self, cells: Vec<crate::doc::layer::CellId>, resized: &[usize], o: Orient) {
        let turned: Vec<_> = cells
            .into_iter()
            .filter_map(|c| Some((c, Arc::new(self.project.cell(c)?.oriented(o)))))
            .collect();
        if turned.is_empty() {
            return;
        }
        self.structural_edit(true, |p| {
            for (c, canvas) in turned {
                p.cells[c] = canvas;
            }
            let (pw, ph) = (p.width, p.height);
            for &li in resized {
                let l = &mut p.layers[li];
                let (w, h) = l.cell_size(pw, ph);
                // Back to "the frame size" when that is what it is again.
                (l.cell_w, l.cell_h) = if (h, w) == (pw, ph) { (0, 0) } else { (h, w) };
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [200, 0, 0, 255];

    /// Paint `x0..x1, y0..y1` of `cell` outside the history.
    fn paint(st: &mut AppState, cell: crate::doc::layer::CellId, (x0, y0, x1, y1): (u32, u32, u32, u32)) {
        let c = st.project.cell_mut(cell).unwrap();
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * c.width + x) * 4) as usize;
                c.pixels_mut()[i..i + 4].copy_from_slice(&RED);
            }
        }
    }

    fn alpha(st: &AppState, cell: crate::doc::layer::CellId, x: u32, y: u32) -> u8 {
        st.project.cell(cell).unwrap().px(x, y)[3]
    }

    /// A blob near the top-left of the current drawing.
    fn blob() -> (AppState, crate::doc::layer::CellId) {
        let mut st = AppState::for_test();
        let cell = st.project.ensure_active_cell();
        paint(&mut st, cell, (10, 20, 30, 25));
        (st, cell)
    }

    #[test]
    fn flipping_the_drawing_is_one_undo_step() {
        let (mut st, cell) = blob();
        let w = st.project.cell(cell).unwrap().width;
        let steps = st.history.undo_len();
        st.orient_art(Orient::FLIP_H);
        assert_eq!(alpha(&st, cell, 15, 22), 0);
        assert_eq!(alpha(&st, cell, w - 1 - 15, 22), 255, "mirrored about the centre");
        assert_eq!(st.history.undo_len(), steps + 1);
        st.undo();
        assert_eq!(alpha(&st, cell, 15, 22), 255);
        assert_eq!(alpha(&st, cell, w - 1 - 15, 22), 0);
    }

    #[test]
    fn rotating_a_non_square_drawing_swaps_its_size_and_back_exactly() {
        let (mut st, cell) = blob();
        let before = st.project.cell(cell).unwrap().pixels().into_owned();
        let (w, h) = (st.project.width, st.project.height);
        assert_ne!(w, h, "the test wants a non-square frame");
        st.orient_art(Orient::CW);
        let c = st.project.cell(cell).unwrap();
        assert_eq!((c.width, c.height), (h, w));
        st.orient_art(Orient::CCW);
        let c = st.project.cell(cell).unwrap();
        assert_eq!((c.width, c.height), (w, h));
        assert_eq!(&c.pixels()[..], &before[..]);
        assert_eq!(st.project.layers[0].cell_size(w, h), (w, h), "the layer's size is untouched");
    }

    /// Every drawing on the layer turns about the same centre, so things that
    /// lined up still do.
    #[test]
    fn flipping_the_layer_keeps_registration() {
        let (mut st, first) = blob();
        st.structural_edit(false, |p| {
            p.add_frame();
            p.insert_blank_key_here();
        });
        let second = st.project.resolved_current().unwrap();
        assert_ne!(first, second);
        paint(&mut st, second, (10, 20, 30, 25));
        let w = st.project.width;
        st.orient_drawings(0, Orient::FLIP_H, true);
        for cell in [first, second] {
            assert_eq!(alpha(&st, cell, w - 1 - 15, 22), 255);
            assert_eq!(alpha(&st, cell, 15, 22), 0);
        }
        st.undo();
        for cell in [first, second] {
            assert_eq!(alpha(&st, cell, 15, 22), 255, "one step undoes both");
        }
    }

    #[test]
    fn a_locked_or_reference_layer_is_not_flipped() {
        for lock in [true, false] {
            let (mut st, cell) = blob();
            st.project.layers[0].locked = lock;
            st.project.layers[0].reference = !lock;
            let steps = st.history.undo_len();
            st.orient_art(Orient::FLIP_H);
            st.orient_drawings(0, Orient::CW, true);
            assert_eq!(alpha(&st, cell, 15, 22), 255);
            assert_eq!(st.history.undo_len(), steps);
        }
    }

    /// Turning the whole layer a quarter turns its new drawings too, and
    /// turning it back makes them frame-sized again.
    #[test]
    fn rotating_the_whole_layer_resizes_new_keys() {
        let (mut st, _) = blob();
        let (w, h) = (st.project.width, st.project.height);
        st.orient_drawings(0, Orient::CW, true);
        assert_eq!(st.project.layers[0].cell_size(w, h), (h, w));
        assert_eq!(st.project.draw_cell_size(0, 3), (h, w));
        st.orient_drawings(0, Orient::CCW, true);
        assert_eq!((st.project.layers[0].cell_w, st.project.layers[0].cell_h), (0, 0));
        st.undo();
        assert_eq!(st.project.layers[0].cell_size(w, h), (h, w), "undo restores the size");
    }
}

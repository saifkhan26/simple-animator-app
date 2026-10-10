//! Layer groups.
//!
//! The stack stays one flat list, bottom first. A group is a header row
//! ([`LayerKind::Group`], no drawings of its own), and its layers are the run
//! of `in_group` layers directly below it — children first, then the header,
//! which is the order Krita lists a group's contents in too. Groups hold
//! layers, never groups.
//!
//! Membership is never stored as a link, only as where a layer sits: a
//! parent pointer would have to be an index (and be patched on every
//! reorder) or a persisted id (which layers don't have), and contiguity is
//! needed anyway to composite and to list a group. So every reorder goes
//! through the operations here, which keep a group's layers together under
//! it, and [`Project::normalize_groups`] repairs anything that arrives
//! otherwise — a file, a Krita pull.
//!
//! [`LayerKind::Group`]: crate::doc::layer::LayerKind::Group

use std::collections::BTreeSet;
use std::ops::Range;

use crate::doc::layer::Layer;
use crate::doc::project::Project;
use crate::doc::transform::Transform;

impl Project {
    pub fn is_group(&self, li: usize) -> bool {
        self.layers.get(li).is_some_and(Layer::is_group)
    }

    /// The header of the group layer `li` is in.
    pub fn group_of(&self, li: usize) -> Option<usize> {
        if !self.layers.get(li)?.in_group {
            return None;
        }
        (li + 1..self.layers.len())
            .find(|&j| !self.layers[j].in_group)
            .filter(|&j| self.layers[j].is_group())
    }

    /// Group `g`'s layers, bottom first. Empty for anything but a header.
    pub fn children(&self, g: usize) -> Range<usize> {
        if !self.is_group(g) {
            return g..g;
        }
        let start = (0..g).rev().take_while(|&j| self.layers[j].in_group).last().unwrap_or(g);
        start..g
    }

    /// The rows that move with `li`: a group's layers and header, or the one
    /// layer.
    pub fn block(&self, li: usize) -> Range<usize> {
        if self.is_group(li) {
            self.children(li).start..li + 1
        } else {
            li..li + 1
        }
    }

    /// The rows the Layers panel lists, top first: every layer, except the
    /// layers of a folded group.
    pub fn panel_rows(&self) -> Vec<usize> {
        (0..self.layers.len())
            .rev()
            .filter(|&li| self.group_of(li).map_or(true, |g| !self.layers[g].collapsed))
            .collect()
    }

    /// Where layer `li` shows in the panel: its own row, or its folded
    /// group's.
    pub fn panel_row_of(&self, li: usize) -> usize {
        match self.group_of(li) {
            Some(g) if self.layers[g].collapsed => g,
            _ => li,
        }
    }

    /// Whether layer `li` shows: its own eye, and its group's.
    pub fn shows(&self, li: usize) -> bool {
        self.layers.get(li).is_some_and(|l| l.visible)
            && self.group_of(li).map_or(true, |g| self.layers[g].visible)
    }

    /// Locked itself, or by its group.
    pub fn locked_eff(&self, li: usize) -> bool {
        self.layers.get(li).map_or(true, |l| l.locked)
            || self.group_of(li).is_some_and(|g| self.layers[g].locked)
    }

    /// Whether layer `li`'s drawings may be edited: a drawing layer, not a
    /// reference, and not locked by itself or its group.
    pub fn paintable(&self, li: usize) -> bool {
        self.layers.get(li).is_some_and(|l| !l.is_group() && !l.reference) && !self.locked_eff(li)
    }

    /// Put the group invariants back, whatever arrived:
    /// - a header has no drawings, transform, clip or reference flag, and
    ///   isn't itself in a group;
    /// - a run of `in_group` layers has a header directly above it, or its
    ///   layers aren't in a group;
    /// - no fill reads its lines from a header, and no key jump follows one.
    pub fn normalize_groups(&mut self) {
        let frames = self.frame_count.max(1);
        for l in self.layers.iter_mut().filter(|l| l.is_group()) {
            l.in_group = false;
            l.exposures = vec![None; frames];
            l.transform = Transform::default();
            l.transform_keys.clear();
            l.track_points.clear();
            l.clip = false;
            l.reference = false;
            l.alpha_lock = false;
            l.cell_w = 0;
            l.cell_h = 0;
            l.lines_from = None;
            l.keys_from = None;
        }
        let n = self.layers.len();
        let mut j = 0;
        while j < n {
            if !self.layers[j].in_group {
                j += 1;
                continue;
            }
            let end = (j..n).find(|&k| !self.layers[k].in_group).unwrap_or(n);
            if end == n || !self.layers[end].is_group() {
                for l in &mut self.layers[j..end] {
                    l.in_group = false;
                }
            }
            j = end;
        }
        let headers: Vec<bool> = self.layers.iter().map(Layer::is_group).collect();
        let header_uids: Vec<u64> = self.layers.iter().filter(|l| l.is_group()).map(|l| l.uid).collect();
        for l in &mut self.layers {
            if l.lines_from.is_some_and(|s| headers.get(s).copied().unwrap_or(true)) {
                l.lines_from = None;
            }
            if l.keys_from.is_some_and(|u| header_uids.contains(&u)) {
                l.keys_from = None;
            }
        }
        self.current_layer = self.current_layer.min(n.saturating_sub(1));
    }

    /// Rearrange the stack: `order[i]` is the old index of the layer that
    /// ends up at `i`, and `in_group[i]` whether it is in a group there.
    /// Every reorder goes through this, so fills keep their `lines_from` and
    /// the active layer stays the active layer.
    pub fn apply_order(&mut self, order: &[usize], in_group: &[bool]) {
        debug_assert_eq!(order.len(), self.layers.len());
        debug_assert_eq!(in_group.len(), order.len());
        let mut to_new = vec![0; order.len()];
        for (new, &old) in order.iter().enumerate() {
            to_new[old] = new;
        }
        let mut old: Vec<Option<Layer>> = std::mem::take(&mut self.layers).into_iter().map(Some).collect();
        self.layers = order
            .iter()
            .map(|&o| old[o].take().expect("order is a permutation"))
            .collect();
        for (l, &g) in self.layers.iter_mut().zip(in_group) {
            l.in_group = g && !l.is_group();
        }
        self.relink(|i| to_new.get(i).copied().unwrap_or(i));
        self.current_layer = to_new.get(self.current_layer).copied().unwrap_or(0);
        debug_assert!(self.groups_ok(), "a reorder broke a group");
    }

    /// Whether the group invariants hold. For assertions and tests.
    pub fn groups_ok(&self) -> bool {
        let mut copy = self.clone();
        copy.normalize_groups();
        copy.layers.iter().zip(&self.layers).all(|(a, b)| a.in_group == b.in_group)
    }

    /// Gather `sel` — drawing layers, or whole groups — into a new group,
    /// headed where the topmost of them was. A group in `sel` gives up its
    /// layers to the new one and goes. Returns the new header's index, which
    /// becomes the active layer.
    pub fn group_layers(&mut self, sel: &[usize]) -> Option<usize> {
        let n = self.layers.len();
        let mut take = BTreeSet::new();
        let mut dissolve = BTreeSet::new();
        for &i in sel.iter().filter(|&&i| i < n) {
            if self.is_group(i) {
                dissolve.insert(i);
                take.extend(self.children(i));
            } else {
                take.insert(i);
            }
        }
        let &top = take.iter().next_back()?;
        // Inside another group, the new one goes just above that group: a
        // group never holds a group.
        let anchor = self.group_of(top).unwrap_or(top);
        let header = self.layers.len();
        // The first "Group n" nothing staying is called.
        let taken: BTreeSet<&str> = (0..n)
            .filter(|i| !dissolve.contains(i))
            .map(|i| self.layers[i].name.as_str())
            .collect();
        let name = (1..)
            .map(|k| format!("Group {k}"))
            .find(|s| !taken.contains(s.as_str()))
            .expect("a free name");
        self.layers.push(Layer::group(name, self.frame_count));

        let rest: Vec<usize> = (0..n).filter(|i| !take.contains(i) && !dissolve.contains(i)).collect();
        let anchor_stays = rest.contains(&anchor);
        let at = rest.iter().filter(|&&r| if anchor_stays { r <= anchor } else { r < anchor }).count();
        let mut order = rest.clone();
        let moved: Vec<usize> = take.iter().copied().chain([header]).collect();
        order.splice(at..at, moved);
        let flags: Vec<bool> = order
            .iter()
            .map(|&i| {
                if take.contains(&i) {
                    true
                } else if i == header {
                    false
                } else {
                    // A dissolved group's layers all moved, so whatever is
                    // left keeps its place.
                    self.layers[i].in_group
                }
            })
            .collect();
        // Removing the dissolved headers: they sit at the end once ordered.
        let mut full = order.clone();
        full.extend(dissolve.iter().copied());
        let mut full_flags = flags;
        full_flags.extend(dissolve.iter().map(|_| false));
        self.apply_order(&full, &full_flags);
        for _ in 0..dissolve.len() {
            let last = self.layers.len() - 1;
            self.layers.pop();
            self.relink_after_remove(last);
        }
        let g = order.iter().position(|&i| i == header)?;
        self.current_layer = g;
        Some(g)
    }

    /// Dissolve group `g`: its layers stay where they are, out of any group.
    pub fn ungroup(&mut self, g: usize) {
        if !self.is_group(g) {
            return;
        }
        let kids = self.children(g);
        for l in &mut self.layers[kids.clone()] {
            l.in_group = false;
        }
        self.layers.remove(g);
        self.relink_after_remove(g);
        if self.current_layer >= g && self.current_layer > 0 {
            self.current_layer = if self.current_layer == g { kids.end.saturating_sub(1).max(kids.start) } else { self.current_layer - 1 };
        }
        self.current_layer = self.current_layer.min(self.layers.len().saturating_sub(1));
    }

    /// Delete layer `li`, a group with all its layers. Refused when it would
    /// leave no layer at all.
    pub fn delete_block(&mut self, li: usize) {
        let b = self.block(li);
        if b.is_empty() || b.len() >= self.layers.len() {
            return;
        }
        for i in b.clone().rev() {
            self.layers.remove(i);
            self.relink_after_remove(i);
        }
        let cur = self.current_layer;
        self.current_layer = if cur >= b.end {
            cur - b.len()
        } else if cur >= b.start {
            b.start.saturating_sub(1)
        } else {
            cur
        };
        self.current_layer = self.current_layer.min(self.layers.len() - 1);
    }

    /// Move layer `li` one step up (`up`) or down the panel, the way the
    /// panel reads: a group moves as a whole; a layer steps into a group it
    /// meets, through it one place at a time, and out the other end.
    /// `false` when there is nowhere to go.
    pub fn move_item(&mut self, li: usize, up: bool) -> bool {
        let n = self.layers.len();
        if li >= n {
            return false;
        }
        let mut order: Vec<usize> = (0..n).collect();
        // Membership, by old index.
        let mut member: Vec<bool> = self.layers.iter().map(|l| l.in_group).collect();
        let inside = self.layers[li].in_group;
        if self.is_group(li) {
            let mine = self.block(li);
            let (lo, hi) = if up {
                if mine.end >= n {
                    return false;
                }
                (mine.clone(), self.block(self.group_of(mine.end).unwrap_or(mine.end)))
            } else {
                if mine.start == 0 {
                    return false;
                }
                (self.block(mine.start - 1), mine.clone())
            };
            // The upper item goes under the lower one.
            let swapped: Vec<usize> = hi.clone().chain(lo.clone()).collect();
            order.splice(lo.start..hi.end, swapped);
        } else if up {
            if li + 1 >= n {
                return false;
            }
            let above = &self.layers[li + 1];
            if inside && above.is_group() {
                // The top of its group: out, above the header.
                order.swap(li, li + 1);
                member[li] = false;
            } else if !inside && (above.is_group() || above.in_group) {
                // Just under a group: in, as its bottom layer.
                member[li] = true;
            } else {
                order.swap(li, li + 1);
            }
        } else {
            if li == 0 && !inside {
                return false;
            }
            if inside && (li == 0 || !self.layers[li - 1].in_group) {
                // The bottom of its group: out, where it is.
                member[li] = false;
            } else if !inside && self.layers[li - 1].is_group() {
                // Just over a group: in, as its top layer.
                order.swap(li, li - 1);
                member[li] = true;
            } else {
                order.swap(li, li - 1);
            }
        }
        let flags: Vec<bool> = order.iter().map(|&o| member[o]).collect();
        self.apply_order(&order, &flags);
        true
    }

    /// Drop layer `from` — a group with its layers — on row `target`, just
    /// above it or just below, the way the panel shows rows:
    /// - on a group's layer: next to it, in that group;
    /// - on a group's header: above it, outside the group; below it, as the
    ///   group's top layer;
    /// - anywhere else: next to it, outside any group.
    ///
    /// A group dropped among another group's rows lands beside that whole
    /// group instead: groups never hold groups. `false` when nothing moved.
    pub fn drop_item(&mut self, from: usize, target: usize, above: bool) -> bool {
        let n = self.layers.len();
        if from >= n || target >= n {
            return false;
        }
        let moving = self.block(from);
        if moving.contains(&target) {
            return false;
        }
        let dragging_group = self.is_group(from);
        // A group dropped inside another group goes beside it, as a whole.
        let target = if dragging_group { self.group_of(target).unwrap_or(target) } else { target };
        let t = &self.layers[target];
        // Where it goes: next to `anchor` (after it when `after`), and
        // whether it is then in a group.
        let (anchor, after, member) = if t.in_group {
            (target, above, true)
        } else if t.is_group() && !above && !dragging_group {
            // Below a header: the group's top layer.
            (target, false, true)
        } else if t.is_group() && !above {
            // A group below a group: under its whole block.
            (self.block(target).start, false, false)
        } else {
            (target, above, false)
        };
        let rest: Vec<usize> = (0..n).filter(|i| !moving.contains(i)).collect();
        let Some(at) = rest.iter().position(|&r| r == anchor) else {
            return false;
        };
        let at = if after { at + 1 } else { at };
        let mut order = rest;
        order.splice(at..at, moving.clone());
        let mut member_of: Vec<bool> = self.layers.iter().map(|l| l.in_group).collect();
        if !dragging_group {
            member_of[from] = member;
        }
        let flags: Vec<bool> = order.iter().map(|&o| member_of[o]).collect();
        if order.iter().copied().eq(0..n) && flags.iter().zip(&self.layers).all(|(&f, l)| f == l.in_group) {
            return false;
        }
        self.apply_order(&order, &flags);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Layers named `names`, bottom first, none grouped; the top one active.
    fn stack(names: &[&str]) -> Project {
        let mut p = Project::new(8, 8, 24.0);
        p.layers[0].name = names[0].into();
        for n in &names[1..] {
            p.add_layer();
            let top = p.layers.len() - 1;
            p.layers[top].name = (*n).into();
        }
        p
    }

    /// The stack bottom first: a header in brackets, a group's layer
    /// indented.
    fn rows(p: &Project) -> Vec<String> {
        p.layers
            .iter()
            .map(|l| match (l.is_group(), l.in_group) {
                (true, _) => format!("[{}]", l.name),
                (false, true) => format!(" {}", l.name),
                (false, false) => l.name.clone(),
            })
            .collect()
    }

    fn by_name(p: &Project, name: &str) -> usize {
        p.layers.iter().position(|l| l.name == name).unwrap()
    }

    #[test]
    fn children_are_the_run_under_the_header() {
        let mut p = stack(&["a", "b", "c", "d"]);
        let g = p.group_layers(&[1, 2]).unwrap();
        assert_eq!(rows(&p), ["a", " b", " c", "[Group 1]", "d"]);
        assert_eq!(g, 3);
        assert_eq!(p.current_layer, g);
        assert_eq!(p.children(g), 1..3);
        assert_eq!(p.block(g), 1..4);
        assert_eq!(p.group_of(1), Some(g));
        assert_eq!(p.group_of(0), None);
        assert_eq!(p.group_of(4), None);
        assert!(p.groups_ok());
    }

    #[test]
    fn normalize_repairs_orphans_and_strips_header_cells() {
        let mut p = stack(&["a", "b", "c"]);
        // A stray: in a group with no header over it.
        p.layers[0].in_group = true;
        // A header that somehow kept drawings and a clip.
        p.layers[2].kind = crate::doc::layer::LayerKind::Group;
        p.layers[2].clip = true;
        p.layers[2].set_key(0, 0);
        p.layers[1].in_group = true;
        p.layers[0].lines_from = Some(2);
        p.layers[0].keys_from = Some(p.layers[2].uid);
        p.layers[1].keys_from = Some(p.layers[0].uid);
        p.normalize_groups();
        assert_eq!(rows(&p), [" a", " b", "[c]"], "a and b now under c's header");
        assert!(p.layers[2].exposures.iter().all(Option::is_none));
        assert!(!p.layers[2].clip);
        assert_eq!(p.layers[0].lines_from, None, "no fill reads a header");
        assert_eq!(p.layers[0].keys_from, None, "no key jump follows a header");
        assert_eq!(p.layers[1].keys_from, Some(p.layers[0].uid), "a link to a drawing layer stays");
        // And a run with nothing above it at all.
        let mut p = stack(&["a", "b"]);
        p.layers[1].in_group = true;
        p.normalize_groups();
        assert_eq!(rows(&p), ["a", "b"]);
    }

    #[test]
    fn grouping_a_scattered_selection_keeps_its_order() {
        let mut p = stack(&["a", "b", "c", "d", "e"]);
        p.group_layers(&[0, 2, 3]).unwrap();
        assert_eq!(rows(&p), ["b", " a", " c", " d", "[Group 1]", "e"]);
        // Grouping layers out of one group into another: the old one keeps
        // the rest, and the new one sits above it — never inside.
        let g1 = by_name(&p, "Group 1");
        let c = by_name(&p, "c");
        let d = by_name(&p, "d");
        p.group_layers(&[c, d]).unwrap();
        assert_eq!(rows(&p), ["b", " a", "[Group 1]", " c", " d", "[Group 2]", "e"]);
        let _ = g1;
        assert!(p.groups_ok());
        // A whole group selected gives its layers up and goes.
        let g2 = by_name(&p, "Group 2");
        let e = by_name(&p, "e");
        p.group_layers(&[g2, e]).unwrap();
        assert_eq!(rows(&p), ["b", " a", "[Group 1]", " c", " d", " e", "[Group 2]"]);
    }

    #[test]
    fn ungroup_and_delete_group() {
        let mut p = stack(&["a", "b", "c", "d"]);
        let g = p.group_layers(&[1, 2]).unwrap();
        p.ungroup(g);
        assert_eq!(rows(&p), ["a", "b", "c", "d"]);
        let g = p.group_layers(&[1, 2]).unwrap();
        p.delete_block(g);
        assert_eq!(rows(&p), ["a", "d"]);
        // Never the last layer.
        let mut p = stack(&["a"]);
        p.delete_block(0);
        assert_eq!(p.layers.len(), 1);
    }

    #[test]
    fn moves_enter_and_leave_one_step_at_a_time() {
        let mut p = stack(&["a", "b", "c", "d"]);
        p.group_layers(&[1, 2]).unwrap();
        // a, under the group: up enters it at the bottom, without moving.
        assert!(p.move_item(0, true));
        assert_eq!(rows(&p), [" a", " b", " c", "[Group 1]", "d"]);
        // Up through it, one place at a time.
        assert!(p.move_item(0, true));
        assert_eq!(rows(&p), [" b", " a", " c", "[Group 1]", "d"]);
        assert!(p.move_item(1, true));
        assert_eq!(rows(&p), [" b", " c", " a", "[Group 1]", "d"]);
        // And out over the header.
        assert!(p.move_item(2, true));
        assert_eq!(rows(&p), [" b", " c", "[Group 1]", "a", "d"]);
        // Down again: in as the top layer.
        assert!(p.move_item(3, false));
        assert_eq!(rows(&p), [" b", " c", " a", "[Group 1]", "d"]);
        // From the bottom of a group, down leaves it in place.
        assert!(p.move_item(0, false));
        assert_eq!(rows(&p), ["b", " c", " a", "[Group 1]", "d"]);
        assert!(!p.move_item(0, false), "nothing below the bottom");
        assert!(p.groups_ok());
    }

    #[test]
    fn a_header_moves_its_block() {
        let mut p = stack(&["a", "b", "c", "d"]);
        let g = p.group_layers(&[1, 2]).unwrap();
        assert!(p.move_item(g, true));
        assert_eq!(rows(&p), ["a", "d", " b", " c", "[Group 1]"]);
        assert!(!p.move_item(4, true), "already on top");
        assert!(p.move_item(4, false));
        assert!(p.move_item(3, false));
        assert_eq!(rows(&p), [" b", " c", "[Group 1]", "a", "d"]);
        // Past another group as a whole.
        let a = by_name(&p, "a");
        let d = by_name(&p, "d");
        p.group_layers(&[a, d]).unwrap();
        assert_eq!(rows(&p), [" b", " c", "[Group 1]", " a", " d", "[Group 2]"]);
        assert!(p.move_item(2, true));
        assert_eq!(rows(&p), [" a", " d", "[Group 2]", " b", " c", "[Group 1]"]);
    }

    #[test]
    fn drops_follow_the_rows() {
        let mut p = stack(&["a", "b", "c", "d"]);
        p.group_layers(&[1, 2]).unwrap();
        // [a,  b,  c, [G], d]
        // d dropped on b's row, above it: into the group, over b.
        assert!(p.drop_item(4, 1, true));
        assert_eq!(rows(&p), ["a", " b", " d", " c", "[Group 1]"]);
        // a dropped below the header: the group's top layer.
        let g = by_name(&p, "Group 1");
        assert!(p.drop_item(0, g, false));
        assert_eq!(rows(&p), [" b", " d", " c", " a", "[Group 1]"]);
        // a dropped above the header: out, over the group.
        let g = by_name(&p, "Group 1");
        assert!(p.drop_item(3, g, true));
        assert_eq!(rows(&p), [" b", " d", " c", "[Group 1]", "a"]);
        // The group dropped on its own layer goes nowhere.
        let g = by_name(&p, "Group 1");
        assert!(!p.drop_item(g, 0, true));
        // The group dropped above a: the whole block moves.
        assert!(p.drop_item(g, 4, true));
        assert_eq!(rows(&p), ["a", " b", " d", " c", "[Group 1]"]);
        assert!(p.groups_ok());
    }

    #[test]
    fn a_group_dropped_into_a_group_lands_beside_it() {
        let mut p = stack(&["a", "b", "c", "d"]);
        p.group_layers(&[0, 1]).unwrap();
        let c = by_name(&p, "c");
        let d = by_name(&p, "d");
        p.group_layers(&[c, d]).unwrap();
        // [ a,  b, [G1],  c,  d, [G2]]
        let g2 = by_name(&p, "Group 2");
        assert!(p.drop_item(g2, 0, false), "dropped under G1's bottom layer");
        assert_eq!(rows(&p), [" c", " d", "[Group 2]", " a", " b", "[Group 1]"]);
        assert!(p.groups_ok());
    }

    #[test]
    fn apply_order_relinks_lines_from_and_current() {
        let mut p = stack(&["ink", "paint", "x"]);
        p.layers[1].lines_from = Some(0);
        p.current_layer = 1;
        p.apply_order(&[2, 1, 0], &[false, false, false]);
        assert_eq!(rows(&p), ["x", "paint", "ink"]);
        assert_eq!(p.layers[1].lines_from, Some(2));
        assert_eq!(p.current_layer, 1);
    }

    #[test]
    fn clip_base_stays_in_the_group() {
        let mut p = stack(&["base", "a", "b", "top"]);
        p.group_layers(&[1, 2]).unwrap();
        // [base, a, b, G, top]
        p.layers[1].clip = true;
        p.layers[2].clip = true;
        assert_eq!(p.clip_base(1), None, "a group's bottom layer clips to nothing");
        assert_eq!(p.clip_base(2), None, "nor does one above it, with only clipped below");
        p.layers[1].clip = false;
        assert_eq!(p.clip_base(2), Some(1));
    }

    #[test]
    fn a_layer_above_a_group_clips_to_it() {
        let mut p = stack(&["base", "a", "b", "top"]);
        let g = p.group_layers(&[1, 2]).unwrap();
        p.layers[g + 1].clip = true;
        assert_eq!(p.clip_base(g + 1), Some(g));
        // A group itself can't be clipped.
        p.layers[g].clip = true;
        assert_eq!(p.clip_base(g), None);
    }

    #[test]
    fn hidden_or_locked_groups_cascade() {
        let mut p = stack(&["a", "b"]);
        let g = p.group_layers(&[0]).unwrap();
        assert!(p.shows(0) && p.paintable(0));
        p.layers[g].visible = false;
        assert!(!p.shows(0), "hidden with its group");
        p.layers[g].visible = true;
        p.layers[g].locked = true;
        assert!(p.locked_eff(0) && !p.paintable(0));
        assert!(!p.paintable(g), "a header is never painted");
        assert!(p.shows(2) && p.paintable(2), "outside the group, untouched");
    }
}


//! The HSL colour wheel: a disc (hue around, saturation out from the centre)
//! beside a lightness bar. Three uses: the Color panel with its harmony set,
//! and the compact picker behind every other colour button in the app.

use egui::epaint::Mesh;
use egui::{pos2, vec2, Color32, Frame, Id, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2};

use crate::app::AppState;
use crate::color::{fmt_rgb, wrap_hue, Harmony, Hsl};
use crate::ui::theme;

/// Mesh resolution of the disc. Vertex colours are interpolated across each
/// cell, so this is fine enough that the steps don't show.
const SEGMENTS: usize = 72;
const RINGS: usize = 8;
/// Width of the lightness bar.
pub const BAR_W: f32 = 16.0;
/// How near a dot or tick a press has to land to grab it.
const GRAB_R: f32 = 10.0;
/// Disc size in the compact picker.
const POPUP_DISC: f32 = 168.0;

/// A dot on the disc: one member of a harmony set.
#[derive(Clone, Copy)]
pub struct Marker {
    pub index: usize,
    pub hsl: Hsl,
    pub active: bool,
}

/// A press or drag on the disc: the dot it grabbed (`None` for bare disc)
/// and where that dot should now sit.
pub struct DiscDrag {
    pub index: Option<usize>,
    pub h: f32,
    pub s: f32,
}

/// What a press grabbed, held for the rest of the drag. The offset is from
/// the pointer to the grabbed dot's centre, so taking hold of a dot a few
/// pixels off-centre doesn't make it jump.
#[derive(Clone, Copy)]
struct Grab {
    index: Option<usize>,
    offset: Vec2,
}

/// Where `(h, s)` sits on a disc at `center` with radius `r`. Hue 0 (red) is
/// at the top and runs clockwise.
fn disc_pos(center: Pos2, r: f32, h: f32, s: f32) -> Pos2 {
    let a = h.to_radians();
    center + vec2(a.sin(), -a.cos()) * (r * s)
}

/// The `(h, s)` under `p`; past the rim reads as the rim.
fn disc_hs(center: Pos2, r: f32, p: Pos2) -> (f32, f32) {
    let d = p - center;
    let s = (d.length() / r).min(1.0);
    let h = wrap_hue(d.x.atan2(-d.y).to_degrees());
    (h, s)
}

/// Black or white, whichever reads against `c`.
fn contrast(c: Hsl) -> Color32 {
    if c.l > 0.55 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// Paint the hue/saturation disc at lightness `l` with `markers` on it, and
/// report a press or drag. A press within reach of a dot grabs that dot,
/// preferring the active one when they overlap; anywhere else is bare disc.
pub fn disc(ui: &mut Ui, diameter: f32, l: f32, markers: &[Marker]) -> Option<DiscDrag> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::click_and_drag());
    let center = rect.center();
    let r = diameter / 2.0 - 1.0;

    if ui.is_rect_visible(rect) {
        let mut mesh = Mesh::default();
        mesh.colored_vertex(center, Hsl { h: 0.0, s: 0.0, l }.to_color32());
        for ring in 1..=RINGS {
            let s = ring as f32 / RINGS as f32;
            for seg in 0..SEGMENTS {
                let h = seg as f32 * 360.0 / SEGMENTS as f32;
                mesh.colored_vertex(disc_pos(center, r, h, s), Hsl { h, s, l }.to_color32());
            }
        }
        let at = |ring: usize, seg: usize| (1 + (ring - 1) * SEGMENTS + seg % SEGMENTS) as u32;
        for seg in 0..SEGMENTS {
            mesh.add_triangle(0, at(1, seg), at(1, seg + 1));
            for ring in 1..RINGS {
                let (a, b) = (at(ring, seg), at(ring, seg + 1));
                let (c, d) = (at(ring + 1, seg), at(ring + 1, seg + 1));
                mesh.add_triangle(a, b, c);
                mesh.add_triangle(b, d, c);
            }
        }
        let painter = ui.painter_at(rect.expand(GRAB_R));
        painter.add(Shape::mesh(mesh));
        painter.circle_stroke(center, r, Stroke::new(1.0, theme::STROKE_THIN));

        // Spokes first, so the dots sit on top of every line.
        if markers.len() > 1 {
            for m in markers {
                let p = disc_pos(center, r, m.hsl.h, m.hsl.s);
                painter.line_segment([center, p], Stroke::new(1.0, theme::white_alpha(90)));
            }
        }
        for m in markers.iter().filter(|m| !m.active).chain(markers.iter().filter(|m| m.active)) {
            let p = disc_pos(center, r, m.hsl.h, m.hsl.s);
            let rad = if m.active { 7.0 } else { 5.0 };
            painter.circle_filled(p, rad, m.hsl.to_color32());
            painter.circle_stroke(p, rad, Stroke::new(1.5, contrast(m.hsl)));
            if m.active {
                painter.circle_stroke(p, rad + 2.0, Stroke::new(1.5, theme::ACCENT));
            }
        }
    }

    let grab_id = resp.id.with("grab");
    if !resp.is_pointer_button_down_on() {
        ui.data_mut(|d| d.remove::<Grab>(grab_id));
        return None;
    }
    let p = resp.interact_pointer_pos()?;
    let grab = ui.data(|d| d.get_temp::<Grab>(grab_id)).unwrap_or_else(|| {
        let hit = markers
            .iter()
            .map(|m| {
                let at = disc_pos(center, r, m.hsl.h, m.hsl.s);
                // A hair of bias, so stacked dots hand over the active one.
                let bias = if m.active { 0.5 } else { 0.0 };
                (m.index, at, p.distance(at) - bias)
            })
            .filter(|(_, _, d)| *d <= GRAB_R)
            .min_by(|a, b| a.2.total_cmp(&b.2));
        let g = match hit {
            Some((i, at, _)) => Grab { index: Some(i), offset: at - p },
            None => Grab { index: None, offset: Vec2::ZERO },
        };
        ui.data_mut(|d| d.insert_temp(grab_id, g));
        g
    });
    let (h, s) = disc_hs(center, r, p + grab.offset);
    Some(DiscDrag { index: grab.index, h, s })
}

/// A tick on the lightness bar: one member's lightness.
#[derive(Clone, Copy)]
pub struct Tick {
    pub index: usize,
    pub l: f32,
    pub active: bool,
}

/// A vertical lightness bar for hue `h`, saturation `s` — white at the top,
/// black at the bottom — with `ticks` on it. A press takes the nearest
/// tick (the active one when stacked) and drags it; one beyond reach jumps
/// it to the pointer. Returns the tick's index and its new lightness.
pub fn l_bar(ui: &mut Ui, height: f32, h: f32, s: f32, ticks: &[Tick]) -> Option<(usize, f32)> {
    let (rect, resp) = ui.allocate_exact_size(vec2(BAR_W, height), Sense::click_and_drag());
    let y_of = |l: f32| rect.bottom() - l * rect.height();

    if ui.is_rect_visible(rect) {
        const STEPS: usize = 24;
        let mut mesh = Mesh::default();
        for i in 0..=STEPS {
            let l = i as f32 / STEPS as f32;
            let c = Hsl { h, s, l }.to_color32();
            mesh.colored_vertex(pos2(rect.left(), y_of(l)), c);
            mesh.colored_vertex(pos2(rect.right(), y_of(l)), c);
            if i > 0 {
                let k = (2 * i) as u32;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k + 1, k);
            }
        }
        let painter = ui.painter_at(rect.expand(4.0));
        painter.add(Shape::mesh(mesh));
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, theme::STROKE_THIN));
        for t in ticks.iter().filter(|t| !t.active).chain(ticks.iter().filter(|t| t.active)) {
            let y = y_of(t.l);
            let line = [pos2(rect.left() - 3.0, y), pos2(rect.right() + 3.0, y)];
            let edge = contrast(Hsl { h, s, l: t.l });
            if t.active {
                painter.line_segment(line, Stroke::new(4.0, edge));
                painter.line_segment(line, Stroke::new(2.0, theme::ACCENT));
            } else {
                painter.line_segment(line, Stroke::new(1.5, edge));
            }
        }
    }

    let grab_id = resp.id.with("grab");
    if !resp.is_pointer_button_down_on() {
        ui.data_mut(|d| d.remove::<Grab>(grab_id));
        return None;
    }
    let p = resp.interact_pointer_pos()?;
    let grab = ui.data(|d| d.get_temp::<Grab>(grab_id)).unwrap_or_else(|| {
        let nearest = ticks
            .iter()
            .map(|t| {
                let bias = if t.active { 0.5 } else { 0.0 };
                (t.index, y_of(t.l), (p.y - y_of(t.l)).abs() - bias)
            })
            .min_by(|a, b| a.2.total_cmp(&b.2));
        let g = match nearest {
            Some((i, y, d)) if d <= GRAB_R => Grab { index: Some(i), offset: vec2(0.0, y - p.y) },
            Some((i, ..)) => Grab { index: Some(i), offset: Vec2::ZERO },
            None => Grab { index: None, offset: Vec2::ZERO },
        };
        ui.data_mut(|d| d.insert_temp(grab_id, g));
        g
    });
    let l = ((rect.bottom() - (p.y + grab.offset.y)) / rect.height()).clamp(0.0, 1.0);
    grab.index.map(|i| (i, l))
}

/// H°, S% and L% as drag fields. Returns the edited colour when one moved.
pub fn hsl_fields(ui: &mut Ui, c: Hsl) -> Option<Hsl> {
    let mut h = c.h.round();
    let mut s = (c.s * 100.0).round();
    let mut l = (c.l * 100.0).round();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let field = |ui: &mut Ui, v: &mut f32, label: &str, max: f32, suffix: &str, tip: &str| {
            ui.label(egui::RichText::new(label).color(theme::TEXT_MUTED));
            ui.add(
                egui::DragValue::new(v)
                    .range(0.0..=max)
                    .speed(0.5)
                    .fixed_decimals(0)
                    .suffix(suffix),
            )
            .on_hover_text(tip)
            .changed()
        };
        changed |= field(ui, &mut h, "H", 360.0, "°", "Hue: where round the wheel");
        changed |= field(ui, &mut s, "S", 100.0, "%", "Saturation: grey at 0, full colour at 100");
        changed |= field(ui, &mut l, "L", 100.0, "%", "Lightness: black at 0, white at 100");
    });
    changed.then(|| Hsl::new(h, s / 100.0, l / 100.0))
}

/// A flat colour swatch that reads as a button.
fn swatch_button(ui: &mut Ui, rgb: [u8; 3], open: bool) -> Response {
    let size = ui.spacing().interact_size;
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
        let stroke = if open || resp.hovered() {
            Stroke::new(1.5, theme::ACCENT)
        } else {
            Stroke::new(1.0, theme::STROKE_THIN)
        };
        ui.painter().rect_stroke(rect, 3.0, stroke);
    }
    resp
}

/// The compact picker: disc, lightness bar and fields, no harmonies.
fn compact_picker(ui: &mut Ui, c: &mut Hsl) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let marker = [Marker { index: 0, hsl: *c, active: true }];
        if let Some(d) = disc(ui, POPUP_DISC, c.l, &marker) {
            c.h = d.h;
            c.s = d.s;
            changed = true;
        }
        let tick = [Tick { index: 0, l: c.l, active: true }];
        if let Some((_, l)) = l_bar(ui, POPUP_DISC, c.h, c.s, &tick) {
            c.l = l;
            changed = true;
        }
    });
    if let Some(n) = hsl_fields(ui, *c) {
        *c = n;
        changed = true;
    }
    changed
}

/// A colour button that opens the HSL picker — the app's replacement for
/// egui's own `color_edit_button_*`. The picker keeps its HSL between
/// frames, so a grey keeps its hue and nothing drifts through the 8-bit
/// round trip; it only re-reads the bytes when they change from outside.
pub fn hsl_edit_button(ui: &mut Ui, rgb: &mut [u8; 3]) -> Response {
    let popup_id = ui.auto_id_with("hsl_popup");
    let open = ui.memory(|m| m.is_popup_open(popup_id));
    let mut resp = swatch_button(ui, *rgb, open).on_hover_text(fmt_rgb(*rgb));
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }
    if !ui.memory(|m| m.is_popup_open(popup_id)) {
        return resp;
    }

    let cache: Id = popup_id.with("hsl");
    let mut c = match ui.data(|d| d.get_temp::<Hsl>(cache)) {
        Some(prev) => Hsl::from_rgb_keeping(*rgb, prev),
        None => Hsl::from_rgb(*rgb),
    };
    let shown = egui::Area::new(popup_id)
        .kind(egui::UiKind::Picker)
        .order(egui::Order::Foreground)
        .fixed_pos(resp.rect.left_bottom() + vec2(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style()).show(ui, |ui| compact_picker(ui, &mut c)).inner
        });
    if shown.inner {
        *rgb = c.to_rgb();
        resp.mark_changed();
    }
    ui.data_mut(|d| d.insert_temp(cache, c));
    if !resp.clicked()
        && (ui.input(|i| i.key_pressed(egui::Key::Escape)) || shown.response.clicked_elsewhere())
    {
        ui.memory_mut(|m| m.close_popup());
    }
    resp
}

/// [`hsl_edit_button`] for a colour held as `0..=1` floats.
pub fn hsl_edit_button_f32(ui: &mut Ui, rgb: &mut [f32; 3]) -> Response {
    let mut bytes = rgb.map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8);
    let resp = hsl_edit_button(ui, &mut bytes);
    if resp.changed() {
        *rgb = bytes.map(|c| c as f32 / 255.0);
    }
    resp
}

/// The Color panel: the harmony scheme, the wheel with every member on it,
/// the members as swatches, and exact H/S/L for the one being painted with.
pub fn color_panel(state: &mut AppState, ui: &mut Ui) {
    // The brush colour can change from anywhere — eyedropper, a pinned
    // swatch, the swatch pie, Paste, a tool switch. Rather than hook every
    // one, notice the brush no longer matches and rebuild around it.
    let brush = {
        let c = state.brush.color;
        [c[0], c[1], c[2]]
    };
    if state.harmony.active_rgb() != brush {
        state.harmony.rebase(brush);
    }

    let mut changed = false;
    let mut pin = false;
    let set = &mut state.harmony;

    let mut scheme = set.scheme;
    egui::ComboBox::from_id_salt("harmony_scheme")
        .selected_text(scheme.label())
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for h in Harmony::ALL {
                ui.selectable_value(&mut scheme, h, h.label());
            }
        });
    if scheme != set.scheme {
        set.set_scheme(scheme);
        changed = true;
    }

    let members = set.members();
    let active = set.active.min(members.len() - 1);
    let mono = set.scheme == Harmony::Monochromatic;
    // Monochrome members share hue and saturation, so they are one dot on
    // the disc; what tells them apart is lightness, shown as bar ticks.
    let markers: Vec<Marker> = if mono {
        vec![Marker { index: active, hsl: members[active], active: true }]
    } else {
        members
            .iter()
            .enumerate()
            .map(|(i, &hsl)| Marker { index: i, hsl, active: i == active })
            .collect()
    };
    let ticks: Vec<Tick> = if mono {
        members
            .iter()
            .enumerate()
            .map(|(i, m)| Tick { index: i, l: m.l, active: i == active })
            .collect()
    } else {
        vec![Tick { index: active, l: set.base.l, active: true }]
    };

    ui.add_space(4.0);
    let gap = ui.spacing().item_spacing.x;
    // Capped so the panel still fits under Brush on a 900 px tall screen.
    let side = (ui.available_width() - BAR_W - gap).clamp(120.0, 184.0);
    ui.horizontal(|ui| {
        let cur = members[active];
        if let Some(d) = disc(ui, side, cur.l, &markers) {
            // Bare disc moves the base (in monochrome, the one dot), so what
            // you press on is what you paint with.
            let i = d.index.unwrap_or(if mono { active } else { set.scheme.base_index() });
            set.move_member(i, d.h, d.s);
            changed = true;
        }
        if let Some((i, l)) = l_bar(ui, side, cur.h, cur.s, &ticks) {
            set.set_member_lightness(i, l);
            changed = true;
        }
    });

    // The set as swatches: click to paint with one. The set stays put.
    ui.add_space(4.0);
    let members = set.members();
    let n = members.len() as f32;
    let w = ((ui.available_width() - gap * (n - 1.0)) / n).min(40.0);
    ui.horizontal(|ui| {
        for (i, m) in members.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click());
            ui.painter().rect_filled(rect, 3.0, m.to_color32());
            let stroke = if i == set.active {
                Stroke::new(2.0, theme::ACCENT)
            } else {
                Stroke::new(1.0, theme::STROKE_THIN)
            };
            ui.painter().rect_stroke(rect, 3.0, stroke);
            if i == set.scheme.base_index() {
                // The base gets a pip, so a turned set still shows its anchor.
                let pip = Rect::from_center_size(rect.center_bottom() - vec2(0.0, 4.0), vec2(6.0, 2.0));
                ui.painter().rect_filled(pip, 1.0, contrast(*m));
            }
            let base = if i == set.scheme.base_index() { " (base)" } else { "" };
            if resp.on_hover_text(format!("{}{base}", m.fmt())).clicked() {
                set.pick(i);
                changed = true;
            }
        }
    });

    ui.add_space(4.0);
    let cur = set.member(set.active);
    if let Some(n) = hsl_fields(ui, cur) {
        let i = set.active;
        if n.l != cur.l {
            set.set_member_lightness(i, n.l);
        } else {
            set.move_member(i, n.h, n.s);
        }
        changed = true;
    }

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(set.member(set.active).fmt()).monospace());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            pin = ui
                .button("Pin set")
                .on_hover_text("Add every colour of the set to the pinned swatches (the oldest drop past 32)")
                .clicked();
        });
    });

    if pin {
        let colors: Vec<[u8; 3]> = state.harmony.members().iter().map(|m| m.to_rgb()).collect();
        state.pin_set(&colors);
    }
    if changed {
        state.set_brush_color(state.harmony.active_rgb());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::HarmonySet;

    /// Run one frame of `ui_fn` across a narrow panel-sized screen.
    fn run(ctx: &egui::Context, events: Vec<egui::Event>, ui_fn: impl FnMut(&mut Ui)) -> egui::FullOutput {
        let mut ui_fn = ui_fn;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(232.0, 600.0))),
            events,
            ..Default::default()
        };
        ctx.run(raw, |ctx| {
            egui::CentralPanel::default().frame(Frame::none()).show(ctx, |ui| ui_fn(ui));
        })
    }

    /// Centre and radius of the disc painted in `out`, found by its mesh.
    fn disc_geom(out: &egui::FullOutput) -> (Pos2, f32) {
        out.shapes
            .iter()
            .find_map(|c| match &c.shape {
                Shape::Mesh(m) if m.vertices.len() == 1 + RINGS * SEGMENTS => {
                    let center = m.vertices[0].pos;
                    let r = m.vertices.iter().map(|v| v.pos.distance(center)).fold(0.0, f32::max);
                    Some((center, r))
                }
                _ => None,
            })
            .expect("a disc was painted")
    }

    /// Where the text `label` was painted in `out`.
    fn text_pos(out: &egui::FullOutput, label: &str) -> Pos2 {
        out.shapes
            .iter()
            .find_map(|c| match &c.shape {
                Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() / 2.0),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{label} was painted"))
    }

    fn press(p: Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    fn release(p: Pos2) -> Vec<egui::Event> {
        vec![egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    fn brush_rgb(state: &AppState) -> [u8; 3] {
        let c = state.brush.color;
        [c[0], c[1], c[2]]
    }

    /// A state whose brush and harmony set start on `rgb` under `scheme`.
    fn state_on(scheme: Harmony, rgb: [u8; 3]) -> AppState {
        let mut state = AppState::for_test();
        state.set_brush_color(rgb);
        state.harmony = HarmonySet::new(scheme, rgb);
        state
    }

    /// Press at `from`, drag through to `to`, let go; returns the disc.
    fn drag_panel(state: &mut AppState, from: impl Fn(Pos2, f32) -> Pos2, to: impl Fn(Pos2, f32) -> Pos2) {
        let ctx = egui::Context::default();
        let out = run(&ctx, vec![], |ui| color_panel(state, ui));
        let (c, r) = disc_geom(&out);
        let (a, b) = (from(c, r), to(c, r));
        run(&ctx, press(a), |ui| color_panel(state, ui));
        for i in 1..=4 {
            let p = a + (b - a) * (i as f32 / 4.0);
            run(&ctx, vec![egui::Event::PointerMoved(p)], |ui| color_panel(state, ui));
        }
        run(&ctx, release(b), |ui| color_panel(state, ui));
        run(&ctx, vec![], |ui| color_panel(state, ui));
    }

    #[test]
    fn a_press_on_bare_disc_paints_with_what_is_under_it() {
        let mut state = state_on(Harmony::Triadic, [200, 40, 40]);
        state.harmony.pick(2);
        state.set_brush_color(state.harmony.active_rgb());
        let at = |c, r| disc_pos(c, r, 200.0, 0.5);
        drag_panel(&mut state, at, at);
        let got = Hsl::from_rgb(brush_rgb(&state));
        assert!((got.h - 200.0).abs() < 2.0 && (got.s - 0.5).abs() < 0.03, "{got:?}");
        assert_eq!(state.harmony.active, 0, "bare disc moves the base and paints with it");
    }

    #[test]
    fn a_press_on_a_dot_paints_with_it_and_leaves_the_set() {
        let mut state = state_on(Harmony::Triadic, Hsl::new(0.0, 0.8, 0.5).to_rgb());
        let base = state.harmony.base;
        let m1 = state.harmony.member(1);
        // A few pixels off the dot's centre: still that dot, and no jump.
        let at = |c, r| disc_pos(c, r, m1.h, m1.s) + vec2(3.0, -2.0);
        drag_panel(&mut state, at, at);
        assert_eq!(state.harmony.active, 1);
        assert!((state.harmony.base.h - base.h).abs() < 0.01 && (state.harmony.base.s - base.s).abs() < 0.001);
        assert_eq!(brush_rgb(&state), m1.to_rgb());
    }

    #[test]
    fn dragging_a_flank_dot_opens_the_spread() {
        let mut state = state_on(Harmony::Analogous, Hsl::new(100.0, 0.8, 0.5).to_rgb());
        let h = state.harmony.base.h;
        let m2 = state.harmony.member(2);
        drag_panel(
            &mut state,
            |c, r| disc_pos(c, r, m2.h, m2.s),
            |c, r| disc_pos(c, r, h + 60.0, m2.s),
        );
        assert!((state.harmony.base.h - h).abs() < 0.01, "base held");
        assert!((state.harmony.analog_spread - 60.0).abs() < 1.0, "{}", state.harmony.analog_spread);
        assert_eq!(state.harmony.active, 2);
    }

    #[test]
    fn dragging_the_base_dot_turns_every_member() {
        let mut state = state_on(Harmony::Complementary, Hsl::new(0.0, 0.8, 0.5).to_rgb());
        let b = state.harmony.base;
        drag_panel(
            &mut state,
            |c, r| disc_pos(c, r, b.h, b.s),
            |c, r| disc_pos(c, r, 90.0, b.s),
        );
        let m = state.harmony.members();
        assert!((m[0].h - 90.0).abs() < 1.0 && (m[1].h - 270.0).abs() < 1.0, "{m:?}");
    }

    #[test]
    fn the_set_rebuilds_when_the_brush_changes_elsewhere() {
        let mut state = state_on(Harmony::Triadic, [255, 0, 0]);
        state.harmony.pick(1);
        state.set_brush_color([0, 0, 255]);
        let ctx = egui::Context::default();
        run(&ctx, vec![], |ui| color_panel(&mut state, ui));
        assert!((state.harmony.base.h - 240.0).abs() < 0.01);
        assert_eq!(state.harmony.active, 0);
        assert_eq!(brush_rgb(&state), [0, 0, 255], "a rebase never repaints the brush");
    }

    #[test]
    fn pin_set_pins_every_member_once() {
        let mut state = state_on(Harmony::Tetradic, [200, 60, 30]);
        state.palette = vec![state.harmony.member(0).to_rgb()];
        let ctx = egui::Context::default();
        let out = run(&ctx, vec![], |ui| color_panel(&mut state, ui));
        let p = text_pos(&out, "Pin set");
        run(&ctx, press(p), |ui| color_panel(&mut state, ui));
        run(&ctx, release(p), |ui| color_panel(&mut state, ui));
        let want: Vec<[u8; 3]> = state.harmony.members().iter().map(|m| m.to_rgb()).collect();
        assert_eq!(state.palette, want, "the held base is not pinned twice");
    }

    #[test]
    fn the_colour_button_opens_an_hsl_picker() {
        let ctx = egui::Context::default();
        let mut rgb = [255u8, 0, 0];
        let button = |ui: &mut Ui, rgb: &mut [u8; 3]| {
            hsl_edit_button(ui, rgb);
        };
        run(&ctx, vec![], |ui| button(ui, &mut rgb));
        let at = pos2(10.0, 9.0);
        run(&ctx, press(at), |ui| button(ui, &mut rgb));
        run(&ctx, release(at), |ui| button(ui, &mut rgb));
        // A new area sits out its first frame while it is sized.
        let out = run(&ctx, vec![], |ui| button(ui, &mut rgb));
        let (c, r) = disc_geom(&out);
        let p = c + (disc_pos(c, r, 240.0, 1.0) - c) * 0.98;
        run(&ctx, press(p), |ui| button(ui, &mut rgb));
        run(&ctx, release(p), |ui| button(ui, &mut rgb));
        let got = Hsl::from_rgb(rgb);
        assert!((got.h - 240.0).abs() < 2.0 && got.s > 0.95 && (got.l - 0.5).abs() < 0.01, "{got:?}");
        let esc = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        run(&ctx, vec![esc], |ui| button(ui, &mut rgb));
        let out = run(&ctx, vec![], |ui| button(ui, &mut rgb));
        assert!(
            !out.shapes.iter().any(|c| matches!(&c.shape, Shape::Mesh(m) if m.vertices.len() == 1 + RINGS * SEGMENTS)),
            "Esc closes the picker"
        );
    }

    #[test]
    fn disc_positions_round_trip() {
        let c = pos2(100.0, 100.0);
        for (h, s) in [(0.0, 1.0), (90.0, 0.5), (210.0, 0.25), (359.0, 0.9)] {
            let (h2, s2) = disc_hs(c, 80.0, disc_pos(c, 80.0, h, s));
            assert!((h2 - h).abs() < 0.01 && (s2 - s).abs() < 0.001, "{h} {s} → {h2} {s2}");
        }
        assert!(disc_pos(c, 80.0, 0.0, 1.0).y < c.y, "red at the top");
        assert!(disc_pos(c, 80.0, 90.0, 1.0).x > c.x, "clockwise");
    }
}

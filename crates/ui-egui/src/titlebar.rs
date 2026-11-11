//! The window's own title bar, where it draws one ([`BimApp::own_title_bar`], Windows and Linux, unless Settings > User Interface
//! asks for the system's): the app bar's empty space moves the window and a double-click there maximizes or restores it,
//! Minimize, Maximize and Close sit flush in its top right corner, and thin zones along its edges and corners resize it. Moving
//! and resizing are the system's own (`StartDrag`, `BeginResize`), so Windows' snapping to the screen's edges works as it does
//! with the system's bar, and Close asks as the system's Close does, so unsaved work is still asked about.

use bimcraft_i18n::msg;

use egui::{Color32, CursorIcon, Id, LayerId, Order, PointerButton, Rect, ResizeDirection, Sense, Stroke, StrokeKind, Ui, ViewportCommand};
use egui::{WidgetInfo, WidgetType, pos2, vec2};

use crate::BimApp;
use crate::lang::t;
use crate::style::pal;

/// One caption button, as wide as the system's on Windows.
pub(crate) const BUTTON_WIDTH: f32 = 46.0;
/// How far in from the window's edge a press resizes it, in points. The zones lie over the window, so they are as thin as the
/// room every layout leaves round its controls: the app bar keeps 5 above its buttons, the Floating cards stand off by `GAP`,
/// the Compact layout's ribbon 7, a Compact card's stroke and padding 3 (and a docked-right card pads its scrollbar in by this).
pub(crate) const EDGE: f32 = 4.0;
/// Close's fill under the pointer, as the system's.
pub(crate) const CLOSE_HOVER: Color32 = Color32::from_rgb(196, 43, 28);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Caption {
    Minimize,
    Maximize,
    Close,
}

/// Left to right.
const CAPTIONS: [Caption; 3] = [Caption::Minimize, Caption::Maximize, Caption::Close];

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

impl BimApp {
    /// Whether the window draws its own title bar now.
    pub(crate) fn own_bar(&self) -> bool {
        self.own_title_bar && !self.system_title_bar
    }

    /// Settings > User Interface's "Use the system's title bar": the system's bar comes back, or goes, at once.
    pub(crate) fn use_system_title_bar(&mut self, ctx: &egui::Context, on: bool) {
        self.system_title_bar = on;
        if self.own_title_bar {
            ctx.send_viewport_cmd(ViewportCommand::Decorations(on));
        }
    }
}

/// The bar's ground, `rect`: a press and drag on it moves the window, a double-click maximizes or restores it. Laid out before
/// what is on the bar, so its buttons keep their presses.
pub(crate) fn drag_area(ui: &Ui, rect: Rect) {
    let ctx = ui.ctx();
    let r = ui.interact(rect, Id::new("title-bar"), Sense::CLICK | Sense::DRAG);
    // egui hands a drag that starts on a button to the ground under it: remember whether the press landed on bare ground.
    if ctx.input(|i| i.pointer.any_pressed()) {
        let bare = ctx.viewport(|v| v.hits.click.is_some_and(|w| w.id == r.id));
        ctx.data_mut(|d| d.insert_temp(r.id, bare));
    }
    if r.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized(ctx)));
    } else if r.drag_started_by(PointerButton::Primary) && ctx.data(|d| d.get_temp(r.id).unwrap_or(false)) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// Where the caption buttons are this frame, for the window's edges to leave them their presses.
const CAPTIONS_AT: &str = "caption-buttons";

/// Where the open project is kept, as the title's chip says it (the browser's: the cloud, only this page, or changed since).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) enum Kept {
    Cloud,
    Unsaved,
    Local,
}

/// The chip after the title: a dot and what `label` says, in flat fills that read on the dark bar (mint on deep green for the
/// cloud, amber for changes not kept, the bar's dim lettering in an outline for a project only on this computer).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn paint_chip(ui: &Ui, rect: Rect, label: std::sync::Arc<egui::Galley>, kept: Kept) {
    let (fill, edge, ink) = match kept {
        Kept::Cloud => (Color32::from_rgb(0x1C, 0x46, 0x32), Color32::TRANSPARENT, Color32::from_rgb(0x8B, 0xE8, 0xB5)),
        Kept::Unsaved => (Color32::from_rgb(0x3A, 0x2E, 0x12), Color32::TRANSPARENT, Color32::from_rgb(0xF5, 0xC8, 0x6B)),
        Kept::Local => (Color32::TRANSPARENT, pal().app_bar_hover, pal().app_bar_muted),
    };
    let p = ui.painter();
    p.rect(rect, 10.0, fill, Stroke::new(1.0, edge), StrokeKind::Inside);
    p.circle_filled(rect.left_center() + vec2(9.0, 0.0), 3.0, ink);
    p.galley_with_override_text_color(pos2(rect.left() + 16.0, rect.center().y - label.size().y / 2.0), label, ink);
}

/// How wide the caption buttons are: none in the browser, whose own window has them.
pub(crate) fn captions_width() -> f32 {
    if cfg!(target_arch = "wasm32") { 0.0 } else { 3.0 * BUTTON_WIDTH }
}

/// Minimize, Maximize (Restore Down when maximized) and Close at the right end of `bar`, its full height. The browser draws none:
/// its tab is the window.
pub(crate) fn caption_buttons(ui: &mut Ui, bar: Rect) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let ctx = ui.ctx().clone();
    let max = maximized(&ctx);
    let cluster = Rect::from_min_max(pos2(bar.right() - 3.0 * BUTTON_WIDTH, bar.top()), bar.right_bottom());
    ctx.data_mut(|d| d.insert_temp(Id::new(CAPTIONS_AT), cluster));
    for (i, c) in CAPTIONS.into_iter().enumerate() {
        let rect = Rect::from_min_size(pos2(bar.right() - (3 - i) as f32 * BUTTON_WIDTH, bar.top()), vec2(BUTTON_WIDTH, bar.height()));
        let label = t(match c {
            Caption::Minimize => msg!("Minimize"),
            Caption::Maximize if max => msg!("Restore Down"),
            Caption::Maximize => msg!("Maximize"),
            Caption::Close => "Close",
        });
        let r = ui.interact(rect, Id::new(("caption", c as u8)), Sense::click());
        r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
        let hot = r.hovered() || r.has_focus();
        let (fill, ink) = match (c, hot) {
            (Caption::Close, true) => (CLOSE_HOVER, Color32::WHITE),
            (_, true) => (pal().app_bar_hover, pal().app_bar_text),
            _ => (Color32::TRANSPARENT, pal().app_bar_text),
        };
        let p = ui.painter();
        p.rect_filled(rect, 0.0, fill);
        // Keyboard focus is a ring in the ink, which holds against the fill: the fill alone is 1.3:1 against the bar.
        if r.has_focus() {
            p.rect_stroke(rect.shrink(3.0), 0.0, Stroke::new(2.0, ink), StrokeKind::Inside);
        }
        glyph(p, c, max, rect.center(), Stroke::new(1.0, ink));
        if r.on_hover_text(label).clicked() {
            ctx.send_viewport_cmd(match c {
                Caption::Minimize => ViewportCommand::Minimized(true),
                Caption::Maximize => ViewportCommand::Maximized(!max),
                Caption::Close => ViewportCommand::Close,
            });
        }
    }
}

/// A caption's sign, 10 points across round `at`, drawn as the system draws its own.
fn glyph(p: &egui::Painter, c: Caption, max: bool, at: egui::Pos2, s: Stroke) {
    let half = 5.0;
    match c {
        Caption::Minimize => {
            p.line_segment([at + vec2(-half, 0.5), at + vec2(half, 0.5)], s);
        }
        Caption::Maximize if max => {
            // Restore Down: a window in front of another.
            let front = Rect::from_min_size(at + vec2(-half, -half + 2.0), vec2(8.0, 8.0));
            p.rect_stroke(front, 1.0, s, StrokeKind::Inside);
            p.line_segment([at + vec2(-half + 2.0, -half + 2.0), at + vec2(-half + 2.0, -half)], s);
            p.line_segment([at + vec2(-half + 2.0, -half), at + vec2(half, -half)], s);
            p.line_segment([at + vec2(half, -half), at + vec2(half, half - 2.0)], s);
            p.line_segment([at + vec2(half, half - 2.0), at + vec2(half - 2.0, half - 2.0)], s);
        }
        Caption::Maximize => {
            p.rect_stroke(Rect::from_center_size(at, vec2(2.0 * half, 2.0 * half)), 1.0, s, StrokeKind::Inside);
        }
        Caption::Close => {
            p.line_segment([at + vec2(-half, -half), at + vec2(half, half)], s);
            p.line_segment([at + vec2(-half, half), at + vec2(half, -half)], s);
        }
    }
}

/// The zones along a window `w` that resize it, `edge` deep: each with the way it resizes and its cursor. The corners come last
/// and are twice as long, so they win where they meet the edges and are easy to hit. The `captions` (Minimize, Maximize and
/// Close, in the top right corner) keep their whole area: the top edge stops short of them, the right edge starts under them
/// and the corner they hold is not one.
pub(crate) fn zones(w: Rect, edge: f32, captions: Option<Rect>) -> Vec<(Rect, ResizeDirection, CursorIcon)> {
    use ResizeDirection::*;
    let c = 2.0 * edge;
    let (north_end, east_start) = captions.map_or((w.right(), w.top()), |r| (r.left(), r.bottom()));
    let mut zones = vec![
        (Rect::from_min_max(w.left_top(), pos2(north_end, w.top() + edge)), North, CursorIcon::ResizeVertical),
        (Rect::from_min_max(pos2(w.left(), w.bottom() - edge), w.right_bottom()), South, CursorIcon::ResizeVertical),
        (Rect::from_min_max(w.left_top(), pos2(w.left() + edge, w.bottom())), West, CursorIcon::ResizeHorizontal),
        (Rect::from_min_max(pos2(w.right() - edge, east_start), w.right_bottom()), East, CursorIcon::ResizeHorizontal),
        (Rect::from_min_size(w.left_top(), vec2(c, c)), NorthWest, CursorIcon::ResizeNwSe),
        (Rect::from_min_size(w.left_bottom() - vec2(0.0, c), vec2(c, c)), SouthWest, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.right_bottom() - vec2(c, c), vec2(c, c)), SouthEast, CursorIcon::ResizeNwSe),
    ];
    if captions.is_none() {
        zones.push((Rect::from_min_size(w.right_top() - vec2(c, 0.0), vec2(c, c)), NorthEast, CursorIcon::ResizeNeSw));
    }
    zones
}

/// The window's frame, over everything else: its edges and corners resize it, and a hairline marks its border, as the system's
/// frame would. A maximized or full-screen window has neither.
pub(crate) fn frame(ui: &mut Ui) {
    // A page in the browser is neither moved nor resized by its edges.
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let ctx = ui.ctx().clone();
    if maximized(&ctx) || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let w = ctx.content_rect();
    let layer = LayerId::new(Order::Foreground, Id::new("window-frame"));
    let top = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(w));
    top.painter().rect_stroke(w, 0.0, Stroke::new(1.0, pal().border), StrokeKind::Inside);
    let captions = ctx.data(|d| d.get_temp::<Rect>(Id::new(CAPTIONS_AT)));
    for (i, (r, dir, cursor)) in zones(w, EDGE, captions).into_iter().enumerate() {
        let resp = top.interact(r, Id::new(("window-edge", i)), Sense::DRAG).on_hover_cursor(cursor);
        if resp.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corners_win_where_they_meet_the_edges() {
        let w = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0));
        let at = |x: f32, y: f32| zones(w, EDGE, None).into_iter().rev().find(|(r, ..)| r.contains(pos2(x, y))).map(|(_, d, _)| d);
        assert_eq!(at(1.0, 300.0), Some(ResizeDirection::West));
        assert_eq!(at(7.0, 1.0), Some(ResizeDirection::NorthWest), "a corner reaches twice the edge along it");
        assert_eq!(at(9.0, 1.0), Some(ResizeDirection::North));
        assert_eq!(at(999.0, 699.0), Some(ResizeDirection::SouthEast));
        assert_eq!(at(500.0, 350.0), None);
    }

    #[test]
    fn the_captions_keep_their_area_and_the_edges_start_where_they_end() {
        let w = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0));
        let captions = Rect::from_min_max(pos2(862.0, 0.0), pos2(1000.0, 34.0));
        let all = zones(w, EDGE, Some(captions));
        assert!(all.iter().all(|(r, ..)| !r.intersect(captions).is_positive()), "no zone over a caption: {all:?}");
        let at = |x: f32, y: f32| all.iter().rev().find(|(r, ..)| r.contains(pos2(x, y))).map(|(_, d, _)| *d);
        assert_eq!(at(861.0, 1.0), Some(ResizeDirection::North), "the top edge runs up to the captions");
        assert_eq!(at(999.0, 35.0), Some(ResizeDirection::East), "the right edge starts under them");
    }
}

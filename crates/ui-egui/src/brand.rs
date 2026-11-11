//! CampusBIM's name and logo on the web build: the start page's hero, the title bar, the browser tab, and the About dialog with
//! the credit and the licence notice BIMCraft's MIT/Apache-2.0 licence asks every copy to carry. The desktop app keeps BIMCraft's.

use std::cell::Cell;

use egui::epaint::PathShape;
use egui::{Align, Color32, Context, FontId, Id, Layout, OpenUrl, Pos2, Rect, RichText, Sense, Stroke, Ui, vec2};

use crate::lang::t;
use crate::style::{self, pal};

/// The name the web build goes by.
pub const NAME: &str = "CampusBIM";
/// What it is, under the name on the start page.
pub const TAGLINE: &str = "Free BIM for Nigerian architecture students.";
/// Where this fork's source code is.
pub const SOURCE: &str = "https://github.com/Kambah123/BIMCraft";
/// The project it is built on.
pub const UPSTREAM: &str = "https://github.com/IPedrax/BIMCraft";

thread_local! {
    static ABOUT: Cell<bool> = const { Cell::new(false) };
    /// The look last kept in the browser, so it is written only when it changes.
    static KEPT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Where the browser keeps the theme the user picked: the scheme's code and the theme, e.g. `campus-dark dark`.
const LS_LOOK: &str = "campusbim.look";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// The window's first look: the theme the user picked last time in this browser, else CampusBIM Dark.
pub fn first_look(app: &mut crate::BimApp) {
    let kept = storage().and_then(|s| s.get_item(LS_LOOK).ok().flatten()).unwrap_or_default();
    let mut parts = kept.split_whitespace();
    let scheme = parts.next().and_then(crate::style::Scheme::from_code);
    let theme = match parts.next() {
        Some("light") => Some(egui::ThemePreference::Light),
        Some("dark") => Some(egui::ThemePreference::Dark),
        Some("system") => Some(egui::ThemePreference::System),
        _ => None,
    };
    match (scheme, theme) {
        (Some(scheme), Some(theme)) => {
            app.scheme = scheme;
            app.theme_wanted = Some(theme);
            KEPT.with(|k| *k.borrow_mut() = kept);
        }
        _ => {
            app.scheme = crate::style::Scheme::CampusDark;
            app.theme_wanted = Some(egui::ThemePreference::Dark);
        }
    }
}

/// Keeps the theme in the browser once the user has picked one (a pick differs from the first look).
pub fn keep_look(app: &crate::BimApp, ctx: &Context) {
    let theme = match ctx.options(|o| o.theme_preference) {
        egui::ThemePreference::Light => "light",
        egui::ThemePreference::Dark => "dark",
        egui::ThemePreference::System => "system",
    };
    let now = format!("{} {theme}", app.scheme.code());
    let first = app.theme_wanted.is_some() || (KEPT.with(|k| k.borrow().is_empty()) && now == "campus-dark dark");
    if first || KEPT.with(|k| *k.borrow() == now) {
        return;
    }
    if let Some(s) = storage() {
        let _ = s.set_item(LS_LOOK, &now);
    }
    KEPT.with(|k| *k.borrow_mut() = now);
}

/// Opens the About dialog.
pub fn open_about() {
    ABOUT.with(|a| a.set(true));
}

/// The grad cap (`mark.svg`, a 120 × 120 box), drawn into `r`.
pub fn mark(ui: &Ui, r: Rect) {
    let k = r.width() / 120.0;
    let at = |x: f32, y: f32| r.min + vec2(x, y) * k;
    let p = ui.painter();
    let rgb = |c: u32| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let (body, board, lime, mint) = (rgb(0x176b4a), rgb(0x0f4d35), rgb(0xd8f36c), rgb(0x8be8b5));
    let convex = |pts: Vec<Pos2>, fill: Color32| p.add(PathShape::convex_polygon(pts, fill, Stroke::NONE));
    // The cap's body: two halves, each convex, under the board; its lower edge a curve (Q60 96 from 30,80 to 90,80).
    let curve = |t: f32| at(30.0 + 60.0 * t, 80.0 + 32.0 * t * (1.0 - t));
    let mut left = vec![at(60.0, 72.0), at(30.0, 58.0)];
    left.extend((0..=8).map(|i| curve(i as f32 / 16.0)));
    convex(left, body);
    let mut right = vec![at(60.0, 72.0)];
    right.extend((8..=16).map(|i| curve(i as f32 / 16.0)));
    right.push(at(90.0, 58.0));
    convex(right, body);
    let diamond = vec![at(60.0, 22.0), at(108.0, 46.0), at(60.0, 70.0), at(12.0, 46.0)];
    convex(diamond.clone(), board);
    p.add(PathShape::closed_line(diamond, Stroke::new(3.5 * k, lime)));
    let grid = Stroke::new(2.5 * k, lime);
    p.line_segment([at(36.0, 34.0), at(84.0, 58.0)], grid);
    p.line_segment([at(84.0, 34.0), at(36.0, 58.0)], grid);
    p.add(PathShape::line(vec![at(60.0, 46.0), at(98.0, 51.0), at(98.0, 78.0)], Stroke::new(3.0 * k, mint)));
    convex(vec![at(94.0, 78.0), at(102.0, 78.0), at(101.0, 90.0), at(95.0, 90.0)], mint);
    p.circle_filled(at(60.0, 46.0), 3.5 * k, lime);
}

/// "Campus" in the text colour and "BIM" in the logo's green: lime on dark (`logo-dark.svg`), deep green on light (`logo-light.svg`).
pub fn wordmark(ui: &mut Ui, size: f32) {
    let bim = if style::dark() { Color32::from_rgb(0xd8, 0xf3, 0x6c) } else { Color32::from_rgb(0x17, 0x6b, 0x4a) };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(RichText::new("Campus").font(FontId::proportional(size)).color(pal().text));
        ui.label(RichText::new("BIM").font(FontId::proportional(size)).strong().color(bim));
    });
}

/// The app bar's logo: the cap and the wordmark, "BIM" in lime, as the bar is dark in both themes.
pub fn app_bar_logo(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
    mark(ui, rect);
    ui.add_space(-2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(RichText::new("Campus").font(FontId::proportional(15.0)).color(pal().app_bar_text));
        ui.label(RichText::new("BIM").font(crate::style::semibold(15.0)).color(Color32::from_rgb(0xd8, 0xf3, 0x6c)));
    });
    ui.add_space(6.0);
}

/// The start page's hero: the cap, the name and what it is.
pub fn hero(ui: &mut Ui) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
        mark(ui, rect);
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.add_space(4.0);
            wordmark(ui, 34.0);
            ui.label(RichText::new(t(TAGLINE)).size(15.0).color(pal().muted));
        });
    });
}

/// The footer's credit, which opens About.
pub fn credit(ui: &mut Ui) {
    ui.label(RichText::new("·").color(pal().muted));
    if ui.link(t("Built on BIMCraft (open source)")).on_hover_text(t("About CampusBIM")).clicked() {
        open_about();
    }
}

/// The browser tab's title.
pub fn set_tab_title(title: &str) {
    if let Some(d) = web_sys::window().and_then(|w| w.document()) {
        d.set_title(title);
    }
}

/// The About dialog, when it's open.
pub fn about_window(ctx: &Context) {
    if !ABOUT.with(Cell::get) {
        return;
    }
    let mut close = false;
    let r = crate::popup::Modal::new(Id::new("campusbim-about"), t("About CampusBIM")).width(460.0).show(ctx, |ui| {
        crate::popup::body(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
                mark(ui, rect);
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    wordmark(ui, 24.0);
                    ui.label(RichText::new(format!("{} {}", t("Version"), env!("CARGO_PKG_VERSION"))).color(pal().muted));
                });
            });
            ui.add_space(8.0);
            ui.label(t(TAGLINE));
            ui.add_space(8.0);
            ui.label(t("CampusBIM is built on BIMCraft, free and open-source building information modelling software."));
            let link = |ui: &mut Ui, label: &str, url: &str| {
                if ui.link(label).on_hover_text(url).clicked() {
                    ui.ctx().open_url(OpenUrl::new_tab(url));
                }
            };
            ui.horizontal_wrapped(|ui| {
                link(ui, t("CampusBIM source code"), SOURCE);
                ui.label(RichText::new("·").color(pal().muted));
                link(ui, t("BIMCraft"), UPSTREAM);
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Copyright (c) 2026 IPedrax and the BIMCraft contributors").small().color(pal().muted));
            ui.label(
                RichText::new(t(
                    "Licensed under the MIT License or the Apache License 2.0, at your option. Third-party notices are in NOTICE.",
                ))
                .small()
                .color(pal().muted),
            );
            ui.horizontal_wrapped(|ui| {
                for (i, name) in ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE"].into_iter().enumerate() {
                    if i > 0 {
                        ui.label(RichText::new("·").color(pal().muted));
                    }
                    link(ui, name, &page(name));
                }
            });
        });
        crate::popup::footer(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if ui.button(t("Close")).clicked() {
                    close = true;
                }
            });
        });
    });
    if close || r.should_close() {
        ABOUT.with(|a| a.set(false));
    }
}

/// A file served beside the app (the licence files are copied into the site when it's built).
fn page(name: &str) -> String {
    let base = web_sys::window().and_then(|w| w.location().href().ok()).unwrap_or_default();
    let base = base.split(['#', '?']).next().unwrap_or_default();
    let dir = base.rsplit_once('/').map(|(d, _)| d).unwrap_or(base);
    format!("{dir}/{name}")
}

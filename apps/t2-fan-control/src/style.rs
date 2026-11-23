// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
//! KaiT2en app style: palette, cards, margins and graph colors.
//!
//! Self contained on purpose. The app looks right with or without the
//! optional desktop look and builds without anything outside this crate.

use adw::StyleManager;
use gtk4::{gdk, glib, prelude::*, CssProvider, Orientation, Overflow, Widget};

const STYLE_CSS: &str = include_str!("style.css");

pub type Rgb = (f64, f64, f64);

/// Margin between window edge and content.
pub const SIDE_MARGIN: i32 = 18;
pub const TOP_MARGIN: i32 = 10;
pub const BOTTOM_MARGIN: i32 = 14;

/// Colors for the parts of a graph that are not data.
#[derive(Clone, Copy)]
pub struct GraphColors {
    pub grid: Rgb,
    pub label: Rgb,
}

const DARK_GRAPH: GraphColors = GraphColors {
    grid: (0.19, 0.19, 0.21),
    label: (0.52, 0.52, 0.55),
};

const LIGHT_GRAPH: GraphColors = GraphColors {
    grid: (0.80, 0.80, 0.83),
    label: (0.42, 0.42, 0.45),
};

/// Installs the app stylesheet for the default display.
///
/// libadwaita is initialized here because the app is a plain GTK app. GTK
/// answers the dark media queries of an app stylesheet from the stylesheet
/// itself, so the preference is handed to it and kept in sync.
pub fn install() {
    let _ = adw::init();
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = CssProvider::new();
    set_color_scheme(&provider, is_dark());
    provider.load_from_string(STYLE_CSS);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    StyleManager::default().connect_dark_notify(move |manager| {
        set_color_scheme(&provider, manager.is_dark());
        provider.load_from_string(STYLE_CSS);
    });
}

// The property exists since GTK 4.20, which gtk4-rs 0.9 does not bind yet.
fn set_color_scheme(provider: &CssProvider, dark: bool) {
    if provider.find_property("prefers-color-scheme").is_none() {
        return;
    }
    let Some(class) = glib::Type::from_name("GtkInterfaceColorScheme").and_then(glib::EnumClass::with_type) else {
        return;
    };
    if let Some(value) = class.to_value_by_nick(if dark { "dark" } else { "light" }) {
        provider.set_property_from_value("prefers-color-scheme", &value);
    }
}

pub fn is_dark() -> bool {
    StyleManager::default().is_dark()
}

pub fn graph_colors() -> GraphColors {
    if is_dark() { DARK_GRAPH } else { LIGHT_GRAPH }
}

/// Gives the root content of a window its margins.
pub fn pad(widget: &impl IsA<Widget>) {
    widget.set_margin_start(SIDE_MARGIN);
    widget.set_margin_end(SIDE_MARGIN);
    widget.set_margin_top(TOP_MARGIN);
    widget.set_margin_bottom(BOTTOM_MARGIN);
}

/// Wraps a widget in a rounded card that clips it to the rounding.
pub fn card(child: &impl IsA<Widget>) -> gtk4::Box {
    let card = gtk4::Box::new(Orientation::Vertical, 0);
    card.add_css_class("kait2en-card");
    card.set_overflow(Overflow::Hidden);
    card.append(child);
    card
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

#[allow(unused_imports)]
use adw::prelude::*;
use gtk4 as gtk;

use crate::collector;
use crate::diagnostics;
use crate::model::{Assessment, DeviceKind, DeviceTree, Property, Severity};

const APP_VERSION: &str = "0.02";

fn kait2en_brand() -> gtk::DrawingArea {
    // White type for dark surfaces, dark type for light ones. Without the
    // light file the white one is used.
    let dark = gtk::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark.png")
        .expect("failed to load kait2en wordmark");
    let light = gtk::gdk_pixbuf::Pixbuf::from_file("/usr/local/share/kait2en/kait2en-wordmark-light.png")
        .unwrap_or_else(|_| dark.clone());
    let brand = gtk::DrawingArea::new();
    brand.set_content_width(80); brand.set_content_height(21); brand.set_size_request(80, 21);
    brand.set_draw_func(move |_area, context, width, height| {
        let pixbuf = if adw::StyleManager::default().is_dark() { &dark } else { &light };
        let scale = f64::min(width as f64 / pixbuf.width() as f64, height as f64 / pixbuf.height() as f64);
        let _ = context.save();
        context.translate((width as f64 - pixbuf.width() as f64 * scale) / 2.0, (height as f64 - pixbuf.height() as f64 * scale) / 2.0);
        context.scale(scale, scale); context.set_source_pixbuf(&pixbuf, 0.0, 0.0);
        let _ = context.paint(); let _ = context.restore();
    });
    let weak = brand.downgrade();
    adw::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(brand) = weak.upgrade() {
            brand.queue_draw();
        }
    });
    brand
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Issues,
    Active,
    D0,
}

struct State {
    tree: DeviceTree,
    snapshot: Snapshot,
    expanded: BTreeSet<String>,
    visible: Vec<String>,
    selected: Option<String>,
    diagnostics: diagnostics::Diagnostics,
    filter: Filter,
}

#[derive(Default)]
struct Snapshot {
    properties: BTreeMap<String, Vec<Property>>,
    assessments: BTreeMap<String, Assessment>,
    runtime_status: BTreeMap<String, Option<String>>,
    pci_power_state: BTreeMap<String, Option<String>>,
    runtime_summary: BTreeMap<String, String>,
}

fn take_snapshot(tree: &DeviceTree, diagnostics: &diagnostics::Diagnostics) -> Snapshot {
    let mut snapshot = Snapshot::default();
    for node in tree.nodes.values() {
        let display_lanes = diagnostics.display_lanes.get(&node.id);
        let pci_pm = diagnostics.pci_pm.get(&node.id);
        snapshot.properties.insert(
            node.id.clone(),
            collector::properties(node, display_lanes.map(String::as_str), pci_pm),
        );
        snapshot
            .assessments
            .insert(node.id.clone(), collector::assessment(node));
        snapshot
            .runtime_status
            .insert(node.id.clone(), collector::runtime_status(node));
        snapshot
            .pci_power_state
            .insert(node.id.clone(), collector::pci_power_state(node));
        snapshot
            .runtime_summary
            .insert(node.id.clone(), collector::runtime_summary(node));
    }
    snapshot
}

#[derive(Clone)]
struct VisibleRow {
    id: String,
    depth: usize,
    ancestor_continues: Vec<bool>,
    is_last: bool,
}

fn palette_css(dark: bool) -> String {
    let (window, foreground, panel) = if dark {
        ("#151517", "#e8e8e8", "#0d0d0f")
    } else {
        ("#ffffff", "#242424", "#e2e2e8")
    };
    let (rim, line) = if dark {
        ("rgba(255,255,255,0.24)", "rgba(255,255,255,0.06)")
    } else {
        ("rgba(255,255,255,0.70)", "rgba(0,0,0,0.08)")
    };
    format!("
        window, headerbar, .app-background {{ background: {window}; color: alpha({foreground}, 0.72); }}
        button, button label, entry, spinbutton, dropdown {{ color: alpha({foreground}, 0.72); }}
        .title-1, .title-2, .title-3, .title-4, .title, .heading, windowtitle .title {{ color: {foreground}; font-weight: 600; }}
        .dim-label, windowtitle .subtitle {{ color: alpha({foreground}, 0.72); }}
        window.csd {{ border-top: 1px solid {rim}; }}
        window.csd.maximized, window.csd.tiled, window.csd.tiled-top, window.csd.tiled-left,
        window.csd.tiled-right, window.csd.tiled-bottom, window.csd.fullscreen {{ border-top-width: 0; }}
        .device-tree {{ background: {panel}; color: alpha({foreground}, 0.72); box-shadow: inset 0 0 0 1px {line}; }}
        .device-row {{ border-bottom: 1px solid alpha(@borders, .45); min-height: 34px; }}
        .device-id, .mono {{ font-family: 'JetBrains Mono'; font-feature-settings: 'tnum'; }}
        .state-text {{ font-family: 'JetBrains Mono'; color: alpha({foreground}, .72); }}
        .section-title {{ font-weight: 600; color: {foreground}; }}
        .property-row {{ border-bottom: 1px solid alpha(@borders, .35); min-height: 32px; }}
        .property-name {{ color: alpha({foreground}, .72); }}
        .property-value {{ font-family: 'JetBrains Mono'; font-feature-settings: 'tnum'; }}
        .thin-toolbar {{ border-bottom: 1px solid @borders; }}
        .tree-toolbar {{ border-bottom: 1px solid @borders; padding: 6px 8px; }}
        .tree-counts {{ font-family: 'JetBrains Mono'; color: alpha({foreground}, .72); font-feature-settings: 'tnum'; }}
        .severity-dot {{ min-width: 8px; min-height: 8px; border-radius: 4px; }}
        .severity-normal {{ background: transparent; }}
        .severity-info {{ background: alpha(@window_fg_color, .32); }}
        .severity-notice {{ background: #e5a50a; }}
        .severity-warning {{ background: #e01b24; }}
        .donate-link, .donate-link > label {{ color: alpha({foreground}, 0.72); }}
    ")
}

fn install_css() {
    let provider = gtk::CssProvider::new();
    let style = adw::StyleManager::default();
    provider.load_from_string(&palette_css(style.is_dark()));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    style.connect_dark_notify(move |manager| {
        provider.load_from_string(&palette_css(manager.is_dark()));
    });
}

fn aggregate_severity(state: &State, id: &str) -> Severity {
    let node = &state.tree.nodes[id];
    node.children.iter().fold(
        state.snapshot.assessments[id].severity,
        |severity, child| severity.max(aggregate_severity(state, child)),
    )
}

fn direct_match(state: &State, id: &str) -> bool {
    match state.filter {
        Filter::All => true,
        Filter::Issues => state.snapshot.assessments[id].severity >= Severity::Notice,
        Filter::Active => state.snapshot.runtime_status[id].as_deref() == Some("active"),
        Filter::D0 => state.snapshot.pci_power_state[id].as_deref() == Some("D0"),
    }
}

fn subtree_matches(state: &State, id: &str) -> bool {
    direct_match(state, id)
        || state.tree.nodes[id]
            .children
            .iter()
            .any(|child| subtree_matches(state, child))
}

fn append_visible(
    state: &State,
    id: &str,
    depth: usize,
    ancestor_continues: &[bool],
    is_last: bool,
    output: &mut Vec<VisibleRow>,
) {
    output.push(VisibleRow {
        id: id.to_owned(),
        depth,
        ancestor_continues: ancestor_continues.to_vec(),
        is_last,
    });
    if state.filter == Filter::All && !state.expanded.contains(id) {
        return;
    }
    let children: Vec<_> = state.tree.nodes[id]
        .children
        .iter()
        .filter(|child| state.filter == Filter::All || subtree_matches(state, child))
        .cloned()
        .collect();
    let child_count = children.len();
    for (index, child) in children.into_iter().enumerate() {
        let mut continuation = ancestor_continues.to_vec();
        if depth > 0 {
            continuation.push(!is_last);
        }
        append_visible(
            state,
            &child,
            depth + 1,
            &continuation,
            index + 1 == child_count,
            output,
        );
    }
}

fn hierarchy_widget(row: &VisibleRow, has_children: bool) -> gtk::Overlay {
    const STEP: i32 = 18;
    const WIDTH: i32 = 110;

    let overlay = gtk::Overlay::new();
    overlay.set_size_request(WIDTH, 30);
    overlay.set_hexpand(false);

    let lines = gtk::DrawingArea::new();
    lines.set_content_width(WIDTH);
    lines.set_content_height(30);
    let depth = row.depth.min(6);
    let continuations = row.ancestor_continues.clone();
    let is_last = row.is_last;
    lines.set_draw_func(move |_, cr, _, height| {
        cr.set_source_rgba(0.46, 0.49, 0.52, 0.55);
        cr.set_line_width(1.0);
        for (level, continues) in continuations.iter().enumerate() {
            if *continues {
                let x = (level as i32 * STEP + 11) as f64 + 0.5;
                cr.move_to(x, 0.0);
                cr.line_to(x, height as f64);
            }
        }
        if depth > 0 {
            let x = ((depth - 1) as i32 * STEP + 11) as f64 + 0.5;
            cr.move_to(x, 0.0);
            cr.line_to(x, if is_last { 15.0 } else { height as f64 });
            cr.move_to(x, 15.0);
            cr.line_to((depth as i32 * STEP + 11) as f64 + 0.5, 15.0);
        }
        let _ = cr.stroke();
    });
    overlay.set_child(Some(&lines));

    if has_children {
        let expander = gtk::Button::from_icon_name("pan-end-symbolic");
        expander.add_css_class("flat");
        expander.set_size_request(24, 24);
        expander.set_halign(gtk::Align::Start);
        expander.set_valign(gtk::Align::Center);
        expander.set_margin_start(((depth as i32 * STEP) - 1).clamp(0, WIDTH - 24));
        overlay.add_overlay(&expander);
    }
    overlay
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn clear_list(container: &gtk::ListBox) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn show_properties(grid: &gtk::Box, state: &Rc<RefCell<State>>) {
    clear_box(grid);
    let selected = state.borrow().selected.clone();
    let Some(id) = selected else {
        let empty = gtk::Label::new(Some("Select a device to inspect its power path"));
        empty.add_css_class("dim-label");
        empty.set_margin_top(36);
        grid.append(&empty);
        return;
    };
    let node = state.borrow().tree.nodes.get(&id).cloned();
    let Some(node) = node else { return };

    let heading = gtk::Box::new(gtk::Orientation::Vertical, 2);
    heading.set_margin_top(18);
    heading.set_margin_bottom(14);
    heading.set_margin_start(18);
    heading.set_margin_end(18);
    let title = gtk::Label::builder().label(&node.name).xalign(0.0).build();
    title.add_css_class("title-3");
    let subtitle = gtk::Label::builder().label(&node.id).xalign(0.0).build();
    subtitle.add_css_class("mono");
    subtitle.add_css_class("dim-label");
    heading.append(&title);
    heading.append(&subtitle);
    grid.append(&heading);

    let properties = state.borrow().snapshot.properties[&node.id].clone();
    let mut section = "";
    for property in properties {
        if section != property.section {
            section = property.section;
            let label = gtk::Label::builder().label(section).xalign(0.0).build();
            label.add_css_class("section-title");
            label.set_margin_top(12);
            label.set_margin_bottom(4);
            label.set_margin_start(18);
            grid.append(&label);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        row.add_css_class("property-row");
        row.set_margin_start(18);
        row.set_margin_end(18);
        let name = gtk::Label::builder()
            .label(property.name)
            .xalign(0.0)
            .width_chars(22)
            .build();
        name.add_css_class("property-name");
        let value = gtk::Label::builder()
            .label(&property.value)
            .xalign(0.0)
            .hexpand(true)
            .selectable(true)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        value.add_css_class("property-value");
        if let Some(path) = property.path {
            value.set_tooltip_text(Some(&path.display().to_string()));
        }
        row.append(&name);
        row.append(&value);
        grid.append(&row);
    }
}

fn update_counts(label: &gtk::Label, state: &State) {
    let mut issues = 0;
    let mut active = 0;
    let mut d0 = 0;
    for node in state
        .tree
        .nodes
        .values()
        .filter(|node| node.kind != DeviceKind::Group)
    {
        if state.snapshot.assessments[&node.id].severity >= Severity::Notice {
            issues += 1;
        }
        if state.snapshot.runtime_status[&node.id].as_deref() == Some("active") {
            active += 1;
        }
        if state.snapshot.pci_power_state[&node.id].as_deref() == Some("D0") {
            d0 += 1;
        }
    }
    label.set_label(&format!("{issues} issues  |  {active} active  |  {d0} D0"));
}

fn rebuild_tree(
    list: &gtk::ListBox,
    details: &gtk::Box,
    counts: &gtk::Label,
    state: &Rc<RefCell<State>>,
) {
    clear_list(list);
    let roots = state.borrow().tree.roots.clone();
    let mut visible = Vec::new();
    {
        let state_ref = state.borrow();
        let roots: Vec<_> = roots
            .into_iter()
            .filter(|root| state_ref.filter == Filter::All || subtree_matches(&state_ref, root))
            .collect();
        let root_count = roots.len();
        for (index, root) in roots.into_iter().enumerate() {
            append_visible(
                &state_ref,
                &root,
                0,
                &[],
                index + 1 == root_count,
                &mut visible,
            );
        }
    }
    state.borrow_mut().visible = visible.iter().map(|row| row.id.clone()).collect();
    update_counts(counts, &state.borrow());

    for visible_row in visible {
        let id = visible_row.id.clone();
        let node = state.borrow().tree.nodes[&id].clone();
        let row = gtk::ListBoxRow::new();
        row.add_css_class("device-row");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        line.set_hexpand(true);
        line.set_halign(gtk::Align::Fill);
        line.set_margin_start(6);
        line.set_margin_end(10);

        let hierarchy = hierarchy_widget(&visible_row, !node.children.is_empty());
        if !node.children.is_empty() {
            let expander = hierarchy.last_child().expect("expander overlay");
            let expander = expander.downcast::<gtk::Button>().expect("button");
            expander.set_icon_name(if state.borrow().expanded.contains(&id) {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            });
            expander.set_tooltip_text(Some("Expand device"));
            let state_clone = state.clone();
            let list_clone = list.clone();
            let details_clone = details.clone();
            let counts_clone = counts.clone();
            let id_clone = id.clone();
            expander.connect_clicked(move |_| {
                let mut state = state_clone.borrow_mut();
                if !state.expanded.remove(&id_clone) {
                    state.expanded.insert(id_clone.clone());
                }
                drop(state);
                rebuild_tree(&list_clone, &details_clone, &counts_clone, &state_clone);
            });
        }
        line.append(&hierarchy);

        let id_label = gtk::Label::builder()
            .label(
                node.id
                    .trim_start_matches("usb:")
                    .trim_start_matches("platform:")
                    .trim_start_matches("virtual:"),
            )
            .xalign(0.0)
            .width_chars(18)
            .max_width_chars(18)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        id_label.set_halign(gtk::Align::Start);
        id_label.add_css_class("device-id");
        let name = gtk::Label::builder()
            .label(&node.name)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        name.add_css_class("device-name");
        let summary = gtk::Label::builder()
            .label(&state.borrow().snapshot.runtime_summary[&id])
            .xalign(1.0)
            .width_chars(17)
            .max_width_chars(17)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        summary.add_css_class("state-text");
        let own_assessment = state.borrow().snapshot.assessments[&id].clone();
        let inherited = aggregate_severity(&state.borrow(), &id);
        let indicator = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        indicator.add_css_class("severity-dot");
        indicator.add_css_class(match inherited {
            Severity::Normal => "severity-normal",
            Severity::Informational => "severity-info",
            Severity::Notice => "severity-notice",
            Severity::Warning => "severity-warning",
        });
        indicator.set_tooltip_text(Some(if inherited > own_assessment.severity {
            "A descendant contains a power-management issue"
        } else {
            &own_assessment.message
        }));
        line.append(&id_label);
        line.append(&name);
        line.append(&indicator);
        line.append(&summary);
        row.set_child(Some(&line));
        list.append(&row);
    }
}

pub fn build(app: &adw::Application) {
    install_css();
    let tree = collector::collect();
    let diagnostics = diagnostics::read();
    let snapshot = take_snapshot(&tree, &diagnostics);
    let mut expanded = BTreeSet::new();
    expanded.extend(tree.roots.iter().cloned());
    let state = Rc::new(RefCell::new(State {
        tree,
        snapshot,
        expanded,
        visible: Vec::new(),
        selected: None,
        diagnostics,
        filter: Filter::All,
    }));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("T2 Power Explorer")
        .default_width(1400)
        .default_height(760)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("app-background");
    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new(
        "Power Explorer",
        "Kernel device topology and runtime power state",
    );
    header.set_title_widget(Some(&title));
    let brand = kait2en_brand();
    brand.set_margin_start(10);
    brand.set_margin_end(10);
    header.pack_start(&brand);
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Rescan device topology"));
    header.pack_end(&refresh);
    root.append(&header);

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_position(700);
    paned.set_wide_handle(false);
    paned.set_vexpand(true);
    let list = gtk::ListBox::new();
    list.add_css_class("device-tree");
    list.set_selection_mode(gtk::SelectionMode::Single);
    let left_scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let tree_toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    tree_toolbar.add_css_class("tree-toolbar");
    let all = gtk::ToggleButton::with_label("All");
    let issues = gtk::ToggleButton::with_label("Issues");
    let active = gtk::ToggleButton::with_label("Active");
    let d0 = gtk::ToggleButton::with_label("D0");
    issues.set_group(Some(&all));
    active.set_group(Some(&all));
    d0.set_group(Some(&all));
    all.set_active(true);
    let counts = gtk::Label::builder().xalign(1.0).hexpand(true).build();
    counts.add_css_class("tree-counts");
    for button in [&all, &issues, &active, &d0] {
        tree_toolbar.append(button);
    }
    tree_toolbar.append(&counts);
    left.append(&tree_toolbar);
    left.append(&left_scroll);
    let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let right = gtk::ScrolledWindow::builder()
        .child(&details)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    paned.set_start_child(Some(&left));
    paned.set_end_child(Some(&right));
    root.append(&paned);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.set_margin_start(18); footer.set_margin_end(18); footer.set_margin_bottom(8);
    let donate = gtk::LinkButton::builder().uri("https://donate.stripe.com/eVq14n8a7agh2lQdqq14400").label("Fund our bugs").build();
    donate.add_css_class("donate-link"); footer.append(&donate);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0); spacer.set_hexpand(true); footer.append(&spacer);
    footer.append(&gtk::Label::new(Some(&format!("v{APP_VERSION}"))));
    root.append(&footer);
    window.set_content(Some(&root));

    rebuild_tree(&list, &details, &counts, &state);
    show_properties(&details, &state);

    let state_selected = state.clone();
    let details_selected = details.clone();
    list.connect_row_selected(move |_, row| {
        let Some(row) = row else { return };
        let index = row.index() as usize;
        let selected = state_selected.borrow().visible.get(index).cloned();
        state_selected.borrow_mut().selected = selected;
        show_properties(&details_selected, &state_selected);
    });

    let list_refresh = list.clone();
    let details_refresh = details.clone();
    let counts_refresh = counts.clone();
    let state_refresh = state.clone();
    refresh.connect_clicked(move |_| {
        let mut current = state_refresh.borrow_mut();
        current.tree = collector::collect();
        current.diagnostics = diagnostics::read();
        current.snapshot = take_snapshot(&current.tree, &current.diagnostics);
        if current
            .selected
            .as_ref()
            .is_some_and(|id| !current.tree.nodes.contains_key(id))
        {
            current.selected = None;
        }
        drop(current);
        rebuild_tree(
            &list_refresh,
            &details_refresh,
            &counts_refresh,
            &state_refresh,
        );
        show_properties(&details_refresh, &state_refresh);
    });

    for (button, filter) in [
        (all, Filter::All),
        (issues, Filter::Issues),
        (active, Filter::Active),
        (d0, Filter::D0),
    ] {
        let list_filter = list.clone();
        let details_filter = details.clone();
        let counts_filter = counts.clone();
        let state_filter = state.clone();
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            state_filter.borrow_mut().filter = filter;
            rebuild_tree(&list_filter, &details_filter, &counts_filter, &state_filter);
        });
    }

    window.present();
}

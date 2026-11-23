#!/usr/bin/python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
import os
import math
import signal
import statistics
import subprocess
import threading
import time
from collections import deque

import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, GdkPixbuf, GLib, Gtk, Pango

APP_ID = "org.t2cpucontrol.gtk"
APP_VERSION = "0.02"
HELPER = "/usr/local/libexec/t2-cpu-control-helper"
STATUS = "/usr/local/libexec/t2-cpu-control-status"
BENCHMARK = "/usr/local/libexec/t2-cpu-kernel-benchmark"
STATUS_CACHE = "/run/t2-cpu-control/status"
HISTORY = 180
FREQUENCY_STEP_MHZ = 100


def frequency_step_mhz(value):
    return round(float(value) / FREQUENCY_STEP_MHZ) * FREQUENCY_STEP_MHZ


def palette_css(dark):
    if dark:
        window_bg, window_fg = "#161616", "#e8e8e8"
        box_bg = "#101010"
    else:
        window_bg, window_fg = "#f2f2f2", "#242424"
        box_bg = "#e8e8e8"
    return f"""
        window, popover {{ font-family: "JetBrains Mono"; font-size: 11pt; font-weight: 400; color: alpha({window_fg}, 0.72); }}
        button, button label, entry, spinbutton, dropdown {{ font-size: 11pt; font-weight: 400; color: alpha({window_fg}, 0.72); }}
        .app-background {{ background-color: {window_bg}; color: alpha({window_fg}, 0.72); }}
        .app-background headerbar {{ background-color: @headerbar_bg_color; color: alpha({window_fg}, 0.72); }}
        .app-background .title-1, .app-background .title-2, .app-background .title-3,
        .app-background .title-4, .app-background .title, .app-background .heading, windowtitle .title {{ color: {window_fg}; font-size: 11pt; font-weight: 400; }}
        .app-background .dim-label, windowtitle .subtitle {{ color: alpha({window_fg}, 0.72); font-size: 11pt; font-weight: 400; }}
        .unified-box {{
            background-color: {box_bg};
            color: alpha({window_fg}, 0.72);
            border: none;
            border-radius: 12px;
            box-shadow: none;
        }}
        .padded-box {{ padding: 14px; }}
        .donate-link, .donate-link > label {{ color: alpha({window_fg}, 0.72); font-size: 11pt; font-weight: 400; }}
    """


def command(*args, check=True):
    result = subprocess.run(args, text=True, capture_output=True)
    if check and result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or f"exit status {result.returncode}"
        raise RuntimeError(detail)
    return result


def thermald_status():
    if command("systemctl", "is-active", "--quiet", "thermald.service", check=False).returncode == 0:
        return "active"
    if command("systemctl", "is-enabled", "--quiet", "thermald.service", check=False).returncode == 0:
        return "stopped"
    return "disabled"


def read_status():
    with open(STATUS_CACHE, encoding="ascii") as status_file:
        output = status_file.read()
    data, cores = {}, []
    for line in output.splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        if key.startswith("cpu."):
            mhz, temp, thermal, prochot = map(int, value.split(","))
            cores.append((int(key[4:]), mhz, temp, bool(thermal), bool(prochot)))
        else:
            data[key] = value
    data["cores"] = sorted(cores)
    return data


class HistoryGraph(Gtk.DrawingArea):
    def __init__(self, title, unit, colors, fixed_max=None, line_width=1.8):
        super().__init__()
        self.title, self.unit, self.colors, self.fixed_max = title, unit, colors, fixed_max
        self.line_width = line_width
        self.series = []
        self.current_value = None
        self.set_content_height(190)
        self.set_hexpand(True)
        self.set_draw_func(self.draw)

    def update(self, series, current_value=None):
        self.series = series
        self.current_value = current_value
        self.queue_draw()

    def draw(self, _area, cr, width, height):
        dark = Adw.StyleManager.get_default().get_dark()
        bg = (0.063, 0.063, 0.063) if dark else (0.91, 0.91, 0.91)
        fg = (0.90, 0.90, 0.90) if dark else (0.15, 0.15, 0.15)
        grid = (0.28, 0.28, 0.28) if dark else (0.72, 0.72, 0.72)
        radius = 12
        cr.new_sub_path()
        cr.arc(width - radius, radius, radius, -math.pi / 2, 0)
        cr.arc(width - radius, height - radius, radius, 0, math.pi / 2)
        cr.arc(radius, height - radius, radius, math.pi / 2, math.pi)
        cr.arc(radius, radius, radius, math.pi, 3 * math.pi / 2)
        cr.close_path()
        cr.clip()
        cr.set_source_rgb(*bg); cr.paint()
        cr.reset_clip()
        left, top, right, bottom = 48, 28, width - 12, height - 24
        cr.set_line_width(1); cr.set_source_rgb(*grid)
        for i in range(5):
            y = top + (bottom - top) * i / 4
            cr.move_to(left, y); cr.line_to(right, y)
        cr.stroke()
        values = [v for _, vals in self.series for v in vals if v is not None]
        maximum = self.fixed_max or (max(values, default=1) * 1.1)
        maximum = max(maximum, 1)
        cr.set_source_rgb(*fg); cr.select_font_face("JetBrains Mono", 0, 0); cr.set_font_size(11)
        cr.move_to(12, 18); cr.show_text(self.title)
        if self.current_value is not None:
            value = f"{self.current_value:.0f}{self.unit}"
            extents = cr.text_extents(value)
            cr.move_to(width - extents.width - 14, 18)
            cr.show_text(value)
        cr.set_font_size(11); cr.move_to(8, top + 4); cr.show_text(f"{maximum:.0f}{self.unit}")
        cr.move_to(18, bottom); cr.show_text(f"0{self.unit}")
        for idx, (label, vals) in enumerate(self.series):
            if len(vals) < 2: continue
            cr.set_source_rgb(*self.colors[idx % len(self.colors)]); cr.set_line_width(self.line_width)
            started = False
            for i, value in enumerate(vals):
                if value is None: started = False; continue
                x = left + (right - left) * i / max(HISTORY - 1, 1)
                y = bottom - min(value / maximum, 1.0) * (bottom - top)
                (cr.line_to if started else cr.move_to)(x, y); started = True
            cr.stroke()
            if len(self.series) <= 6:
                cr.move_to(left + idx * 115, height - 7); cr.show_text(label)


class CpuControl(Adw.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID)
        self.connect("activate", self.activate)
        self.data = {}
        self.history_power = deque(maxlen=HISTORY)
        self.history_package_temp = deque(maxlen=HISTORY)
        self.last_energy = None
        self.last_energy_time = None
        self.calibrating = False
        self.calibration_persist = 0
        self.notice_until = 0
        self.control_update = False
        self.settings_dirty = False
        self.cpu_rows = []
        self.controls_initialized = False
        self.prochot_control_update = False
        self.prochot_change_pending = False
        self.cancel_event = threading.Event()
        self.status_monitor = None
        self.thermald_state = "unknown"

    def activate(self, _app):
        self.thermald_state = thermald_status()
        if self.status_monitor is None or self.status_monitor.poll() is not None:
            self.status_monitor = subprocess.Popen([
                "pkexec", "--disable-internal-agent", STATUS, "monitor", str(os.getpid())
            ])
        self.controls_initialized = False
        win = Adw.ApplicationWindow(application=self, title="T2 CPU Control")
        win.set_default_size(940, 940)
        style_manager = Adw.StyleManager.get_default()
        css = Gtk.CssProvider()
        css.load_from_string(palette_css(style_manager.get_dark()))
        Gtk.StyleContext.add_provider_for_display(
            win.get_display(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
        )
        def update_theme(manager, _property):
            css.load_from_string(palette_css(manager.get_dark()))
            win.queue_draw()

        style_manager.connect("notify::dark", update_theme)
        header = Adw.HeaderBar()
        title = Adw.WindowTitle(title="CPU Control", subtitle="Package power and thermal stability")
        header.set_title_widget(title)
        brand_pixbuf = GdkPixbuf.Pixbuf.new_from_file(
            "/usr/local/share/kait2en/kait2en-wordmark.png"
        )
        brand = Gtk.DrawingArea(content_width=80, content_height=21)
        def draw_brand(_area, context, width, height):
            source_width = brand_pixbuf.get_width()
            source_height = brand_pixbuf.get_height()
            scale = min(width / source_width, height / source_height)
            offset_x = (width - source_width * scale) / 2
            offset_y = (height - source_height * scale) / 2
            context.save()
            context.translate(offset_x, offset_y)
            context.scale(scale, scale)
            Gdk.cairo_set_source_pixbuf(context, brand_pixbuf, 0, 0)
            context.paint()
            context.restore()
        brand.set_draw_func(draw_brand)
        brand.set_size_request(80, 21)
        brand.set_margin_start(10); brand.set_margin_end(10); header.pack_start(brand)
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        root.add_css_class("app-background")
        root.append(header)
        body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        body.set_margin_top(14); body.set_margin_bottom(18); body.set_margin_start(18); body.set_margin_end(18)
        body.set_vexpand(True)
        root.append(body); win.set_content(root)

        self.summary = Gtk.Label(xalign=0); self.summary.add_css_class("title-3")
        self.status = Gtk.Label(xalign=0)
        self.status.add_css_class("dim-label")
        self.status.add_css_class("monospace")
        self.status.set_hexpand(True)
        self.status.set_ellipsize(Pango.EllipsizeMode.END)
        body.append(self.summary); body.append(self.status)

        controls = Gtk.Grid(column_spacing=14, column_homogeneous=True)
        auto_panel = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        manual_panel = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        auto_content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        manual_content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        for panel in (auto_panel, manual_panel):
            panel.add_css_class("unified-box")
            panel.set_valign(Gtk.Align.FILL)
        for content in (auto_content, manual_content):
            content.set_margin_top(14)
            content.set_margin_bottom(14)
            content.set_margin_start(14)
            content.set_margin_end(14)
        auto_panel.append(auto_content)
        manual_panel.append(manual_content)
        controls.attach(auto_panel, 0, 0, 1, 1)
        controls.attach(manual_panel, 1, 0, 1, 1)
        body.append(controls)

        auto_title = Gtk.Label(label="Automatic tuning", xalign=0)
        auto_title.add_css_class("title-4")
        self.auto_hint = Gtk.Label(
            label="Tests PL2 while keeping PL1 at the detected processor base power.",
            xalign=0,
        )
        self.auto_hint.set_wrap(True)
        self.auto_hint.add_css_class("dim-label")
        auto_actions = Gtk.Box(spacing=8)
        self.cal_btn = Gtk.Button(label="Auto-tune power limits")
        self.cal_btn.add_css_class("suggested-action")
        self.cancel_btn = Gtk.Button(label="Cancel")
        self.cancel_btn.set_sensitive(False)
        auto_actions.append(self.cal_btn)
        auto_actions.append(self.cancel_btn)
        self.auto_result = Gtk.Label(label="No auto-tune result yet", xalign=0)
        self.auto_result.set_wrap(True)
        self.auto_result.add_css_class("dim-label")
        auto_content.append(auto_title); auto_content.append(self.auto_hint); auto_content.append(auto_actions)
        auto_content.append(self.auto_result)
        self.cal_btn.connect("clicked", self.confirm_calibration)
        self.cancel_btn.connect("clicked", lambda *_: self.cancel_event.set())

        manual_title = Gtk.Label(label="Manual power limits", xalign=0)
        manual_title.add_css_class("title-4")
        turbo_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        turbo_label = Gtk.Label(label="Disable Turbo Boost", xalign=0)
        turbo_label.set_hexpand(True)
        self.no_turbo = Gtk.Switch()
        self.no_turbo.set_valign(Gtk.Align.CENTER)
        turbo_row.append(turbo_label)
        turbo_row.append(self.no_turbo)
        manual_content.append(manual_title); manual_content.append(turbo_row)
        self.no_turbo.connect("notify::active", self.turbo_changed)

        grid = Gtk.Grid(column_spacing=18, row_spacing=10)
        manual_content.append(grid)
        self.pl1_label = Gtk.Label(xalign=0); self.pl2_label = Gtk.Label(xalign=0)
        self.tcc_label = Gtk.Label(xalign=0)
        self.max_freq_label = Gtk.Label(xalign=0)
        for label in (self.pl1_label, self.pl2_label, self.tcc_label, self.max_freq_label):
            label.set_width_chars(21)
            label.set_max_width_chars(21)
            label.add_css_class("monospace")
        self.pl1 = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 5, 125, 1)
        self.pl2 = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 5, 125, 1)
        self.tcc_target = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 60, 100, 1)
        self.max_freq = Gtk.Scale.new_with_range(
            Gtk.Orientation.HORIZONTAL, 400, 3500, FREQUENCY_STEP_MHZ
        )
        for scale in (self.pl1, self.pl2, self.tcc_target, self.max_freq): scale.set_hexpand(True); scale.set_draw_value(False)
        grid.attach(self.pl1_label, 0, 0, 1, 1); grid.attach(self.pl1, 1, 0, 1, 1)
        grid.attach(self.pl2_label, 0, 1, 1, 1); grid.attach(self.pl2, 1, 1, 1, 1)
        grid.attach(self.tcc_label, 0, 2, 1, 1); grid.attach(self.tcc_target, 1, 2, 1, 1)
        grid.attach(self.max_freq_label, 0, 3, 1, 1); grid.attach(self.max_freq, 1, 3, 1, 1)
        self.pl1.connect("value-changed", self.limit_changed, "pl1")
        self.pl2.connect("value-changed", self.limit_changed, "pl2")
        self.tcc_target.connect("value-changed", self.limit_changed, "tcc")
        self.max_freq.connect("value-changed", self.limit_changed, "frequency")
        self.tcc_target.set_tooltip_text(
            "Persistent CPU thermal throttling target. This changes only the TCC activation offset in MSR 0x1A2; the hardware TjMax protection remains active."
        )
        self.max_freq.set_tooltip_text(
            "Maximum CPU frequency applied to every cpufreq policy. The control is disabled when the kernel does not expose writable frequency limits."
        )

        actions = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        self.apply_btn = Gtk.Button(label="Apply settings")
        self.apply_btn.add_css_class("suggested-action")
        self.apply_btn.set_sensitive(False)
        self.restore_btn = Gtk.Button(label="Load system defaults")
        self.restore_btn.set_sensitive(False)
        self.persistence_btn = Gtk.Button(label="Enable persistence")
        self.persistence_btn.set_sensitive(False)
        actions.append(self.apply_btn)
        actions.append(self.restore_btn)
        actions.append(self.persistence_btn)
        manual_content.append(actions)
        self.apply_btn.connect("clicked", self.apply_settings)
        self.restore_btn.connect("clicked", self.restore_defaults)
        self.persistence_btn.connect("clicked", self.toggle_persistence)

        telemetry = Gtk.Grid(column_spacing=18, column_homogeneous=True)
        table_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        table_box.add_css_class("unified-box")
        table_box.add_css_class("padded-box")
        graph_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        telemetry.attach(table_box, 0, 0, 1, 1)
        telemetry.attach(graph_box, 1, 0, 1, 1)
        body.append(telemetry)

        table_title = Gtk.Label(label="Logical CPU status", xalign=0)
        table_title.add_css_class("title-4")
        table_box.append(table_title)
        self.cpu_table = Gtk.Grid(column_spacing=12, row_spacing=4)
        self.cpu_table.set_halign(Gtk.Align.CENTER)
        self.cpu_column_widths = (36, 58, 58, 42, 105)
        for column, heading in enumerate(("CPU", "MHz", "Temp", "dTj", "Status")):
            label = Gtk.Label(label=heading, xalign=1 if column < 4 else 0)
            label.set_size_request(self.cpu_column_widths[column], -1)
            label.add_css_class("heading")
            self.cpu_table.attach(label, column, 0, 1, 1)
        table_box.append(self.cpu_table)

        self.power_graph = HistoryGraph("Package power", " W", [(0.94, .55, .24), (.35, .68, .90), (.45, .75, .45)])
        self.temp_graph = HistoryGraph(
            "CPU package temperature", " °C", [(.92, .30, .25), (.95, .65, .20)], 105
        )
        self.power_graph.add_css_class("unified-box")
        self.temp_graph.add_css_class("unified-box")
        graph_box.append(self.power_graph); graph_box.append(self.temp_graph)
        throttle_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        override_label = Gtk.Label(label="PROCHOT override", xalign=0)
        self.throttle = Gtk.Label(xalign=0)
        self.throttle.set_wrap(True)
        self.prochot_override = Gtk.Switch()
        self.prochot_override.set_valign(Gtk.Align.CENTER)
        self.prochot_override.set_tooltip_text("Temporary diagnostic override; disabled automatically before auto-tuning")
        throttle_row.append(override_label)
        throttle_row.append(self.prochot_override)
        throttle_row.append(self.throttle)
        body.append(throttle_row)
        footer = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        footer.set_margin_start(8); footer.set_margin_end(8); footer.set_margin_bottom(8)
        donate = Gtk.LinkButton(uri="https://donate.stripe.com/eVq14n8a7agh2lQdqq14400", label="Fund our bugs")
        donate.add_css_class("donate-link"); footer.append(donate)
        spacer = Gtk.Box(hexpand=True); footer.append(spacer)
        footer.append(Gtk.Label(label=f"v{APP_VERSION}"))
        root.append(footer)
        self.prochot_override.connect("notify::active", self.prochot_override_changed)
        win.connect("close-request", self.close_requested)
        win.present()
        GLib.timeout_add_seconds(1, self.poll)
        self.poll()

    def close_requested(self, _window):
        if not self.calibrating:
            return False
        self.cancel_event.set()
        self.status.set_text("Cancelling calibration and restoring hardware state")
        return True

    def limit_changed(self, _scale, which):
        if not self.control_update:
            self.control_update = True
            if which == "pl1" and self.pl1.get_value() > self.pl2.get_value():
                self.pl1.set_value(self.pl2.get_value())
            elif which == "pl2" and self.pl2.get_value() < self.pl1.get_value():
                self.pl2.set_value(self.pl1.get_value())
            elif which == "frequency":
                self.max_freq.set_value(frequency_step_mhz(self.max_freq.get_value()))
            self.control_update = False
        self.pl1_label.set_text(f"PL1 sustained  {self.pl1.get_value():3.0f} W")
        self.pl2_label.set_text(f"PL2 burst      {self.pl2.get_value():3.0f} W")
        self.tcc_label.set_text(f"Thermal target {self.tcc_target.get_value():3.0f} °C")
        self.max_freq_label.set_text(f"Maximum frequency {self.max_freq.get_value():4.0f} MHz")
        if self.controls_initialized and not self.control_update and not self.calibrating:
            self.mark_settings_dirty()

    def run_helper(self, *args):
        result = command("pkexec", HELPER, *map(str, args))
        if args[0] == "apply":
            self.thermald_state = "disabled" if int(args[3]) else "stopped"
        elif args[0] in ("restore", "disable-persistence"):
            self.thermald_state = thermald_status()
        return result

    def show_notice(self, message, seconds=10):
        self.notice_until = time.monotonic() + seconds
        self.status.set_text(message)

    def mark_settings_dirty(self):
        if not self.controls_initialized or self.control_update:
            return
        self.settings_dirty = True
        self.apply_btn.set_sensitive(not self.calibrating)

    def apply_settings(self, *_):
        if self.calibrating:
            return
        enabling_prochot = self.prochot_override.get_active() and self.data.get("prochot_override") != "1"
        if enabling_prochot:
            dialog = Adw.MessageDialog.new(
                self.get_active_window(),
                "Enable PROCHOT override?",
                "PROCHOT is a hardware protection path for charger, battery and voltage-regulator faults. "
                "Overriding it can permit operation during an unsafe electrical condition.",
            )
            dialog.add_response("cancel", "Cancel")
            dialog.add_response("apply", "Apply and enable")
            dialog.set_default_response("cancel")
            dialog.set_close_response("cancel")
            dialog.set_response_appearance("apply", Adw.ResponseAppearance.DESTRUCTIVE)
            dialog.connect("response", lambda _dialog, response: self.commit_settings() if response == "apply" else None)
            dialog.present()
            return
        self.commit_settings()

    def commit_settings(self):
        settings = (
            int(self.pl1.get_value() * 1e6),
            int(self.pl2.get_value() * 1e6),
            int(self.data.get("persistent") == "1"),
            int(self.no_turbo.get_active()),
            int(self.tcc_target.get_value()) if self.data.get("tcc_locked") != "1" and self.data.get("tcc_programmable", "1") == "1" else 0,
            int(self.max_freq.get_value() * 1000) if self.data.get("freq_writable") == "1" else 0,
            int(self.prochot_override.get_active()),
        )
        self.set_settings_sensitive(False)
        self.status.set_text("Applying settings…")
        threading.Thread(target=self.apply_settings_worker, args=(settings,), daemon=True).start()

    def apply_settings_worker(self, settings):
        try:
            command("pkexec", HELPER, "apply", *map(str, settings))
            GLib.idle_add(self.finish_apply_settings, settings, None)
        except Exception as error:
            GLib.idle_add(self.finish_apply_settings, settings, str(error))

    def finish_apply_settings(self, settings, error):
        if error is not None:
            self.status.set_text(f"Apply failed: {error}")
            self.set_settings_sensitive(True)
            self.apply_btn.set_sensitive(True)
            return GLib.SOURCE_REMOVE
        _, _, persistent, no_turbo, tcc_target, max_freq_khz, prochot_override = settings
        self.thermald_state = "disabled" if persistent else "stopped"
        self.data["no_turbo"] = str(no_turbo)
        self.data["prochot_override"] = str(prochot_override)
        if tcc_target:
            self.data["tcc_target"] = str(tcc_target)
        self.data["max_freq_khz"] = str(max_freq_khz)
        self.data["persistent"] = str(persistent)
        self.update_persistence_button()
        self.settings_dirty = False
        self.set_settings_sensitive(True)
        self.apply_btn.set_sensitive(False)
        persistence = "enabled" if persistent else "disabled"
        self.show_notice(
            f"Settings applied · maximum frequency {max_freq_khz // 1000} MHz · power-limit persistence {persistence}"
        )
        return GLib.SOURCE_REMOVE

    def set_settings_sensitive(self, sensitive):
        for widget in (
            self.pl1, self.pl2, self.tcc_target, self.max_freq, self.no_turbo,
            self.prochot_override, self.restore_btn, self.persistence_btn,
        ):
            widget.set_sensitive(sensitive)
        if sensitive and self.data:
            self.tcc_target.set_sensitive(
                self.data.get("tcc_locked") != "1" and
                self.data.get("tcc_programmable", "1") == "1"
            )
            self.max_freq.set_sensitive(self.data.get("freq_writable") == "1")

    def update_persistence_button(self):
        persistent = self.data.get("persistent") == "1"
        self.persistence_btn.set_label("Disable persistence" if persistent else "Enable persistence")
        self.persistence_btn.set_sensitive(not self.calibrating)

    def toggle_persistence(self, *_):
        if self.calibrating:
            return
        enable = self.data.get("persistent") != "1"
        self.set_settings_sensitive(False)
        self.status.set_text("Enabling persistence…" if enable else "Disabling persistence…")
        threading.Thread(target=self.persistence_worker, args=(enable,), daemon=True).start()

    def persistence_worker(self, enable):
        try:
            if enable:
                command("pkexec", HELPER, "enable-persistence")
            else:
                command("pkexec", HELPER, "disable-persistence")
            GLib.idle_add(self.finish_persistence, enable, None)
        except Exception as error:
            GLib.idle_add(self.finish_persistence, enable, str(error))

    def finish_persistence(self, enabled, error):
        self.set_settings_sensitive(True)
        if error is not None:
            self.status.set_text(f"Could not change persistence: {error}")
            return GLib.SOURCE_REMOVE
        self.data["persistent"] = "1" if enabled else "0"
        self.thermald_state = "disabled" if enabled else thermald_status()
        self.update_persistence_button()
        self.show_notice("Persistence enabled" if enabled else "Persistence disabled")
        return GLib.SOURCE_REMOVE

    def set_no_turbo(self, disabled):
        self.control_update = True
        self.no_turbo.set_active(disabled)
        self.control_update = False

    def turbo_changed(self, switch, _property):
        if not self.controls_initialized or self.control_update or self.calibrating:
            return
        self.mark_settings_dirty()

    def set_prochot_override(self, enabled):
        self.prochot_control_update = True
        self.prochot_override.set_active(enabled)
        self.prochot_control_update = False

    def apply_prochot_override(self, enabled):
        try:
            self.run_helper("prochot-override", int(enabled))
            self.data["prochot_override"] = "1" if enabled else "0"
            self.set_prochot_override(enabled)
            state = "enabled" if enabled else "disabled"
            self.show_notice(f"PROCHOT override {state}")
            return True
        except Exception as error:
            current = self.data.get("prochot_override") == "1"
            self.set_prochot_override(current)
            self.status.set_text(f"Could not change PROCHOT override: {error}")
            return False

    def prochot_override_changed(self, switch, _property):
        if self.prochot_control_update or not self.controls_initialized:
            return
        self.mark_settings_dirty()

    def prochot_override_warning_response(self, _dialog, response):
        if response == "enable":
            self.apply_prochot_override(True)
        else:
            current = self.data.get("prochot_override") == "1"
            self.set_prochot_override(current)
        self.prochot_change_pending = False

    def set_sliders(self, pl1, pl2, tcc_target=None, max_freq_mhz=None):
        self.control_update = True
        self.pl1.set_value(pl1)
        self.pl2.set_value(pl2)
        if tcc_target is not None:
            self.tcc_target.set_value(tcc_target)
        if max_freq_mhz is not None:
            self.max_freq.set_value(frequency_step_mhz(max_freq_mhz))
        self.control_update = False

    def cancel_manual_apply(self):
        pass

    def update_cpu_table(self, cores, tjmax):
        while len(self.cpu_rows) < len(cores):
            row_number = len(self.cpu_rows) + 1
            labels = [Gtk.Label(xalign=1 if column < 4 else 0) for column in range(5)]
            for column, label in enumerate(labels):
                label.set_size_request(self.cpu_column_widths[column], -1)
                label.add_css_class("monospace")
                self.cpu_table.attach(label, column, row_number, 1, 1)
            self.cpu_rows.append(labels)
        for labels, core in zip(self.cpu_rows, cores):
            cpu, mhz, temp, thermal, prochot = core
            if prochot:
                state = "PROCHOT"
            elif thermal:
                state = "thermal"
            else:
                state = "ok"
            values = (str(cpu), str(mhz), f"{temp} °C", str(tjmax - temp), state)
            for label, value in zip(labels, values):
                label.set_text(value)
                label.remove_css_class("error")
                if state != "ok":
                    label.add_css_class("error")

    def restore_defaults(self, *_):
        try:
            data = self.data
            self.set_sliders(
                int(data["defaults_pl1_uw"]) / 1e6,
                int(data["defaults_pl2_uw"]) / 1e6,
                int(data.get("defaults_tcc_target", data.get("tjmax", 100))),
                int(data.get("defaults_max_freq_khz", data.get("freq_max_khz", 0))) / 1000,
            )
            self.set_no_turbo(data.get("defaults_no_turbo") == "1")
            self.set_prochot_override(False)
            self.mark_settings_dirty()
            self.show_notice("System defaults loaded. Click Apply settings to write them.")
        except Exception as e: self.status.set_text(f"Could not load system defaults: {e}")

    def poll(self):
        try:
            data = read_status(); now = time.monotonic(); energy = int(data.get("energy_uj", 0)); maxe = int(data.get("max_energy_uj", 1))
            watts = None
            if self.last_energy is not None:
                delta = energy - self.last_energy
                if delta < 0: delta += maxe
                watts = delta / 1e6 / (now - self.last_energy_time)
            self.last_energy, self.last_energy_time = energy, now
            self.data = data; cores = data["cores"]
            active_pl1 = int(data["pl1_uw"]) / 1e6
            active_pl2 = int(data["pl2_uw"]) / 1e6
            if not self.controls_initialized:
                base = max(5, int(data.get("base_uw", 5000000))/1e6)
                ceiling = max(
                    base + 10,
                    int(data.get("ceiling_uw", 0)) / 1e6,
                    int(data["pl1_uw"]) / 1e6,
                    int(data["pl2_uw"]) / 1e6,
                    int(data.get("defaults_pl1_uw", 0)) / 1e6,
                    int(data.get("defaults_pl2_uw", 0)) / 1e6,
                )
                for s in (self.pl1, self.pl2): s.set_range(5, ceiling)
                tjmax = int(data.get("tjmax", 100))
                self.tcc_target.set_range(max(60, tjmax - 63), tjmax)
                freq_min_mhz = int(data.get("freq_min_khz", 0)) / 1000
                freq_max_mhz = int(data.get("freq_max_khz", 0)) / 1000
                if freq_min_mhz > 0 and freq_max_mhz >= freq_min_mhz:
                    self.max_freq.set_range(freq_min_mhz, freq_max_mhz)
                self.set_sliders(
                    int(data["pl1_uw"])/1e6,
                    int(data["pl2_uw"])/1e6,
                    int(data.get("tcc_target", tjmax)),
                    int(data.get("max_freq_khz", 0)) / 1000,
                )
                self.tcc_target.set_sensitive(
                    data.get("tcc_locked") != "1" and data.get("tcc_programmable", "1") == "1"
                )
                self.max_freq.set_sensitive(data.get("freq_writable") == "1")
                self.set_no_turbo(data.get("no_turbo") == "1")
                self.update_persistence_button()
                self.set_prochot_override(False)
                if data.get("defaults_pl1_uw") and data.get("defaults_pl2_uw"):
                    defaults_pl1 = int(data["defaults_pl1_uw"]) / 1e6
                    defaults_pl2 = int(data["defaults_pl2_uw"]) / 1e6
                    self.restore_btn.set_label(
                        f"Load system defaults ({defaults_pl1:.0f}/{defaults_pl2:.0f} W)"
                    )
                    self.restore_btn.set_sensitive(True)
                base_name = "cTDP-down" if data.get("base_kind") == "ctdp-down" else "Package TDP"
                base_watts = int(data.get("base_uw", 0)) / 1e6
                self.auto_hint.set_text(
                    f"PL1 stays at {base_watts:.0f} W ({base_name}); "
                    f"PL2 is tested in 5 W steps."
                )
                self.controls_initialized = True
                if data.get("prochot_override") == "1":
                    self.mark_settings_dirty()
                    self.show_notice("PROCHOT override is active in hardware; Apply settings will disable it", 30)
            self.summary.set_text(data.get("model", "Intel CPU"))
            avg_freq = statistics.mean([c[1] for c in cores]) if cores else 0
            max_temp = max([c[2] for c in cores], default=0)
            package_temp = int(data.get("package_temp", max_temp))
            self.update_cpu_table(cores, int(data.get("tjmax", 100)))
            if not self.calibrating and time.monotonic() >= self.notice_until:
                pending = "  ·  Unapplied settings" if self.settings_dirty else ""
                self.status.set_text(
                    f"Power {watts or 0:6.1f} W  |  Average {avg_freq:4.0f} MHz  |  "
                    f"Package {package_temp:3d} °C / target {int(data.get('tcc_target', data.get('tjmax', 100))):3d} °C  |  "
                    f"thermald {self.thermald_state:<8}{pending}"
                )
            self.history_power.append(watts); self.history_package_temp.append(package_temp)
            pl1=float(data["pl1_uw"])/1e6; pl2=float(data["pl2_uw"])/1e6
            self.power_graph.update([("Power", list(self.history_power)), ("PL1", [pl1]*len(self.history_power)), ("PL2", [pl2]*len(self.history_power))])
            tcc_target = int(data.get("tcc_target", data.get("tjmax", 100)))
            self.temp_graph.update([
                ("Package", list(self.history_package_temp)),
                ("Target", [tcc_target] * len(self.history_package_temp)),
            ], package_temp)
            reasons=[]; perf=int(data.get("perf_active",0))
            thermal_cores=[str(c[0]) for c in cores if c[3]]
            prochot_cores=[str(c[0]) for c in cores if c[4]]
            if data.get("prochot_external")=="1":
                detail=[]
                if data.get("smc_adapter_current_ua"): detail.append(f"adapter {int(data['smc_adapter_current_ua'])/1e6:.2f} A")
                if data.get("smc_battery_current_ua"): detail.append(f"battery {int(data['smc_battery_current_ua'])/1e6:.2f} A")
                if data.get("smc_cpu_power_uw"): detail.append(f"SMC CPU {int(data['smc_cpu_power_uw'])/1e6:.0f} W")
                reasons.append("Platform PROCHOT inferred" + (f" ({', '.join(detail)})" if detail else ""))
            elif prochot_cores: reasons.append("PROCHOT CPU " + ",".join(prochot_cores))
            if thermal_cores: reasons.append("thermal CPU " + ",".join(thermal_cores))
            if perf & (1<<10): reasons.append("PL1")
            if perf & (1<<11): reasons.append("PL2")
            self.throttle.set_text("Active throttle: " + (", ".join(reasons) or "none") + f"    Logged bits: 0x{int(data.get('perf_log',0)):x}")
        except Exception as e: self.status.set_text(f"Telemetry unavailable: {e}")
        return True

    def set_calibration_ui(self, active):
        self.calibrating=active
        for w in (self.cal_btn,self.pl1,self.pl2,self.tcc_target,self.max_freq,self.no_turbo,self.prochot_override,self.persistence_btn): w.set_sensitive(not active)
        self.apply_btn.set_sensitive(not active and self.settings_dirty)
        if not active and self.data:
            self.tcc_target.set_sensitive(
                self.data.get("tcc_locked") != "1" and self.data.get("tcc_programmable", "1") == "1"
            )
            self.max_freq.set_sensitive(self.data.get("freq_writable") == "1")
        self.restore_btn.set_sensitive(
            not active and bool(self.data.get("defaults_pl1_uw")) and bool(self.data.get("defaults_pl2_uw"))
        )
        self.cancel_btn.set_sensitive(active)

    def confirm_calibration(self, *_):
        if self.settings_dirty:
            self.status.set_text("Apply or discard the pending settings before auto-tuning")
            return
        if self.prochot_override.get_active() and not self.apply_prochot_override(False):
            self.status.set_text("Auto-tuning cancelled: PROCHOT override could not be disabled")
            return
        dialog = Adw.MessageDialog.new(
            self.get_active_window(),
            "Auto-tune CPU power limits?",
            "PL1 remains fixed at the detected base power: cTDP-down when the processor exposes it, otherwise "
            "Package TDP. PL2 starts at the same value and increases in 5 W steps. "
            "Each PL2 value runs the same kernel compilation workload for 30 seconds. The matching Fedora "
            "kernel source is downloaded and cached before the first run. When a value triggers PROCHOT, the previous successful "
            "PL2 value is applied immediately. The result remains active until reboot. "
            "After auto-tuning completes, enable persistence "
            "to keep the result. "
            "Administrator authentication is required to control the fans and power limits.",
        )
        dialog.set_default_size(480, 480)
        dialog.add_response("cancel", "Cancel")
        dialog.add_response("calibrate", "Start auto-tuning")
        dialog.set_default_response("calibrate")
        dialog.set_close_response("cancel")
        dialog.set_response_appearance("calibrate", Adw.ResponseAppearance.SUGGESTED)
        dialog.connect("response", self.calibration_response)
        dialog.present()

    def calibration_response(self, _dialog, response):
        if response == "calibrate":
            self.start_calibration()

    def start_calibration(self):
        if self.calibrating or not self.data: return
        if self.prochot_override.get_active() and not self.apply_prochot_override(False):
            self.status.set_text("Auto-tuning cancelled: PROCHOT override could not be disabled")
            return
        self.calibration_persist = int(self.data.get("persistent") == "1")
        self.cancel_event.clear(); self.set_calibration_ui(True)
        self.auto_result.set_text("Auto-tuning in progress")
        self.status.set_text("Waiting for administrator authorization")
        threading.Thread(target=self.calibrate, daemon=True).start()

    def run_until_prochot(self, duration, label):
        proc = subprocess.Popen([
            BENCHMARK, "run",
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        started = time.monotonic()
        next_sample = started
        prochot = False
        completed_window = False
        try:
            while proc.poll() is None:
                if self.cancel_event.is_set():
                    os.killpg(proc.pid, signal.SIGTERM)
                    break
                now = time.monotonic()
                if now - started >= duration:
                    completed_window = True
                    os.killpg(proc.pid, signal.SIGTERM)
                    break
                if now < next_sample:
                    time.sleep(min(next_sample - now, 0.05))
                    continue
                status = read_status()
                prochot = (
                    status.get("prochot") == "1" or
                    any(core[4] for core in status["cores"])
                )
                elapsed = min(duration, int(time.monotonic() - started) + 1)
                GLib.idle_add(self.status.set_text, f"{label} · {elapsed}/{duration} s")
                if prochot:
                    os.killpg(proc.pid, signal.SIGTERM)
                    break
                next_sample = time.monotonic() + .25
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
        if not completed_window and not prochot and not self.cancel_event.is_set():
            raise RuntimeError("kernel compilation benchmark failed")
        return prochot

    def prepare_benchmark(self):
        proc = subprocess.Popen(
            [BENCHMARK, "prepare"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        assert proc.stdout is not None
        last_message = ""
        for line in proc.stdout:
            message = line.strip()
            if message:
                last_message = message
                GLib.idle_add(self.status.set_text, message)
        if proc.wait() != 0:
            raise RuntimeError(last_message or "kernel source preparation failed")

    def calibrate(self):
        original=(int(self.data["pl1_uw"]),int(self.data["pl2_uw"]),
                  int(self.data.get("persistent") == "1"),int(self.data.get("no_turbo") == "1"),
                  int(self.data.get("tcc_target", self.data.get("tjmax", 100))) if self.data.get("tcc_locked") != "1" and self.data.get("tcc_programmable", "1") == "1" else 0,
                  int(self.data.get("max_freq_khz", self.data.get("freq_max_khz", 0))) if self.data.get("freq_writable") == "1" else 0)
        down=max(5,round(int(self.data.get("base_uw",35000000))/1e6))
        ceiling=max(down+10,round(int(self.data.get("ceiling_uw",(down+10)*1000000))/1e6))
        try:
            self.prepare_benchmark()
            self.run_helper("fans-max")
            candidates=list(range(down,ceiling+1,5))
            if candidates[-1] != ceiling:
                candidates.append(ceiling)
            selected_pl2=down
            last_passing_pl2=None
            for pl2 in candidates:
                if self.cancel_event.is_set(): break
                self.run_helper("apply",down*1000000,pl2*1000000,0,0,original[4],original[5])
                GLib.idle_add(self.set_no_turbo,False)
                prochot=self.run_until_prochot(30,f"Testing {down}/{pl2} W")
                if self.cancel_event.is_set(): break
                if prochot:
                    selected_pl2=last_passing_pl2 if last_passing_pl2 is not None else down
                    break
                last_passing_pl2=pl2
                selected_pl2=pl2
            if not self.cancel_event.is_set():
                pl1=down; pl2=selected_pl2
                self.run_helper("apply",pl1*1000000,pl2*1000000,self.calibration_persist,original[3],original[4],original[5])
                GLib.idle_add(self.set_sliders,pl1,pl2)
                GLib.idle_add(self.set_no_turbo,bool(original[3]))
                persistence="enabled" if self.calibration_persist else "disabled"
                GLib.idle_add(self.auto_result.set_text,f"Applied result: PL1 {pl1} W, PL2 {pl2} W; persistence {persistence}")
                GLib.idle_add(self.show_notice,f"Auto-tuning complete and applied: {pl1}/{pl2} W · persistence {persistence}",15)
            else:
                self.run_helper("apply",*original)
                GLib.idle_add(self.auto_result.set_text,"Auto-tuning cancelled; previous limits restored")
        except Exception as e:
            try: self.run_helper("apply",*original)
            except Exception: pass
            message=f"Auto-tuning failed: {e}. Previous limits restored"
            GLib.idle_add(self.auto_result.set_text,message)
            GLib.idle_add(self.show_notice,message,30)
        finally:
            try: self.run_helper("fans-restore")
            except Exception: pass
            GLib.idle_add(self.set_calibration_ui,False)


if __name__ == "__main__":
    raise SystemExit(CpuControl().run())

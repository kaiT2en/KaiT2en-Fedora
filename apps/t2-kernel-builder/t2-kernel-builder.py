#!/usr/bin/python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
import json
import os
import re
import signal
import shutil
import subprocess
import threading
import time
import urllib.request
import uuid
from pathlib import Path

import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, GdkPixbuf, Gio, GLib, GObject, Gtk

APP_ID = "org.t2kernelbuilder.gtk"
APP_VERSION = "0.03"

def kait2en_brand():
    # White type for dark surfaces, dark type for light ones. Without the
    # light file the white one is used.
    dark = GdkPixbuf.Pixbuf.new_from_file("/usr/local/share/kait2en/kait2en-wordmark.png")
    try:
        light = GdkPixbuf.Pixbuf.new_from_file("/usr/local/share/kait2en/kait2en-wordmark-light.png")
    except GLib.Error:
        light = dark
    manager = Adw.StyleManager.get_default()
    brand = Gtk.DrawingArea(content_width=80, content_height=21)
    def draw(_area, context, width, height):
        pixbuf = dark if manager.get_dark() else light
        scale = min(width / pixbuf.get_width(), height / pixbuf.get_height())
        context.save()
        context.translate((width - pixbuf.get_width() * scale) / 2,
                          (height - pixbuf.get_height() * scale) / 2)
        context.scale(scale, scale)
        Gdk.cairo_set_source_pixbuf(context, pixbuf, 0, 0)
        context.paint(); context.restore()
    brand.set_draw_func(draw); brand.set_size_request(80, 21)
    manager.connect("notify::dark", lambda *_: brand.queue_draw())
    return brand
HERE = Path(__file__).resolve().parent
SOURCE_ENGINE = HERE / "engine"
INSTALLED_ENGINE = Path("/usr/local/libexec/t2-kernel-builder")
ENGINE = SOURCE_ENGINE if (SOURCE_ENGINE / "build.sh").is_file() else INSTALLED_ENGINE
BUILD_SCRIPT = ENGINE / "build.sh"
CLEANUP_HELPER = (HERE / "t2-kernel-builder-cleanup" if (HERE / "t2-kernel-builder-cleanup").is_file()
                  else Path("/usr/local/libexec/t2-kernel-builder-cleanup"))
WORK_DIR = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "t2-kernel-builder" / "build"
EMPTY_PATCH_DIR = WORK_DIR.parent / "empty-patches"
CONFIG_DIR = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "t2-kernel-builder"
UI_STATE = CONFIG_DIR / "ui-state.json"
SETTINGS_FILE = CONFIG_DIR / "settings.json"

T2_REQUIRED = [
    "ACPI", "ACPI_TAD", "APPLE_MFI_FASTCHARGE", "BLK_DEV_NVME", "BT",
    "CFG80211", "DRM", "DRM_APPLETBDRM", "DRM_I915", "EFI", "EFI_STUB",
    "EXT4_FS", "HID", "HID_APPLE", "HID_APPLETB_BL", "HID_APPLETB_KBD",
    "HID_MAGICMOUSE", "INPUT", "INPUT_EVDEV", "INPUT_SPARSEKMAP",
    "MAC80211", "NET", "PCI", "PCIEPORTBUS", "SND", "SND_HDA_INTEL",
    "SENSORS_APPLESMC", "SOUND", "TMPFS", "USB", "USB4", "USB_SUPPORT",
    "USB_XHCI_HCD", "VFAT_FS", "WLAN",
]

SECTION_HELP = {
    "Kernel source": ("Kernel source", "Select a Fedora kernel release. The read-only version is combined with the editable suffix to form the installed kernel name."),
    "Base configuration": ("Base configuration", "Fedora default keeps Fedora's complete x86_64 configuration. localmodconfig removes options that are not needed by the modules currently loaded on this Mac. Connect every device you need before running localmodconfig."),
    "Optional components": ("Optional components", "After either base configuration has been prepared, this tree exposes the kernel's real Kconfig hierarchy. T2 requirements are locked; other visible bool and tristate symbols can be changed."),
    "Patch series": ("Patch series", "Every readable *.patch file in the selected folder is applied alphabetically after Fedora's own patch set. Preparation stops on a missing or incompatible patch."),
    "Build kernel": ("Build kernel", "Compile the configured kernel as your normal user. Choose the number of parallel build threads; the default leaves one hardware thread available for the desktop. Building does not modify the running system."),
    "Install kernel": ("Install kernel", "Select a completed build and install it separately when you are ready. Installation requests administrator authorization through PolicyKit and creates the boot files for that kernel."),
    "Installed kernels": ("Installed kernels", "The running kernel and rescue images are protected. Package-managed kernels are removed with DNF; locally installed custom kernels are removed from /boot and /lib/modules."),
    "Builder data": ("Builder data", "Delete the cache and downloaded kernel sources for selected versions. Keep entries that you want to build again without downloading and preparing their sources."),
}

class KconfigItem(GObject.Object):
    def __init__(self, node):
        super().__init__(); self.node = node

def palette_css(dark):
    if dark:
        window, fg, panel, shadow = "#151517", "#e8e8e8", "#0d0d0f", "rgba(0,0,0,0.32)"
        rim, line = "rgba(255,255,255,0.24)", "rgba(255,255,255,0.06)"
    else:
        window, fg, panel, shadow = "#ffffff", "#242424", "#e2e2e8", "rgba(0,0,0,0.14)"
        rim, line = "rgba(255,255,255,0.70)", "rgba(0,0,0,0.08)"
    return (f"window{{color:alpha({fg},0.72);}}"
            f"button,button label,entry,spinbutton,dropdown{{color:alpha({fg},0.72);}}"
            f".monospace{{font-family:'JetBrains Mono';}}"
            f"window.csd{{border-top:1px solid {rim};}}"
            f"window.csd.maximized,window.csd.tiled,window.csd.tiled-top,window.csd.tiled-left,"
            f"window.csd.tiled-right,window.csd.tiled-bottom,window.csd.fullscreen{{border-top-width:0;}}"
            f".app-background{{background:{window};color:alpha({fg},0.72);}}"
            f".app-background headerbar{{background:{window};color:alpha({fg},0.72);}}"
            f".app-background .title-1,.app-background .title-2,.app-background .title-3,.app-background .title-4,.app-background .title,.app-background .heading,windowtitle .title{{color:{fg};font-weight:600;}}"
            f".app-background .dim-label,windowtitle .subtitle{{color:alpha({fg},0.72);}}"
            f".unified-box{{background:{panel};color:alpha({fg},0.72);border-radius:12px;padding:14px;box-shadow:inset 0 0 0 1px {line};}}"
            f".unified-box textview,.unified-box textview>text{{background:transparent;}}"
            f".sticky-bar{{background:{panel};color:alpha({fg},0.72);padding:9px 18px;box-shadow:0 -5px 12px {shadow};}}"
            f".donate-link,.donate-link>label{{color:alpha({fg},0.72);}}")

class KernelBuilder(Adw.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID)
        self.connect("activate", self.activate)
        self.process = None
        self.running = False
        self.prepared = False
        self.base_mode = None
        self.kernel_values = []
        self.kernel_tree = None
        self.kconfig = None
        self.command_output = []
        self.pending_config_values = {}
        self.config_overrides = {}
        self.progress_timer = 0
        self.cleanup_running = False
        self.build_after_prepare = False
        self.built_tree = None
        self.built_release = None
        self.cancel_file = None
        self.updating_kernel_model = False
        self.available_jobs = max(1, os.cpu_count() or 1)
        self.saved_settings = self.load_settings()
        self.pending_config_values = dict(self.saved_settings.get("config_values", {}))

    def activate(self, _app):
        ui_state = self.load_ui_state()
        self.win = Adw.ApplicationWindow(application=self, title="T2 Kernel Builder")
        self.win.set_default_size(ui_state.get("width", 1340), ui_state.get("height", 760))
        manager = Adw.StyleManager.get_default()
        css = Gtk.CssProvider()
        css.load_from_string(palette_css(manager.get_dark()))
        Gtk.StyleContext.add_provider_for_display(self.win.get_display(), css,
                                                   Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        manager.connect("notify::dark", lambda m, _p: css.load_from_string(palette_css(m.get_dark())))

        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        root.add_css_class("app-background")
        header = Adw.HeaderBar()
        header.set_title_widget(Adw.WindowTitle(title="Kernel Builder",
                                                subtitle="Minimal Fedora kernels for T2 Macs"))
        brand = kait2en_brand()
        brand.set_margin_start(10); brand.set_margin_end(10); header.pack_start(brand)
        root.append(header)

        self.stack = Gtk.Stack(vexpand=True, transition_type=Gtk.StackTransitionType.SLIDE_LEFT_RIGHT)
        self.stack.set_hhomogeneous(False)
        switcher = Gtk.StackSwitcher(stack=self.stack)
        switcher.set_margin_top(8); switcher.set_margin_bottom(8)
        switcher.set_halign(Gtk.Align.CENTER)
        left = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, hexpand=True)
        left.set_size_request(280, -1)
        left.append(switcher); left.append(self.stack)
        log_panel = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        log_panel.add_css_class("unified-box")
        log_panel.set_hexpand(True); log_panel.set_size_request(280, -1)
        log_panel.set_margin_top(10); log_panel.set_margin_bottom(10); log_panel.set_margin_end(18)
        self.log = Gtk.TextView(editable=False, cursor_visible=False, monospace=True,
                                wrap_mode=Gtk.WrapMode.NONE)
        self.log_end_mark = self.log.get_buffer().create_mark(None,
                                                               self.log.get_buffer().get_end_iter(), False)
        log_scroll = Gtk.ScrolledWindow(vexpand=True)
        log_scroll.set_policy(Gtk.PolicyType.AUTOMATIC, Gtk.PolicyType.AUTOMATIC)
        log_scroll.set_propagate_natural_width(False)
        log_scroll.set_child(self.log); log_panel.append(log_scroll)
        self.workspace = Gtk.Paned(orientation=Gtk.Orientation.HORIZONTAL, vexpand=True,
                                   wide_handle=True, position=ui_state.get("width", 1340) // 2)
        self.workspace.set_start_child(left); self.workspace.set_end_child(log_panel)
        self.workspace.set_resize_start_child(True); self.workspace.set_resize_end_child(True)
        self.workspace.set_shrink_start_child(True); self.workspace.set_shrink_end_child(False)
        root.append(self.workspace)

        source_page = self.page()
        source = self.section(source_page, "Kernel source", help_key="Kernel source")
        self.source_detail = Gtk.Label(xalign=0)
        self.source_detail.add_css_class("dim-label")
        row = Gtk.Grid(column_spacing=8, column_homogeneous=True)
        self.kernel_model = Gtk.StringList.new(["Loading available kernels…"])
        self.kernel_drop = Gtk.DropDown(model=self.kernel_model, hexpand=True)
        dropdown_factory = Gtk.SignalListItemFactory()
        dropdown_factory.connect("setup", self.setup_kernel_choice)
        dropdown_factory.connect("bind", self.bind_kernel_choice)
        self.kernel_drop.set_factory(dropdown_factory)
        self.kernel_drop.set_list_factory(dropdown_factory)
        self.kernel_drop.connect("notify::selected", self.kernel_selection_changed)
        row.attach(self.kernel_drop, 0, 0, 1, 1)
        saved_suffix = str(self.saved_settings.get("suffix", "-t2-custom"))
        self.suffix = Gtk.Entry(text=saved_suffix, hexpand=True)
        self.suffix.set_placeholder_text("custom-suffix")
        self.suffix.connect("changed", self.suffix_changed)
        row.attach(self.suffix, 1, 0, 1, 1)
        source.append(row); source.append(self.source_detail)
        self.stack.add_titled(source_page, "source", "1. Source")

        config_page = self.page()
        config = self.section(config_page, "1. Base configuration", help_key="Base configuration")
        local_row = Gtk.Box(spacing=10)
        self.fedora_btn = Gtk.Button(label="Use Fedora default configuration")
        self.fedora_btn.set_tooltip_text("Keep Fedora's full x86_64 kernel configuration")
        self.fedora_btn.connect("clicked", lambda *_: self.prepare("fedora", base_action=True))
        self.local_btn = Gtk.Button(label="Run localmodconfig")
        self.local_btn.add_css_class("suggested-action")
        self.local_btn.set_tooltip_text("Create a minimal configuration from currently loaded modules")
        self.local_btn.connect("clicked", self.confirm_localmod)
        self.local_state = Gtk.Label(label="Not completed", xalign=0, hexpand=True)
        self.local_state.set_ellipsize(3)
        self.local_state.add_css_class("dim-label")
        local_row.append(self.fedora_btn); local_row.append(self.local_btn); local_row.append(self.local_state); config.append(local_row)

        features = self.section(config_page, "2. Optional components", help_key="Optional components")
        tree_actions = Gtk.Box(spacing=8)
        self.config_search = Gtk.SearchEntry(placeholder_text="Search symbol or description", hexpand=True,
                                             sensitive=False)
        self.config_search.connect("search-changed", lambda *_: self.rebuild_kconfig_tree())
        self.config_help = Gtk.Button(label="Selected item help", sensitive=False)
        self.config_help.connect("clicked", lambda *_: self.show_kconfig_help())
        tree_actions.append(self.config_search); tree_actions.append(self.config_help); features.append(tree_actions)
        self.component_tree = Gtk.ScrolledWindow(vexpand=True, min_content_height=310, sensitive=False)
        self.component_tree.set_policy(Gtk.PolicyType.AUTOMATIC, Gtk.PolicyType.AUTOMATIC)
        self.component_tree.set_propagate_natural_width(False)
        self.component_placeholder = Gtk.Label(label="Prepare a base configuration to load the Kconfig tree.",
                                               xalign=0, yalign=0)
        self.component_placeholder.set_margin_top(8)
        self.component_tree.set_child(self.component_placeholder); features.append(self.component_tree)
        patch_page = self.page()
        patches = self.section(patch_page, "Patch series", help_key="Patch series")
        self.patch_entry = self.entry_row(patches, "Patch folder", self.saved_settings.get("patch_folder", ""), self.patch_folder_changed)
        patch_buttons = Gtk.Box(spacing=8)
        choose = Gtk.Button(label="Choose folder")
        choose.connect("clicked", self.choose_patch_folder)
        patch_buttons.append(choose); patches.append(patch_buttons)
        patch_contents_title = Gtk.Label(label="Patch folder contents", xalign=0)
        patch_contents_title.add_css_class("dim-label"); patches.append(patch_contents_title)
        patch_contents_frame = Gtk.Frame()
        self.patch_contents = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        for side in ("top", "bottom", "start", "end"):
            getattr(self.patch_contents, f"set_margin_{side}")(10)
        patch_contents_frame.set_child(self.patch_contents); patches.append(patch_contents_frame)
        self.stack.add_titled(patch_page, "patches", "2. Patches")
        self.stack.add_titled(config_page, "configuration", "3. Config")

        build_page = self.page()
        build = self.section(build_page, "Build kernel", help_key="Build kernel")
        jobs_row = Gtk.Box(spacing=10)
        jobs_row.append(Gtk.Label(label="Parallel build threads", xalign=0, hexpand=True))
        default_jobs = max(1, self.available_jobs - 1)
        remembered_jobs = self.saved_settings.get("jobs", default_jobs)
        try: remembered_jobs = int(remembered_jobs)
        except (TypeError, ValueError): remembered_jobs = default_jobs
        remembered_jobs = min(self.available_jobs, max(1, remembered_jobs))
        self.jobs_spin = Gtk.SpinButton.new_with_range(1, self.available_jobs, 1)
        self.jobs_spin.set_value(remembered_jobs)
        self.jobs_spin.set_tooltip_text(f"Available hardware threads: {self.available_jobs}")
        self.jobs_spin.connect("value-changed", lambda *_: self.save_settings())
        jobs_row.append(self.jobs_spin); build.append(jobs_row)
        controls = Gtk.Box(spacing=8)
        self.build_btn = Gtk.Button(label="Build kernel")
        self.build_btn.add_css_class("suggested-action")
        self.build_btn.connect("clicked", lambda *_: self.start_build())
        self.cancel_btn = Gtk.Button(label="Cancel", sensitive=False)
        self.cancel_btn.connect("clicked", lambda *_: self.cancel())
        controls.append(self.build_btn); controls.append(self.cancel_btn)
        build.append(controls)
        install = self.section(build_page, "Install kernel", help_key="Install kernel")
        install.append(Gtk.Label(label="Completed build", xalign=0))
        self.built_model = Gtk.StringList.new(["No completed builds found"])
        self.built_drop = Gtk.DropDown(model=self.built_model, hexpand=True)
        self.built_drop.connect("notify::selected", self.built_selection_changed)
        install.append(self.built_drop)
        self.install_btn = Gtk.Button(label="Install built kernel", sensitive=False)
        self.install_btn.connect("clicked", lambda *_: self.start_install())
        self.install_btn.set_halign(Gtk.Align.START)
        install.append(self.install_btn)
        self.stack.add_titled(build_page, "build", "4. Build")

        cleanup_page = self.page()
        kernels = self.section(cleanup_page, "Installed kernels", help_key="Installed kernels")
        self.cleanup_kernel_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=5)
        kernel_scroll = Gtk.ScrolledWindow(vexpand=True, min_content_height=150)
        kernel_scroll.set_child(self.cleanup_kernel_box); kernels.append(kernel_scroll)
        kernel_actions = Gtk.Box(spacing=8)
        self.remove_kernels_btn = Gtk.Button(label="Remove selected kernels")
        self.remove_kernels_btn.add_css_class("destructive-action")
        self.remove_kernels_btn.connect("clicked", lambda *_: self.confirm_cleanup("kernels"))
        kernel_actions.append(self.remove_kernels_btn); kernels.append(kernel_actions)

        cache = self.section(cleanup_page, "Downloaded sources and build data", help_key="Builder data")
        self.cleanup_cache_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=5)
        cache_scroll = Gtk.ScrolledWindow(vexpand=True, min_content_height=150)
        cache_scroll.set_child(self.cleanup_cache_box); cache.append(cache_scroll)
        cache_actions = Gtk.Box(spacing=8)
        self.remove_cache_btn = Gtk.Button(label="Delete selected cache and sources")
        self.remove_cache_btn.set_tooltip_text("Delete all cached files and downloaded sources for the selected kernel versions")
        self.remove_cache_btn.add_css_class("destructive-action")
        self.remove_cache_btn.connect("clicked", lambda *_: self.confirm_cleanup("cache"))
        cache_actions.append(self.remove_cache_btn); cache.append(cache_actions)
        self.cleanup_status = Gtk.Label(xalign=0); self.cleanup_status.add_css_class("dim-label"); cache.append(self.cleanup_status)
        self.stack.add_titled(cleanup_page, "cleanup", "5. Cleanup")

        self.activity = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4, visible=False)
        self.activity.add_css_class("sticky-bar")
        activity_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        self.activity_label = Gtk.Label(xalign=0, hexpand=True)
        self.activity_label.set_ellipsize(3)
        self.activity_cancel = Gtk.Button(label="Cancel")
        self.activity_cancel.connect("clicked", lambda *_: self.cancel())
        activity_row.append(self.activity_label); activity_row.append(self.activity_cancel)
        self.progress = Gtk.ProgressBar(show_text=False)
        self.activity.append(activity_row); self.activity.append(self.progress)
        root.append(self.activity)

        footer = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        footer.set_margin_start(18); footer.set_margin_end(18); footer.set_margin_bottom(8)
        self.notice = Gtk.Label(xalign=0, wrap=True)
        self.notice.set_margin_start(18); self.notice.set_margin_end(18)
        self.notice.set_margin_top(6); self.notice.set_margin_bottom(6)
        root.append(self.notice)
        donate = Gtk.LinkButton(
            uri="https://donate.stripe.com/eVq14n8a7agh2lQdqq14400",
            label="Fund our bugs",
        )
        donate.add_css_class("donate-link")
        footer.append(donate)
        footer.append(Gtk.Box(hexpand=True))
        footer.append(Gtk.Label(label=f"v{APP_VERSION}"))
        root.append(footer)
        self.win.set_content(root)
        self.win.connect("close-request", self.close_request)
        self.win.present()
        GLib.idle_add(self.apply_initial_divider, ui_state.get("ratio", 0.5), 0)
        self.inspect_patches()
        self.refresh_kernels()
        self.scan_built_kernels()
        self.scan_cleanup()
        self.update_actions()

    def page(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        for side in ("top", "bottom", "start", "end"):
            getattr(page, f"set_margin_{side}")(18)
        return page

    def section(self, parent, title, subtitle=None, help_key=None):
        panel = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        panel.add_css_class("unified-box")
        heading_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        heading = Gtk.Label(label=title, xalign=0)
        heading.add_css_class("title-4")
        heading.set_hexpand(True); heading_row.append(heading)
        if help_key:
            help_button = Gtk.Button(icon_name="help-about-symbolic", has_frame=False)
            help_button.set_tooltip_text(f"Explain {title}")
            help_button.connect("clicked", lambda *_args, key=help_key: self.show_help(*SECTION_HELP[key]))
            heading_row.append(help_button)
        panel.append(heading_row)
        if subtitle:
            detail = Gtk.Label(label=subtitle, xalign=0, wrap=True)
            detail.add_css_class("dim-label"); panel.append(detail)
        parent.append(panel)
        return panel

    def show_help(self, title, text):
        dialog = Adw.Window(transient_for=self.win, modal=True, title=title)
        dialog.set_default_size(560, 340)
        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        header = Adw.HeaderBar(); header.set_title_widget(Adw.WindowTitle(title=title)); content.append(header)
        label = Gtk.Label(label=text, xalign=0, yalign=0, wrap=True)
        for side in ("top", "bottom", "start", "end"): getattr(label, f"set_margin_{side}")(20)
        content.append(label); dialog.set_content(content); dialog.present()

    def entry_row(self, parent, label, value, changed=None):
        row = Gtk.Box(spacing=10)
        name = Gtk.Label(label=label, xalign=0, max_width_chars=18, ellipsize=3)
        name.set_tooltip_text(label)
        row.append(name)
        entry = Gtk.Entry(text=value, hexpand=True)
        entry.connect("changed", lambda *_: (changed or self.inputs_changed)())
        row.append(entry); parent.append(row)
        return entry

    def setup_kernel_choice(self, _factory, list_item):
        label = Gtk.Label(xalign=0)
        list_item.set_child(label)

    def bind_kernel_choice(self, _factory, list_item):
        item = list_item.get_item()
        list_item.get_child().set_text(item.get_string() if item else "")

    def append_log(self, text):
        buf = self.log.get_buffer(); end = buf.get_end_iter()
        buf.insert(end, text)
        buf.move_mark(self.log_end_mark, buf.get_end_iter())
        self.log.scroll_mark_onscreen(self.log_end_mark)
        return False

    def set_notice(self, text):
        self.notice.set_text(text); return False

    def refresh_kernels(self):
        self.set_notice("Loading available Fedora kernels from Koji…")
        threading.Thread(target=self._fetch_kernels, daemon=True).start()

    def _fetch_kernels(self):
        try:
            html = urllib.request.urlopen("https://kojipkgs.fedoraproject.org/packages/kernel/", timeout=20).read().decode()
            versions = sorted(set(re.findall(r'href="([0-9][^"/]+)/"', html)), key=self.version_key)[-30:]
            dist = f".fc{self.fedora_release()}"
            found = []
            for version in reversed(versions):
                page = urllib.request.urlopen(f"https://kojipkgs.fedoraproject.org/packages/kernel/{version}/", timeout=20).read().decode()
                releases = sorted({r for r in re.findall(r'href="([^"/]+\.fc[0-9]+)/"', page) if r.endswith(dist)}, key=self.version_key)
                if releases: found.append(f"{version}-{releases[-1]}.x86_64")
                if len(found) == 10: break
            GLib.idle_add(self.finish_kernels, found, "")
        except Exception as error:
            GLib.idle_add(self.finish_kernels, [], str(error))

    @staticmethod
    def fedora_release():
        for line in Path("/etc/os-release").read_text().splitlines():
            if line.startswith("VERSION_ID="):
                return line.split("=", 1)[1].strip('"').split(".")[0]
        raise RuntimeError("VERSION_ID missing from /etc/os-release")

    @staticmethod
    def version_key(value):
        return [int(x) if x.isdigit() else x for x in re.split(r"([0-9]+)", value)]

    def finish_kernels(self, kernels, error):
        self.kernel_values = kernels
        self.updating_kernel_model = True
        try:
            self.kernel_model.splice(0, self.kernel_model.get_n_items(), kernels or ["No kernels available"])
            remembered = self.saved_settings.get("kernel", "")
            if remembered in kernels:
                self.kernel_drop.set_selected(kernels.index(remembered))
        finally:
            self.updating_kernel_model = False
        mode = self.saved_settings.get("base_mode")
        if mode in ("fedora", "localmodconfig"):
            self.base_mode = mode
            self.local_state.set_text(f"Last used: {mode}")
        self.save_settings()
        self.set_notice(f"Found {len(kernels)} kernel releases." if kernels else f"Kernel lookup failed: {error}")
        self.update_kernel_base()
        self.update_actions(); return False

    def kernel_selection_changed(self, *_args):
        if self.updating_kernel_model: return
        self.update_kernel_base()
        self.base_inputs_changed()
        self.save_settings()

    def suffix_changed(self, *_args):
        self.update_kernel_base()
        self.base_inputs_changed()
        self.save_settings()

    def update_kernel_base(self):
        if not hasattr(self, "source_detail"): return
        release = self.selected_kernel()
        if release and hasattr(self, "suffix"):
            result = f"{release}{self.suffix.get_text()}"
        else:
            result = ""
        self.source_detail.set_text(f"Resulting kernel: {result}" if result else "Select a kernel release")

    def choose_patch_folder(self, *_args):
        dialog = Gtk.FileDialog(title="Choose patch folder", modal=True)
        dialog.select_folder(self.win, None, self.folder_selected)

    def folder_selected(self, dialog, result):
        try: folder = dialog.select_folder_finish(result)
        except GLib.Error: return
        self.patch_entry.set_text(folder.get_path())

    def patch_folder_changed(self):
        self.prepared = False
        self.inspect_patches()
        self.save_settings()

    def patch_files(self):
        value = self.patch_entry.get_text().strip()
        if not value:
            return []
        folder = Path(value).expanduser()
        return sorted((path for path in folder.glob("*.patch")
                       if path.is_file() and os.access(path, os.R_OK)), key=lambda path: path.name)

    def inspect_patches(self):
        self.clear_box(self.patch_contents)
        for path in self.patch_files():
            self.patch_contents.append(Gtk.Label(label=path.name, xalign=0, ellipsize=3))
        self.update_actions()

    def selected_symbols(self):
        result = list(T2_REQUIRED)
        try:
            pci = subprocess.run(["lspci", "-n"], text=True, capture_output=True).stdout
            if re.search(r"\b1002:", pci): result.append("DRM_AMDGPU")
        except OSError:
            pass
        return sorted(set(result))

    def node_label(self, node):
        prompt = node.prompt[0] if node.prompt else ""
        item = node.item
        name = getattr(item, "name", None)
        return f"{prompt}  [{name}]" if prompt and name else prompt or name or "Unnamed entry"

    def node_store(self, first, query=""):
        store = Gio.ListStore.new(KconfigItem)
        node = first
        while node:
            label = self.node_label(node)
            if node.prompt and (not query or query in label.casefold()):
                store.append(KconfigItem(node))
            elif node.list:
                nested = self.node_store(node.list, query)
                for index in range(nested.get_n_items()): store.append(nested.get_item(index))
            node = node.next
        return store

    def child_store(self, item):
        if not item.node.list: return None
        store = self.node_store(item.node.list)
        return store if store.get_n_items() else None

    def rebuild_kconfig_tree(self):
        if not self.kconfig: return
        query = self.config_search.get_text().strip().casefold()
        if query:
            root = Gio.ListStore.new(KconfigItem)
            for symbol in self.kconfig.unique_defined_syms:
                if not symbol.nodes: continue
                node = symbol.nodes[0]; label = self.node_label(node)
                if query in label.casefold() or query in (symbol.name or "").casefold(): root.append(KconfigItem(node))
            tree = Gtk.TreeListModel.new(root, False, False, lambda item: None)
        else:
            root = self.node_store(self.kconfig.top_node.list)
            tree = Gtk.TreeListModel.new(root, False, False, self.child_store)
        selection = Gtk.SingleSelection(model=tree)
        selection.connect("notify::selected-item", lambda *_: self.update_help_button(selection))
        factory = Gtk.SignalListItemFactory()
        factory.connect("setup", self.setup_kconfig_row)
        factory.connect("bind", self.bind_kconfig_row)
        self.config_selection = selection
        self.kconfig_tree_model = tree
        self.component_tree.set_child(Gtk.ListView(model=selection, factory=factory, single_click_activate=False))

    def setup_kconfig_row(self, _factory, list_item):
        expander = Gtk.TreeExpander()
        content = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        check = Gtk.CheckButton(valign=Gtk.Align.CENTER)
        label = Gtk.Label(xalign=0, ellipsize=3, hexpand=True)
        value = Gtk.Label(xalign=1, width_chars=2)
        value.add_css_class("dim-label")
        content.append(check); content.append(label); content.append(value); expander.set_child(content)
        list_item.set_child(expander)
        list_item._expander, list_item._check, list_item._label, list_item._value = expander, check, label, value
        list_item._handler = None

    def bind_kconfig_row(self, _factory, list_item):
        if list_item._handler:
            list_item._check.disconnect(list_item._handler); list_item._handler = None
        row = list_item.get_item(); wrapped = row.get_item(); node = wrapped.node
        list_item._expander.set_list_row(row)
        list_item._label.set_text(self.node_label(node))
        symbol = node.item if self.kclib and isinstance(node.item, self.kclib.Symbol) else None
        check = list_item._check
        check.set_visible(symbol is not None and symbol.type in (self.kclib.BOOL, self.kclib.TRISTATE))
        list_item._value.set_text(symbol.str_value if symbol is not None else "")
        if not check.get_visible(): return
        check._updating = True
        check.set_active(symbol.tri_value != 0); check.set_inconsistent(symbol.tri_value == 1)
        required = symbol.name in T2_REQUIRED
        check.set_sensitive(bool(symbol.visibility) and not required)
        check.set_tooltip_text("Required by the T2 base profile" if required else
                               "Click to cycle n, m and y" if symbol.type == self.kclib.TRISTATE else None)
        check._updating = False
        list_item._handler = check.connect("toggled", self.toggle_kconfig_symbol, symbol)

    def toggle_kconfig_symbol(self, check, symbol):
        if getattr(check, "_updating", False): return
        current = symbol.tri_value
        target = (current + 1) % 3 if symbol.type == self.kclib.TRISTATE else (0 if current else 2)
        action = "--disable" if target == 0 else "--module" if target == 1 else "--enable"
        result = subprocess.run([str(self.kernel_tree / "scripts/config"), "--file",
                                 str(self.kernel_tree / ".config"), action, symbol.name],
                                text=True, capture_output=True)
        if result.returncode == 0:
            result = subprocess.run(["make", "-s", "-C", str(self.kernel_tree), "olddefconfig"],
                                    text=True, capture_output=True)
        self.kconfig.load_config(str(self.kernel_tree / ".config"))
        if result.returncode:
            self.set_notice(result.stderr.strip() or f"Could not update CONFIG_{symbol.name}.")
        elif symbol.tri_value != target:
            self.set_notice(f"CONFIG_{symbol.name} is constrained to {symbol.str_value} by its dependencies.")
        else:
            self.config_overrides[symbol.name] = symbol.str_value
            self.save_settings()
        check._updating = True
        check.set_active(symbol.tri_value != 0); check.set_inconsistent(symbol.tri_value == 1)
        check._updating = False
        if hasattr(self, "kconfig_tree_model"):
            count = self.kconfig_tree_model.get_n_items()
            self.kconfig_tree_model.items_changed(0, count, count)
        self.prepared = True; self.update_actions()

    def update_help_button(self, selection):
        self.config_help.set_sensitive(selection.get_selected_item() is not None)

    def show_kconfig_help(self):
        row = self.config_selection.get_selected_item() if hasattr(self, "config_selection") else None
        if not row: return
        node = row.get_item().node; symbol = node.item
        lines = [self.node_label(node)]
        if getattr(symbol, "name", None):
            lines += [f"Value: {symbol.str_value}", f"Dependencies: {self.kclib.expr_str(symbol.direct_dep)}"]
        if node.help: lines += ["", node.help.strip()]
        dialog = Adw.Window(transient_for=self.win, modal=True, title="Kconfig help")
        dialog.set_default_size(620, 440)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        box.append(Adw.HeaderBar())
        text = Gtk.TextView(editable=False, cursor_visible=False, wrap_mode=Gtk.WrapMode.WORD_CHAR)
        text.get_buffer().set_text("\n".join(lines))
        scroll = Gtk.ScrolledWindow(vexpand=True); scroll.set_child(text)
        for side in ("top", "bottom", "start", "end"): getattr(scroll, f"set_margin_{side}")(18)
        box.append(scroll); dialog.set_content(box); dialog.present()

    def load_kconfig_tree(self):
        try:
            import kconfiglib
            self.kclib = kconfiglib
            os.environ.update(srctree=str(self.kernel_tree), ARCH="x86_64", SRCARCH="x86",
                              CC=os.environ.get("CC", "gcc"), HOSTCC=os.environ.get("HOSTCC", "gcc"),
                              LD=os.environ.get("LD", "ld"), RUSTC=os.environ.get("RUSTC", "rustc"),
                              PAHOLE=os.environ.get("PAHOLE", "pahole"))
            self.kconfig = kconfiglib.Kconfig(str(self.kernel_tree / "Kconfig"), warn=False)
            self.kconfig.load_config(str(self.kernel_tree / ".config"))
            for name, value in self.pending_config_values.items():
                symbol = self.kconfig.syms.get(name)
                if symbol is None or name in T2_REQUIRED or value not in ("n", "m", "y"): continue
                action = "--disable" if value == "n" else "--module" if value == "m" else "--enable"
                subprocess.run([str(self.kernel_tree / "scripts/config"), "--file",
                                str(self.kernel_tree / ".config"), action, name], check=False)
            if self.pending_config_values:
                subprocess.run(["make", "-s", "-C", str(self.kernel_tree), "olddefconfig"], check=False)
                self.kconfig.load_config(str(self.kernel_tree / ".config"))
                self.config_overrides.update(self.pending_config_values)
                self.pending_config_values = {}
            self.component_tree.set_sensitive(True); self.config_search.set_sensitive(True)
            self.rebuild_kconfig_tree()
            return True
        except Exception as error:
            self.component_tree.set_child(Gtk.Label(label=f"Could not load Kconfig: {error}", xalign=0, wrap=True))
            self.set_notice("Install python3-kconfiglib to display the kernel configuration tree.")
            return False

    def selected_kernel(self):
        index = self.kernel_drop.get_selected()
        return self.kernel_values[index] if index < len(self.kernel_values) else ""

    def source_error(self):
        if not self.selected_kernel(): return "Select an available kernel."
        if not re.fullmatch(r"[A-Za-z0-9._+-]+", self.suffix.get_text()): return "The kernel suffix must be non-empty and contain only letters, numbers, '.', '_', '+', or '-'."
        if not BUILD_SCRIPT.is_file(): return f"Build script not found: {BUILD_SCRIPT}"
        return ""

    def validation_error(self):
        error = self.source_error()
        if error: return error
        if self.base_mode not in ("fedora", "localmodconfig"): return "Prepare a base configuration first."
        if not self.patch_files(): return "Select a folder containing at least one readable *.patch file."
        return ""

    def inputs_changed(self):
        self.prepared = False
        self.update_actions()

    def base_inputs_changed(self):
        self.base_mode = None
        self.prepared = False
        if hasattr(self, "component_tree"):
            self.component_tree.set_sensitive(False)
            self.config_search.set_sensitive(False); self.config_help.set_sensitive(False)
            self.kconfig = None; self.kernel_tree = None
            self.config_overrides = {}
            self.component_tree.set_child(self.component_placeholder)
            self.local_state.set_text("Not completed")
        if hasattr(self, "build_btn"): self.update_actions()

    def update_actions(self):
        busy = self.running
        valid = not self.validation_error()
        self.fedora_btn.set_sensitive(not self.source_error() and not busy)
        self.local_btn.set_sensitive(not self.source_error() and not busy)
        self.build_btn.set_sensitive(valid and not busy)
        self.install_btn.set_sensitive(bool(self.built_tree and self.built_release) and not busy)
        self.cancel_btn.set_sensitive(busy)

    def confirm_localmod(self, *_args):
        dialog = Adw.AlertDialog(
            heading="Connect all required hardware first",
            body="localmodconfig uses currently loaded modules. Connect USB and Thunderbolt devices, docks, displays, storage and network adapters that the minimal kernel must support.")
        dialog.add_response("cancel", "Cancel")
        dialog.add_response("continue", "Run localmodconfig")
        dialog.set_response_appearance("continue", Adw.ResponseAppearance.SUGGESTED)
        dialog.choose(self.win, None, self.localmod_response)

    def localmod_response(self, dialog, result):
        try: response = dialog.choose_finish(result)
        except GLib.Error: return
        if response == "continue": self.prepare("localmodconfig", base_action=True)

    def command(self, prepare_only=False, clean=False):
        patch_files = self.patch_files()
        patch_folder = Path(self.patch_entry.get_text()).expanduser() if patch_files else EMPTY_PATCH_DIR
        command = [str(BUILD_SCRIPT), self.selected_kernel(), "--patch-dir", str(patch_folder),
                   "--localversion", self.suffix.get_text(), "--jobs", str(self.jobs_spin.get_value_as_int()), "--t2-config"]
        if not patch_files: command.append("--allow-no-patches")
        if self.base_mode == "localmodconfig": command.append("--localmodconfig")
        for symbol in self.selected_symbols(): command += ["--enable-config", symbol]
        if clean: command.append("--clean")
        if prepare_only: command.append("--prepare-only")
        if not prepare_only: command += ["--local-install", "--defer-install"]
        return command

    def prepare(self, mode, base_action=False):
        selecting_base = base_action and mode in ("fedora", "localmodconfig")
        error = self.source_error() if selecting_base else self.validation_error()
        if error: self.set_notice(error); return
        if selecting_base and not self.patch_files():
            EMPTY_PATCH_DIR.mkdir(parents=True, exist_ok=True)
        self.base_mode = mode
        self.save_settings()
        self.local_state.set_text("Running localmodconfig…" if mode == "localmodconfig" else
                                  "Preparing Fedora default configuration…")
        self.run_command(self.command(True),
                         "Preparing sources, checking patches and generating configuration…", "prepare")

    def start_build(self):
        error = self.validation_error()
        if error: self.set_notice(error); return
        if not self.prepared:
            self.build_after_prepare = True
            self.prepare(self.base_mode)
            return
        self.run_command(self.command(False),
                         f"Building with {self.jobs_spin.get_value_as_int()} parallel jobs…", "build")

    def start_install(self):
        if not self.built_tree or not self.built_release: return
        cancel_dir = WORK_DIR.parent / "cancel"
        cancel_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
        cancel_dir.chmod(0o700)
        self.cancel_file = cancel_dir / str(uuid.uuid4())
        self.cancel_file.touch(mode=0o600)
        self.run_command(["pkexec", str(CLEANUP_HELPER), "install-kernel",
                          str(self.built_tree), self.built_release, str(self.cancel_file)],
                         "Waiting for authorization, then installing the built kernel…", "install")

    def scan_built_kernels(self, preferred_release=None):
        builds = []
        if WORK_DIR.is_dir():
            for entry in WORK_DIR.iterdir():
                tree_marker = entry / "built-kernel-tree"
                release_marker = entry / "built-kernel-release"
                try:
                    tree = Path(tree_marker.read_text(encoding="utf-8").strip()).resolve()
                    release = release_marker.read_text(encoding="utf-8").strip()
                    if (tree.is_relative_to(entry.resolve()) and (tree / "Makefile").is_file() and
                            re.fullmatch(r"[A-Za-z0-9._+-]+", release)):
                        builds.append((release, tree))
                except (OSError, ValueError):
                    continue
        builds.sort(key=lambda item: self.version_key(item[0]), reverse=True)
        self.completed_builds = builds
        labels = []
        for release, _tree in builds:
            present = [Path(f"/lib/modules/{release}").is_dir(),
                       Path(f"/boot/vmlinuz-{release}").is_file(),
                       Path(f"/boot/initramfs-{release}.img").is_file()]
            state = "installed" if all(present) else "partial installation" if any(present) else "ready"
            labels.append(f"{release} — {state}")
        if not labels: labels = ["No completed builds found"]
        self.built_model.splice(0, self.built_model.get_n_items(), labels)
        selected = next((i for i, item in enumerate(builds) if item[0] == preferred_release), 0)
        self.built_drop.set_selected(selected)
        self.built_selection_changed()

    def built_selection_changed(self, *_args):
        index = self.built_drop.get_selected()
        if index < len(getattr(self, "completed_builds", [])):
            self.built_release, self.built_tree = self.completed_builds[index]
        else:
            self.built_release = None; self.built_tree = None
        self.update_actions()

    def run_command(self, command, message, kind):
        if self.running: return
        self.running = True
        self.active_command = list(command)
        self.command_output = []
        self.operation_kind = kind
        self.operation_started = time.monotonic()
        self.last_output_at = self.operation_started
        self.operation_phase = message
        self.log.get_buffer().set_text("")
        self.progress.set_fraction(0.0)
        self.activity.set_visible(True)
        self.activity_cancel.set_sensitive(True)
        self.activity_label.set_text(message)
        if self.progress_timer: GLib.source_remove(self.progress_timer)
        self.progress_timer = GLib.timeout_add(250, self.update_progress_clock)
        self.set_notice(message); self.update_actions()
        threading.Thread(target=self._run, args=(command, kind), daemon=True).start()

    def apply_initial_divider(self, ratio, attempt):
        width = self.workspace.get_width()
        if width <= 1 and attempt < 10:
            GLib.timeout_add(50, self.apply_initial_divider, ratio, attempt + 1)
            return False
        self.workspace.set_position(round(width * min(0.5, max(0.5, ratio))))
        return False

    def _run(self, command, kind):
        try:
            environment = os.environ.copy()
            environment["KERNEL_BUILD_ROOT"] = str(WORK_DIR)
            self.process = subprocess.Popen(command, cwd=Path.home(), env=environment, stdout=subprocess.PIPE,
                                            stderr=subprocess.STDOUT, text=True,
                                            bufsize=1, start_new_session=True)
            GLib.idle_add(self.update_actions)
            log_batch = []
            last_log_flush = 0.0
            for line in self.process.stdout:
                self.command_output.append(line)
                self.last_output_at = time.monotonic()
                log_batch.append(line)
                if len(log_batch) >= 100 or self.last_output_at - last_log_flush >= 0.1:
                    GLib.idle_add(self.append_log, "".join(log_batch))
                    log_batch = []
                    last_log_flush = self.last_output_at
                if ("[kait2en-progress]" in line or line.startswith(("Downloading ", "Extracting ",
                                                                     "Preparing ", "Applying ", "Building "))):
                    GLib.idle_add(self.process_progress_line, line)
            if log_batch:
                GLib.idle_add(self.append_log, "".join(log_batch))
            code = self.process.wait()
        except OSError as error:
            code = 127; GLib.idle_add(self.append_log, f"{error}\n")
        finally:
            self.process = None
            self.running = False
            if self.cancel_file is not None:
                self.cancel_file.unlink(missing_ok=True)
                self.cancel_file = None
        GLib.idle_add(self.finished, kind, code)

    def process_progress_line(self, line):
        self.last_output_at = time.monotonic()
        phases = {
            "Downloading ": ("Downloading kernel source", 0.08),
            "Extracting Fedora source package": ("Extracting Fedora source", 0.28),
            "Preparing Fedora kernel sources": ("Preparing kernel source tree", 0.42),
            "Applying ": ("Applying and validating patches", 0.58),
            "[kait2en-progress] phase=localmodconfig": ("Running localmodconfig", 0.78),
            "Building ": ("Building kernel", 0.0),
            "[kait2en-progress] phase=installing": ("Waiting for authorization, then installing", 0.98),
            "[kait2en-progress] phase=install-modules": ("Installing kernel modules", 0.98),
            "[kait2en-progress] phase=install-dkms": ("Installing DKMS modules", 0.98),
            "[kait2en-progress] phase=install-boot": ("Creating kernel boot files and initramfs", 0.98),
            "[kait2en-progress] phase=verify-install": ("Verifying installed kernel", 0.98),
        }
        for marker, (phase, fraction) in phases.items():
            if marker in line:
                self.operation_phase = phase
                if self.operation_kind == "prepare": self.progress.set_fraction(fraction)
                break
        return False

    def update_progress_clock(self):
        if not self.running:
            self.progress_timer = 0
            return False
        elapsed = int(time.monotonic() - self.operation_started)
        minutes, seconds = divmod(elapsed, 60)
        detail = f"{self.operation_phase} · {minutes}:{seconds:02d} elapsed"
        self.progress.pulse()
        quiet = int(time.monotonic() - self.last_output_at)
        if quiet >= 30: detail += f" · no new output for {quiet}s"
        self.activity_label.set_text(detail)
        if self.operation_kind == "prepare": self.local_state.set_text(detail)
        return True

    @staticmethod
    def format_elapsed(seconds):
        hours, remainder = divmod(max(0, int(seconds)), 3600)
        minutes, seconds = divmod(remainder, 60)
        return f"{hours}:{minutes:02d}:{seconds:02d}" if hours else f"{minutes}:{seconds:02d}"

    def finished(self, kind, code):
        if self.progress_timer:
            GLib.source_remove(self.progress_timer); self.progress_timer = 0
        if (code != 0 and kind == "prepare" and "--clean" not in self.active_command and
                "Build inputs changed. Re-run with --clean." in "".join(self.command_output)):
            self.run_command(self.active_command + ["--clean"],
                             "Build inputs changed; reusing downloads and recreating the derived tree…", "prepare")
            return False
        self.progress.set_fraction(1.0 if code == 0 else 0.0)
        if code == 0 and kind == "prepare":
            self.prepared = True
            base_label = "localmodconfig" if self.base_mode == "localmodconfig" else "Fedora default"
            self.local_state.set_text(f"{base_label} ready - optional components unlocked")
            match = re.search(r"^Prepared kernel tree: (.+)$", "".join(self.command_output), re.MULTILINE)
            self.kernel_tree = Path(match.group(1)) if match else None
            if self.kernel_tree: self.load_kconfig_tree()
            self.progress.set_text("Sources, patches and configuration validated")
            self.set_notice("Preparation succeeded. Starting the kernel build…" if self.build_after_prepare else
                            "Preparation succeeded. The kernel is ready to build.")
            if self.build_after_prepare:
                self.build_after_prepare = False
                GLib.idle_add(self.start_build)
        elif code == 0:
            if kind == "build":
                output = "".join(self.command_output)
                tree = re.search(r"^Built kernel tree: (.+)$", output, re.MULTILINE)
                release = re.search(r"^Built kernel release: (.+)$", output, re.MULTILINE)
                self.built_tree = Path(tree.group(1)) if tree else None
                self.built_release = release.group(1) if release else None
                self.scan_built_kernels(self.built_release)
                elapsed = self.format_elapsed(time.monotonic() - self.operation_started)
                self.progress.set_text(f"Build finished in {elapsed}")
                self.set_notice(f"Build finished in {elapsed}. Install the kernel when you are ready.")
            else:
                self.progress.set_text("Kernel installed")
                self.set_notice(f"Kernel {self.built_release} installed successfully.")
                self.scan_built_kernels()
        else:
            if kind == "prepare": self.build_after_prepare = False
            self.prepared = False if kind == "prepare" else self.prepared
            self.progress.set_text("Operation failed")
            self.set_notice("Operation failed. See the build log for the exact patch, configuration or build error.")
        self.activity_label.set_text(self.progress.get_text() or ("Completed" if code == 0 else "Operation failed"))
        self.activity_cancel.set_sensitive(False)
        self.update_actions(); return False

    def cancel(self):
        if self.process:
            if self.operation_kind == "install" and self.cancel_file is not None:
                self.cancel_file.write_text("cancel\n", encoding="utf-8")
                self.activity_label.set_text("Cancelling installation and rolling back…")
                self.activity_cancel.set_sensitive(False)
                try: os.killpg(self.process.pid, signal.SIGTERM)
                except (ProcessLookupError, PermissionError): pass
            else:
                try: os.killpg(self.process.pid, signal.SIGTERM)
                except ProcessLookupError: pass

    def settings_data(self):
        config_values = dict(self.pending_config_values)
        config_values.update(self.config_overrides)
        return {"version": 2, "kernel": self.selected_kernel(), "suffix": self.suffix.get_text(),
                "patch_folder": self.patch_entry.get_text(), "base_mode": self.base_mode,
                "config_values": config_values, "jobs": self.jobs_spin.get_value_as_int()}

    def save_settings(self):
        if not hasattr(self, "suffix") or not hasattr(self, "jobs_spin"):
            return
        try:
            CONFIG_DIR.mkdir(parents=True, exist_ok=True)
            temporary = SETTINGS_FILE.with_suffix(".tmp")
            temporary.write_text(json.dumps(self.settings_data(), indent=2) + "\n", encoding="utf-8")
            temporary.replace(SETTINGS_FILE)
        except OSError:
            pass

    def clear_box(self, box):
        while (child := box.get_first_child()): box.remove(child)

    def scan_cleanup(self):
        self.clear_box(self.cleanup_kernel_box); self.clear_box(self.cleanup_cache_box)
        self.cleanup_kernel_checks = {}; self.cleanup_cache_checks = {}
        running = os.uname().release
        module_root = Path("/lib/modules")
        releases = sorted((path.name for path in module_root.iterdir() if path.is_dir()), key=self.version_key)
        for release in releases:
            protected = release == running or "rescue" in release.lower()
            label = release + (" - running, protected" if release == running else
                               " - rescue, protected" if protected else "")
            check = Gtk.CheckButton(label=label, sensitive=not protected)
            self.cleanup_kernel_box.append(check)
            if not protected: self.cleanup_kernel_checks[release] = check
        rescue_images = sorted(Path("/boot").glob("vmlinuz-0-rescue-*"))
        for image in rescue_images:
            self.cleanup_kernel_box.append(Gtk.CheckButton(label=f"{image.name} - protected", sensitive=False))
        if not releases and not rescue_images:
            self.cleanup_kernel_box.append(Gtk.Label(label="No installed kernels found.", xalign=0))

        cache_entries = []
        if WORK_DIR.is_dir():
            cache_entries = sorted((item for item in WORK_DIR.iterdir() if item.is_dir()), key=lambda p: self.version_key(p.name))
            for path in cache_entries:
                state = "prepared" if (path / ".prepared").is_file() else "cached source"
                in_use = False
                marker = path / "kernel-tree"
                if marker.is_file():
                    try:
                        tree = Path(marker.read_text(encoding="utf-8").strip()).resolve()
                        for link in list(Path("/lib/modules").glob("*/build")) + list(Path("/lib/modules").glob("*/source")):
                            release = link.parent.name
                            if ((release == running or Path(f"/boot/vmlinuz-{release}").is_file()) and
                                    link.is_symlink() and link.resolve() == tree):
                                in_use = True
                                break
                    except OSError:
                        pass
                label = f"{path.name} - {state}" + (" - installed kernel uses this tree" if in_use else "")
                check = Gtk.CheckButton(label=label, sensitive=not in_use)
                self.cleanup_cache_box.append(check)
                if not in_use: self.cleanup_cache_checks[path] = check
        if not cache_entries:
            self.cleanup_cache_box.append(Gtk.Label(label="No downloaded sources or build data found.", xalign=0))
        self.cleanup_status.set_text(f"Running kernel: {running}")

    def cleanup_selection(self, kind):
        checks = self.cleanup_kernel_checks if kind == "kernels" else self.cleanup_cache_checks
        return [item for item, check in checks.items() if check.get_active()]

    def confirm_cleanup(self, kind):
        if self.running or self.cleanup_running:
            self.cleanup_status.set_text("Another operation is already running."); return
        selected = self.cleanup_selection(kind)
        if not selected:
            self.cleanup_status.set_text("Select at least one item first."); return
        labels = {"kernels": "Remove selected kernels?",
                  "cache": "Delete selected cache and sources?"}
        descriptions = {
            "kernels": "The running kernel and rescue images remain protected. Selected kernels cannot be restored without reinstalling or rebuilding them.",
            "cache": "All cached files, downloaded and extracted sources, configurations and compiled files for the selected kernel versions are permanently removed.",
        }
        dialog = Adw.AlertDialog(heading=labels[kind], body=descriptions[kind])
        dialog.add_response("cancel", "Cancel"); dialog.add_response("continue", "Continue")
        dialog.set_response_appearance("continue", Adw.ResponseAppearance.DESTRUCTIVE)
        dialog.choose(self.win, None, lambda d, result: self.cleanup_response(d, result, kind, selected))

    def cleanup_response(self, dialog, result, kind, selected):
        try: response = dialog.choose_finish(result)
        except GLib.Error: return
        if response != "continue": return
        self.cleanup_running = True
        self.remove_kernels_btn.set_sensitive(False); self.remove_cache_btn.set_sensitive(False)
        self.cleanup_status.set_text("Cleanup in progress… Follow progress in the bar and log.")
        self.activity.set_visible(True); self.activity_cancel.set_sensitive(False)
        self.progress.set_fraction(0.0)
        action = "Removing kernels" if kind == "kernels" else "Deleting cache and sources"
        self.activity_label.set_text(f"{action} - 0 of {len(selected)}")
        self.append_log(f"\n{action}: {', '.join(str(item) for item in selected)}\n")
        threading.Thread(target=self._cleanup, args=(kind, selected), daemon=True).start()

    def _cleanup(self, kind, selected):
        errors = []
        total = len(selected)
        for index, item in enumerate(selected, start=1):
            GLib.idle_add(self.cleanup_item_started, kind, str(item), index, total)
            try:
                if kind == "kernels":
                    process = subprocess.Popen(["pkexec", str(CLEANUP_HELPER), "remove-kernel", item],
                                               text=True, stdout=subprocess.PIPE,
                                               stderr=subprocess.STDOUT, bufsize=1)
                    output = []
                    for line in process.stdout:
                        output.append(line); GLib.idle_add(self.append_log, line)
                    if process.wait():
                        raise RuntimeError("".join(output).strip() or f"failed to remove {item}")
                else:
                    target = item.resolve()
                    if target.parent != WORK_DIR.resolve(): raise RuntimeError(f"invalid cache path: {target}")
                    shutil.rmtree(target)
                    GLib.idle_add(self.append_log, f"Deleted {target}\n")
            except (OSError, RuntimeError) as error:
                errors.append(str(error))
                GLib.idle_add(self.append_log, f"ERROR: {error}\n")
            GLib.idle_add(self.progress.set_fraction, index / total)
        GLib.idle_add(self.finish_cleanup, errors, kind, total)

    def cleanup_item_started(self, kind, item, index, total):
        action = "Removing kernel" if kind == "kernels" else "Deleting data for"
        self.activity_label.set_text(f"{action} {item} - {index} of {total}")
        self.append_log(f"{action} {item}\n")
        return False

    def finish_cleanup(self, errors, kind, total):
        if kind == "cache":
            self.base_inputs_changed()
            self.save_settings()
        self.scan_cleanup()
        self.scan_built_kernels()
        self.cleanup_running = False
        self.remove_kernels_btn.set_sensitive(True); self.remove_cache_btn.set_sensitive(True)
        if errors:
            result = f"Cleanup failed for {len(errors)} of {total} selected items. See the log."
            self.progress.set_fraction(0.0)
        else:
            noun = "kernel" if kind == "kernels" and total == 1 else "kernels" if kind == "kernels" else "entries"
            result = f"Successfully removed {total} {noun}."
            self.progress.set_fraction(1.0)
        self.cleanup_status.set_text(result); self.activity_label.set_text(result)
        self.append_log(result + "\n")
        return False

    def load_settings(self):
        try:
            data = json.loads(SETTINGS_FILE.read_text(encoding="utf-8"))
            return data if isinstance(data, dict) and data.get("version") == 2 else {}
        except (OSError, ValueError, TypeError):
            return {}

    def load_ui_state(self):
        try:
            data = json.loads(UI_STATE.read_text(encoding="utf-8"))
            if not isinstance(data, dict) or data.get("version") not in (2, 3): return {}
            width, height = int(data.get("width", 1340)), int(data.get("height", 760))
            if data.get("version") == 2:
                width += 100
            ratio = float(data.get("ratio", 0.5))
            return {"width": max(800, width), "height": max(600, height),
                    "ratio": min(0.8, max(0.2, ratio))}
        except (OSError, ValueError, TypeError):
            return {}

    def save_ui_state(self):
        try:
            CONFIG_DIR.mkdir(parents=True, exist_ok=True)
            pane_width = max(1, self.workspace.get_width())
            data = {"version": 3, "width": self.win.get_width(), "height": self.win.get_height(),
                    "ratio": self.workspace.get_position() / pane_width}
            temporary = UI_STATE.with_suffix(".tmp")
            temporary.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
            temporary.replace(UI_STATE)
        except OSError:
            pass

    def close_request(self, *_args):
        self.save_settings()
        self.save_ui_state()
        self.cancel()
        return False

if __name__ == "__main__":
    KernelBuilder().run()

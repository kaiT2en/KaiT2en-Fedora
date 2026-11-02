// SPDX-License-Identifier: GPL-3.0-or-later

import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Background from 'resource:///org/gnome/shell/ui/background.js';
import * as IconGrid from 'resource:///org/gnome/shell/ui/iconGrid.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as ModalDialog from 'resource:///org/gnome/shell/ui/modalDialog.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {Extension, InjectionManager} from 'resource:///org/gnome/shell/extensions/extension.js';

const PANEL_BLUR_RADIUS = 60;
const PANEL_BLUR_BRIGHTNESS = 0.9;
const OVERVIEW_BLUR_RADIUS = 88;
const OVERVIEW_BLUR_BRIGHTNESS = 0.64;
const GLASS_BLUR_RADIUS = 40;
const GLASS_BLUR_BRIGHTNESS = 0.8;
const FADE_TIME = 250;

// Icon tile: inset of the tile, corner radius and rim width relative to
// the tile, and the share of the tile the icon fills.
const ICON_TILE_MARGIN = 0.015;
const ICON_TILE_RADIUS = 0.225;
const ICON_TILE_RIM = 0.009;
const ICON_TILE_FILL = 0.86;

// How far the dash icon under the pointer lifts, how much it grows and
// how long it takes.
const DOCK_LIFT = 10;
const DOCK_GROW = 1.15;
const DOCK_LIFT_TIME = 150;

const APP_ICON_SIZE = 64;
const APP_GRID_MODES = [
    {rows: 8, columns: 4},
    {rows: 7, columns: 5},
    {rows: 5, columns: 7},
    {rows: 4, columns: 8},
];

// A monitor sized copy of the wallpaper. The blur renders once and stays
// cached until the wallpaper or the scale changes.
class BlurredWallpaper {
    constructor(parent, monitorIndex, x, y, radius, brightness) {
        const monitor = Main.layoutManager.monitors[monitorIndex];
        const themeContext = St.ThemeContext.get_for_stage(global.stage);

        this._radius = radius;
        this._effect = new Shell.BlurEffect({
            mode: Shell.BlurMode.ACTOR,
            brightness,
        });
        this._widget = new St.Widget({
            x, y,
            width: monitor.width,
            height: monitor.height,
            effect: this._effect,
        });
        parent.add_child(this._widget);

        this._bgManager = new Background.BackgroundManager({
            container: this._widget,
            monitorIndex,
            controlPosition: false,
        });

        themeContext.connectObject('notify::scale-factor', () => this._scale(), this);
        this._scale();
    }

    _scale() {
        const {scaleFactor} = St.ThemeContext.get_for_stage(global.stage);
        this._effect.radius = this._radius * scaleFactor;
    }

    destroy() {
        St.ThemeContext.get_for_stage(global.stage).disconnectObject(this);
        this._bgManager.destroy();
        this._widget.destroy();
    }
}

// Windows never cover the top bar, so a blurred copy of the wallpaper
// behind it looks the same as live blur.
class PanelBlur {
    constructor() {
        const {layoutManager} = Main;

        this._strip = new St.Widget({clip_to_allocation: true});
        this._strip.add_constraint(new Clutter.BindConstraint({
            source: layoutManager.panelBox,
            coordinate: Clutter.BindCoordinate.ALL,
        }));
        layoutManager.uiGroup.insert_child_below(this._strip, layoutManager.panelBox);

        this._build();

        layoutManager.connectObject('monitors-changed', () => this._build(), this);
        layoutManager.panelBox.connectObject('notify::visible', () => this._sync(), this);
        Main.overview.connectObject(
            'showing', () => this._fade(0),
            'hiding', () => this._fade(255),
            this);
        this._sync();
    }

    _build() {
        this._wall?.destroy();
        this._wall = null;

        if (Main.layoutManager.primaryMonitor) {
            this._wall = new BlurredWallpaper(this._strip,
                Main.layoutManager.primaryIndex, 0, 0,
                PANEL_BLUR_RADIUS, PANEL_BLUR_BRIGHTNESS);
        }
    }

    _sync() {
        this._strip.visible = Main.layoutManager.panelBox.visible;
        this._strip.opacity = Main.overview.visible ? 0 : 255;
    }

    _fade(opacity) {
        this._strip.ease({
            opacity,
            duration: FADE_TIME,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
        });
    }

    destroy() {
        Main.layoutManager.disconnectObject(this);
        Main.layoutManager.panelBox.disconnectObject(this);
        Main.overview.disconnectObject(this);
        this._wall?.destroy();
        this._strip.destroy();
    }
}

// The overview shows the blurred wallpaper instead of a flat gray.
class OverviewBackground {
    constructor() {
        this._group = new Clutter.Actor();
        Main.layoutManager.overviewGroup.insert_child_at_index(this._group, 0);

        this._walls = [];
        this._build();
        Main.layoutManager.connectObject('monitors-changed', () => this._build(), this);
    }

    _build() {
        this._walls.forEach(wall => wall.destroy());
        this._walls = Main.layoutManager.monitors.map((monitor, index) =>
            new BlurredWallpaper(this._group, index, monitor.x, monitor.y,
                OVERVIEW_BLUR_RADIUS, OVERVIEW_BLUR_BRIGHTNESS));
    }

    destroy() {
        Main.layoutManager.disconnectObject(this);
        this._walls.forEach(wall => wall.destroy());
        this._group.destroy();
    }
}

// GNOME 51 replaced Shell.GLSLEffect with snippets on Clutter.ShaderEffect.
// The fragment code stays the same, only the way it is attached and how
// uniforms are set differs.
function shaderEffect(name, declarations, code, replace) {
    if (Shell.GLSLEffect) {
        return GObject.registerClass({GTypeName: `Kait2enVornehm${name}`},
        class extends Shell.GLSLEffect {
            vfunc_build_pipeline() {
                this.add_glsl_snippet(Cogl.SnippetHook.FRAGMENT,
                    declarations, code, replace);
            }

            setUniform(uniform, values) {
                this.set_uniform_float(this.get_uniform_location(uniform),
                    values.length, values);
            }
        });
    }

    return GObject.registerClass({GTypeName: `Kait2enVornehm${name}`},
    class extends Clutter.ShaderEffect {
        vfunc_get_static_snippet() {
            const snippet = Cogl.Snippet.new(Cogl.SnippetHook.FRAGMENT,
                declarations, replace ? null : code);
            if (replace)
                snippet.set_replace(code);
            return snippet;
        }

        setUniform(uniform, values) {
            this.set_uniform_float(uniform, values.length, values);
        }
    });
}

// Cuts the blurred copy to the rounded shape of the menu.
const MASK_DECLARATIONS = `
uniform vec2 size;
uniform float radius;
`;

const MASK_CODE = `
vec2 p = cogl_tex_coord_in[0].xy * size;
vec2 q = abs(p - 0.5 * size) - (0.5 * size - radius);
float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
cogl_color_out *= clamp(0.5 - d, 0.0, 1.0);
`;

const RoundedMask = GObject.registerClass(
class RoundedMask extends shaderEffect('RoundedMaskBase',
    MASK_DECLARATIONS, MASK_CODE, false) {
    setShape(width, height, radius) {
        this.setUniform('size', [width, height]);
        this.setUniform('radius', [radius]);
        this.queue_repaint();
    }
});

// Live glass behind a floating surface of the shell, a menu, a dialog or
// a notification banner. What lies behind it is copied, blurred and cut to
// its rounded shape, the surface paints on top. The window group has no
// size of its own, so its parts are cloned one by one. A clone paints its
// source also while the source is hidden, mapping it for that paint. The
// shell then runs map handlers in the middle of the paint, the search
// entry of a hidden overview adds and removes stage actions there. So the
// glass only clones what is visible and builds anew once a source shows
// or hides. Every frame the glass copies the place, zoom and opacity of
// the surface on screen, and it goes away with the surface. A lasting
// surface, the banner, only hides while a menu of the top bar is open and
// gets fresh glass after.
class GlassSurface {
    constructor(surface, parent, below, lasting) {
        this._surface = surface;
        this._lasting = lasting;
        this._wasMapped = false;
        this._stale = false;
        this._sources = [];

        this._content = new St.Widget();
        this._blur = new Shell.BlurEffect({
            mode: Shell.BlurMode.ACTOR,
            brightness: GLASS_BLUR_BRIGHTNESS,
        });
        this._mask = new RoundedMask();
        this._glass = new St.Widget({clip_to_allocation: true, visible: false});
        this._glass.add_child(this._content);
        this._glass.add_effect(this._mask);
        this._glass.add_effect(this._blur);
        parent.insert_child_below(this._glass, below);
        this._glass.connect('destroy', () => this._release());
        this._fill();

        surface.connectObject('destroy', () => this.destroy(), this);
        this._timeline = new Clutter.Timeline({
            actor: surface,
            duration: 1000,
            repeat_count: -1,
        });
        this._timeline.connect('new-frame', () => this._sync());
        this._timeline.start();
        this._sync();
    }

    _fill() {
        this._unwatch();
        this._bgManager?.destroy();
        this._bgManager = null;
        this._content.destroy_all_children();
        this._stale = false;

        // The shell hides the windows while the overview shows and the
        // overview while the windows show.
        const {overviewGroup} = Main.layoutManager;
        this._watch(overviewGroup);
        this._watch(global.window_group);
        if (overviewGroup.visible) {
            this._content.add_child(new Clutter.Clone({source: overviewGroup}));
            return;
        }

        const monitor = Main.layoutManager.primaryMonitor;
        const wallpaper = new St.Widget({
            x: monitor.x,
            y: monitor.y,
            width: monitor.width,
            height: monitor.height,
        });
        this._content.add_child(wallpaper);
        this._bgManager = new Background.BackgroundManager({
            container: wallpaper,
            monitorIndex: Main.layoutManager.primaryIndex,
            controlPosition: false,
        });

        if (!global.window_group.visible)
            return;
        for (const windowActor of global.get_window_actors()) {
            if (!windowActor.visible)
                continue;
            this._watch(windowActor);
            this._content.add_child(new Clutter.Clone({
                source: windowActor,
                x: windowActor.x,
                y: windowActor.y,
            }));
        }
    }

    _watch(source) {
        source.connectObject('notify::visible', () => this._fill(), this);
        this._sources.push(source);
    }

    _unwatch() {
        this._sources.forEach(source => source.disconnectObject(this));
        this._sources = [];
    }

    _sync() {
        const surface = this._surface;
        if (!surface.mapped) {
            // A menu or dialog that was on screen and left it is done.
            if (this._wasMapped && !this._lasting) {
                this.destroy();
                return;
            }
            this._glass.hide();
            this._stale = this._wasMapped;
            return;
        }
        this._wasMapped = true;

        // While a child of the surface changes size, for example the
        // volume icon next to its slider, the surface waits for a new
        // layout. It keeps its place, so the glass stays where it is.
        if (!surface.has_allocation())
            return;

        if (this._stale)
            this._fill();

        const [stageX, stageY] = surface.get_transformed_position();
        const [width, height] = surface.get_transformed_size();
        const [ok, x, y] = this._glass.get_parent().transform_stage_point(stageX, stageY);
        if (!ok || !(width >= 1 && height >= 1)) {
            this._glass.hide();
            return;
        }

        const {scaleFactor} = St.ThemeContext.get_for_stage(global.stage);
        const radius = surface.get_theme_node().get_border_radius(St.Corner.TOPLEFT) /
            scaleFactor * width / surface.width;
        this._glass.set_position(x, y);
        this._glass.set_size(width, height);
        this._glass.opacity = surface.get_paint_opacity();
        this._content.set_position(-stageX, -stageY);
        this._blur.radius = GLASS_BLUR_RADIUS * scaleFactor;
        this._mask.setShape(width, height, radius);
        this._glass.show();
    }

    // Runs while the glass still has its children, also when the shell
    // destroys the parent of a closing dialog before the dialog itself.
    _release() {
        if (!this._timeline)
            return;
        this._timeline.stop();
        this._timeline = null;
        this._surface.disconnectObject(this);
        this._unwatch();
        this._bgManager?.destroy();
        this._bgManager = null;
        GlassSurface.active.delete(this);
    }

    destroy() {
        if (this._timeline)
            this._glass.destroy();
    }

    // Glass is optional, a surface without a place for it opens plain.
    static follow(surface, parent, below, lasting = false) {
        if (surface && parent && below?.get_parent() === parent)
            GlassSurface.active.add(new GlassSurface(surface, parent, below, lasting));
    }
}
GlassSurface.active = new Set();

// Menus, dialogs and banners are hooked where they open, so every one of
// them gets glass, also those created after the extension.
class Glass {
    constructor() {
        this._injections = new InjectionManager();

        this._injections.overrideMethod(PopupMenu.PopupMenu.prototype, 'open',
            original => function (...args) {
                const wasOpen = this.isOpen;
                original.call(this, ...args);
                if (this.isOpen && !wasOpen)
                    GlassSurface.follow(this.box, this.actor.get_parent(), this.actor);
            });

        this._injections.overrideMethod(ModalDialog.ModalDialog.prototype, 'open',
            original => function (...args) {
                const wasClosed = this.state === ModalDialog.State.CLOSED;
                const opened = original.call(this, ...args);
                if (opened && wasClosed)
                    GlassSurface.follow(this.dialogLayout._dialog, this, this._backgroundBin);
                return opened;
            });

        const tray = Main.messageTray;
        this._injections.overrideMethod(Object.getPrototypeOf(tray), '_showNotification',
            original => function (...args) {
                original.call(this, ...args);
                if (this._banner)
                    GlassSurface.follow(this._banner, this, this._bannerBin, true);
            });

        // The shell enables extensions while it starts up, a banner of
        // the login can already be on screen and gets its glass now.
        if (tray._banner)
            GlassSurface.follow(tray._banner, tray, tray._bannerBin, true);
    }

    destroy() {
        this._injections.clear();
        GlassSurface.active.forEach(glass => glass.destroy());
    }
}

// App icons sit on a tile like on macOS Tahoe. In the dark style the
// tile fades from gray 49 to 19 and a fine light rim runs along its edge,
// fading out towards the bottom. In the light style the tile fades from
// white to gray 232 and the rim is a dark hairline. The icon fills most
// of the tile.
const TILE_STYLES = {
    dark: {top: 0.192, bottom: 0.075, rim: 1, rimTop: 0.34, rimBottom: 0.06},
    light: {top: 1, bottom: 0.91, rim: 0, rimTop: 0.08, rimBottom: 0.14},
};

const TILE_DECLARATIONS = `
uniform sampler2D tex;
uniform float size;
uniform float top;
uniform float bottom;
uniform float rimGray;
uniform float rimTop;
uniform float rimBottom;
`;

const TILE_CODE = `
vec2 uv = cogl_tex_coord_in[0].xy;
vec2 p = uv * size;
float margin = size * ${ICON_TILE_MARGIN};
float radius = (size - 2.0 * margin) * ${ICON_TILE_RADIUS};
vec2 q = abs(p - 0.5 * size) - (0.5 * size - margin - radius);
float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
float inside = clamp(0.5 - d, 0.0, 1.0);
float rim = inside * clamp(d + max(1.0, size * ${ICON_TILE_RIM}) + 0.5, 0.0, 1.0);

vec4 color = vec4(vec3(mix(top, bottom, uv.y)) * inside, inside);
vec2 iconUv = (uv - 0.5) / ${ICON_TILE_FILL} + 0.5;
if (all(greaterThanEqual(iconUv, vec2(0.0))) && all(lessThanEqual(iconUv, vec2(1.0)))) {
    vec4 icon = texture2D(tex, iconUv);
    color = icon + color * (1.0 - icon.a);
}
float edge = rim * mix(rimTop, rimBottom, uv.y) * (1.08 - 0.16 * uv.x);
color = vec4(vec3(rimGray * edge), edge) + color * (1.0 - edge);
cogl_color_out = color * cogl_color_in.a;
`;

const IconTile = GObject.registerClass(
class IconTile extends shaderEffect('IconTileBase',
    TILE_DECLARATIONS, TILE_CODE, true) {
    vfunc_paint_target(node, paintContext) {
        const actor = this.get_actor();
        const style = TILE_STYLES[Main.getStyleVariant()] ?? TILE_STYLES.dark;
        this.setUniform('size', [actor.width * actor.get_resource_scale()]);
        this.setUniform('top', [style.top]);
        this.setUniform('bottom', [style.bottom]);
        this.setUniform('rimGray', [style.rim]);
        this.setUniform('rimTop', [style.rimTop]);
        this.setUniform('rimBottom', [style.rimBottom]);
        super.vfunc_paint_target(node, paintContext);
    }
});

// Every app icon of the shell is built by BaseIcon, in the dash, the app
// grid and the search. Each new icon texture gets the tile, folders keep
// their own look because they are no plain icon.
class IconTiles {
    constructor() {
        this._injections = new InjectionManager();
        this._injections.overrideMethod(IconGrid.BaseIcon.prototype, '_createIconTexture',
            original => function (...args) {
                original.call(this, ...args);
                if (this.icon instanceof St.Icon)
                    this.icon.add_effect_with_name('kait2en-vornehm-tile', new IconTile());
            });
        this._rebuild();
    }

    // Icons that exist already are built again, with or without tile.
    _rebuild(actor = global.stage) {
        if (actor instanceof IconGrid.BaseIcon && actor.icon)
            actor.update();
        for (const child of actor.get_children())
            this._rebuild(child);
    }

    destroy() {
        this._injections.clear();
        this._rebuild();
    }
}

// The icon under the pointer lifts and grows a little. The others only
// move aside to make room, the shelf keeps its size.
class DockLift {
    constructor(dash) {
        this._dash = dash;
        this._lifted = null;
        this._paused = false;

        // The icons keep their motion events, so the stage is watched
        // while the overview is open.
        Main.overview.connectObject(
            'showing', () => this._watch(true),
            'hidden', () => this._watch(false),
            'item-drag-begin', () => this._pause(true),
            'item-drag-end', () => this._pause(false),
            'item-drag-cancelled', () => this._pause(false),
            this);
        if (Main.overview.visible)
            this._watch(true);
    }

    _watch(on) {
        global.stage.disconnectObject(this);
        if (on) {
            global.stage.connectObject('captured-event', (stage, event) => {
                if (event.type() === Clutter.EventType.MOTION ||
                    event.type() === Clutter.EventType.LEAVE)
                    this._lift(this._itemAt(stage.get_event_actor(event)));
                return Clutter.EVENT_PROPAGATE;
            }, this);
        } else {
            this._lift(null, false);
        }
    }

    // The dash item that holds the actor, if it is an app or show apps.
    _itemAt(actor) {
        const {_box: box, _showAppsIcon: showApps} = this._dash;
        for (; actor; actor = actor.get_parent()) {
            if (actor === showApps)
                return actor;
            if (actor.get_parent() === box)
                return actor.child?._delegate?.icon ? actor : null;
        }
        return null;
    }

    _lift(item, animate = true) {
        if (this._paused)
            item = null;
        if (item === this._lifted)
            return;
        this._lifted = item;

        // A lifted icon reaches above the row, which clips its children.
        if (item)
            this._dash._box.clip_to_allocation = false;

        // The grown icon needs room, the others only move aside.
        const items = [...this._dash._box.get_children(), this._dash._showAppsIcon];
        const index = items.indexOf(item);
        const room = item ? item.child.width * (DOCK_GROW - 1) / 2 : 0;
        items.forEach((actor, i) => {
            const lifted = i === index;
            const shift = index < 0 || lifted ? 0 : Math.sign(i - index) * room;
            this._move(actor, shift, lifted, animate);
        });
    }

    _move(item, shift, lifted, animate) {
        const params = {
            duration: animate ? DOCK_LIFT_TIME : 0,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
        };
        item.ease({
            ...params,
            translation_x: shift,
            translation_y: lifted ? -DOCK_LIFT : 0,
            onStopped: isFinished => {
                if (isFinished && !this._lifted)
                    this._dash._box.clip_to_allocation = true;
            },
        });

        const icon = item.child;
        if (!icon)
            return;
        const scale = lifted ? DOCK_GROW : 1;
        icon.set_pivot_point(0.5, 1);
        icon.ease({...params, scale_x: scale, scale_y: scale});

        // The shell places the label above the icon at rest size.
        if (item.label)
            item.label.translation_y = lifted ? -icon.height * (DOCK_GROW - 1) : 0;
    }

    _pause(paused) {
        this._lift(null, false);
        this._paused = paused;
    }

    destroy() {
        Main.overview.disconnectObject(this);
        this._watch(false);
    }
}

// The shell scales app icons up to 96px to fill the page. A fixed size
// and denser modes give the airy grid of a launcher.
class AppGrid {
    constructor() {
        this._appDisplay = Main.overview._overview.controls.appDisplay;
        this._grid = this._appDisplay._grid;
        watchPageSize(this._appDisplay);
        this._grid.setGridModes(APP_GRID_MODES);
        this._grid.layoutManager.fixedIconSize = APP_ICON_SIZE;
        relayout(this._grid);
    }

    destroy() {
        watchPageSize(this._appDisplay);
        this._grid.setGridModes(null);
        this._grid.layoutManager.fixedIconSize = -1;
        relayout(this._grid);
    }
}

// Mode and icon size only refresh on a size change, so both are reset.
function relayout(grid) {
    grid._currentMode = -1;
    grid.layoutManager._pageWidth = 0;
    grid.queue_relayout();
}

// Every app stays on the page it was put on. Once the new page size is
// known, the apps are packed again in their current order.
function watchPageSize(appDisplay) {
    const grid = appDisplay._grid;
    const oldSize = grid.itemsPerPage;
    const layout = grid.layoutManager;
    const ids = [];
    const check = () => {
        if (grid.itemsPerPage === oldSize)
            return;
        ids.forEach(id => layout.disconnect(id));
        GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
            repackPages(appDisplay);
            return GLib.SOURCE_REMOVE;
        });
    };
    ids.push(layout.connect('notify::rows-per-page', check));
    ids.push(layout.connect('notify::columns-per-page', check));
}

function repackPages(appDisplay) {
    const grid = appDisplay._grid;
    const items = appDisplay._orderedItems.slice()
        .sort(appDisplay._compareItems.bind(appDisplay));
    items.forEach(item => grid.removeItem(item));
    items.forEach(item => grid.appendItem(item));

    // A saved custom order is kept in sync with the new pages.
    if (appDisplay._pageManager.pages.length > 0)
        appDisplay._savePages();
}

// GNOME Settings only offers the default and the dark style, and the
// shell stays dark on default. Vornehm follows the switch, so default
// means a light shell. The shell sets the scheme of its mode again on
// every mode change, a notify on the setting reloads all stylesheets.
class LightStyle {
    constructor() {
        Main.sessionMode.connectObject('updated', () => this._apply(), this);
        this._apply();
    }

    _apply() {
        this._set('prefer-dark', 'prefer-light');
    }

    _set(from, to) {
        if (Main.sessionMode.colorScheme !== from)
            return;
        Main.sessionMode.colorScheme = to;
        St.Settings.get().notify('color-scheme');
    }

    destroy() {
        Main.sessionMode.disconnectObject(this);
        this._set('prefer-light', 'prefer-dark');
    }
}

export default class Kait2enVornehmExtension extends Extension {
    enable() {
        this._overviewBackground = new OverviewBackground();
        this._panelBlur = new PanelBlur();
        this._appGrid = new AppGrid();
        this._dockLift = new DockLift(Main.overview.dash);
        this._iconTiles = new IconTiles();
        this._glass = new Glass();
        this._lightStyle = new LightStyle();
    }

    disable() {
        this._lightStyle?.destroy();
        this._lightStyle = null;
        this._glass?.destroy();
        this._glass = null;
        this._iconTiles?.destroy();
        this._iconTiles = null;
        this._dockLift?.destroy();
        this._dockLift = null;
        this._appGrid?.destroy();
        this._appGrid = null;
        this._panelBlur?.destroy();
        this._panelBlur = null;
        this._overviewBackground?.destroy();
        this._overviewBackground = null;
    }
}

// SPDX-License-Identifier: GPL-3.0-only

var active = null;
var TRIES = 30;
var DELAY = 1000;

function emit(window) {
    if (window) {
        callDBus('org.touchbar.DynamicShortcuts',
                 '/org/touchbar/DynamicShortcuts',
                 'org.touchbar.DynamicShortcuts',
                 'SetActiveWindow',
                 window.resourceClass, window.caption, window.pid);
    } else {
        callDBus('org.touchbar.DynamicShortcuts',
                 '/org/touchbar/DynamicShortcuts',
                 'org.touchbar.DynamicShortcuts',
                 'SetActiveWindow',
                 '', '', 0);
    }
}

function onCaption() {
    emit(active);
}

function onActivated(w) {
    if (active) {
        try { active.captionChanged.disconnect(onCaption); } catch (e) {}
    }
    active = w;
    if (w) {
        try { w.captionChanged.connect(onCaption); } catch (e) {}
    }
    emit(w);
}

// Poll NameHasOwner via org.freedesktop.DBus (always available) so the
// callback always fires. When react-drm is detected, emit the current
// active window. Without this, the initial state is lost if react-drm
// hasn't registered its D-Bus name yet.
function initialHandshake(remaining) {
    callDBus('org.freedesktop.DBus', '/org/freedesktop/DBus',
             'org.freedesktop.DBus', 'NameHasOwner',
             'org.touchbar.DynamicShortcuts',
             function(hasOwner, error) {
                 if (!error && hasOwner) {
                     emit(workspace.activeWindow);
                 } else if (remaining > 0) {
                     setTimeout(function() {
                         initialHandshake(remaining - 1);
                     }, DELAY);
                 }
             });
}

workspace.windowActivated.connect(onActivated);
initialHandshake(TRIES);

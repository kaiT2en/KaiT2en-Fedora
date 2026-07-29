// SPDX-License-Identifier: GPL-3.0-only

var active = null;
var TRIES = 30;
var DELAY = 1000;
var lastOwner = false;

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

function checkService(retries) {
    callDBus('org.freedesktop.DBus', '/org/freedesktop/DBus',
             'org.freedesktop.DBus', 'NameHasOwner',
             'org.touchbar.DynamicShortcuts',
             function(hasOwner, error) {
                 var available = !error && hasOwner;
                 if (available && !lastOwner) {
                     emit(workspace.activeWindow);
                 }
                 lastOwner = available;
                 var nextDelay = retries > 0 ? DELAY : 10000;
                 setTimeout(function() {
                     checkService(retries > 0 ? retries - 1 : 0);
                 }, nextDelay);
             });
}

workspace.windowActivated.connect(onActivated);
checkService(TRIES);

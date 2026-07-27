// SPDX-License-Identifier: GPL-3.0-only

var active = null;

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

workspace.windowActivated.connect(onActivated);
onActivated(workspace.activeWindow);

// SPDX-License-Identifier: GPL-3.0-only

var active = null;

function emit(window) {
    callDBus('org.touchbar.DynamicShortcuts',
             '/org/touchbar/DynamicShortcuts',
             'org.touchbar.DynamicShortcuts',
             'SetActiveWindow',
             window ? window.resourceClass : '',
             window ? window.caption : '',
             window ? window.pid : 0);
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
emit(workspace.activeWindow);

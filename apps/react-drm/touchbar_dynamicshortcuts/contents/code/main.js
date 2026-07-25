// SPDX-License-Identifier: GPL-3.0-only

function setActiveWindow(window) {
    if (!window) {
        return;
    }
    callDBus('org.touchbar.DynamicShortcuts',
             '/org/touchbar/DynamicShortcuts',
             'org.touchbar.DynamicShortcuts',
             'SetActiveWindow',
             window.resourceClass, window.caption);
}

if (workspace.windowActivated) {
    workspace.windowActivated.connect(setActiveWindow);
} else if (workspace.clientActivated) {
    workspace.clientActivated.connect(setActiveWindow);
}

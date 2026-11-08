// SPDX-License-Identifier: GPL-3.0-or-later

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension, InjectionManager} from 'resource:///org/gnome/shell/extensions/extension.js';

function followBacklight(manager) {
    const global = manager.globalScale;
    const scales = [...manager.scales];
    if (!global || scales.length === 0 || manager._inhibitUpdates)
        return;

    const max = Math.max(...scales.map(scale => scale.value));
    if (Math.abs(global.value - max) < 0.001)
        return;

    manager._inhibitUpdates = true;
    try {
        if (max > 0.01)
            scales.forEach(scale => scale.updateScaleFactor(max));
        global.value = max;
    } finally {
        manager._inhibitUpdates = false;
    }
}

export default class AutoBrightnessExtension extends Extension {
    enable() {
        const manager = Main.brightnessManager;
        if (!manager)
            return;

        this._injections = new InjectionManager();
        this._injections.overrideMethod(manager, '_sync', original => function (...args) {
            original.call(this, ...args);
            followBacklight(this);
        });
        followBacklight(manager);
    }

    disable() {
        this._injections?.clear();
        this._injections = null;
    }
}

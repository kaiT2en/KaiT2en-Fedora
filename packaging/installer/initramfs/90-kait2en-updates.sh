#!/bin/sh

source_image=/kait2en-anaconda-updates.img
target_directory=/run/kait2en

if [ ! -f "$source_image" ]; then
    warn "KaiT2en: Anaconda updates image is missing"
    exit 1
fi

mkdir -p "$target_directory"
cp "$source_image" "$target_directory/updates.img"

firmware_source=/kait2en-wifi-firmware
helper_source=/usr/lib/kait2en
runtime_units=/run/systemd/system

if [ -d "$firmware_source" ]; then
    mkdir -p "$target_directory/apple-firmware"
    cp -r "$firmware_source"/. "$target_directory/apple-firmware/"

    # /run survives the switch root, so the live session can install this
    # firmware for itself. Nothing placed here reaches the installed system.
    for helper in install-wifi-firmware.sh install-bt-firmware.sh \
        kait2en-live-wifi kait2en-live-bluetooth kait2en-live-diagnostics; do
        if [ -f "$helper_source/$helper" ]; then
            cp "$helper_source/$helper" "$target_directory/$helper"
            chmod 0755 "$target_directory/$helper"
        else
            warn "KaiT2en: live firmware helper is missing: $helper"
        fi
    done

    mkdir -p "$runtime_units/multi-user.target.wants"
    for unit_name in kait2en-live-wifi.service kait2en-live-bluetooth.service; do
        if [ -f "$helper_source/$unit_name" ]; then
            cp "$helper_source/$unit_name" "$runtime_units/$unit_name"
            chmod 0644 "$runtime_units/$unit_name"
            ln -sf "../$unit_name" \
                "$runtime_units/multi-user.target.wants/$unit_name"
        else
            warn "KaiT2en: live service unit is missing: $unit_name"
        fi
    done
fi

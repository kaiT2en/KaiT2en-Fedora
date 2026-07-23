```bash
# helper script
sudo tee /usr/local/sbin/kait2en-amdgpu-profile >/dev/null <<'EOF'
#!/bin/sh
set -eu

attempt=0
while [ "$attempt" -lt 50 ]; do
for vendor_file in /sys/class/drm/card*/device/vendor; do
[ -r "$vendor_file" ] || continue
[ "$(cat "$vendor_file")" = "0x1002" ] || continue

device="${vendor_file%/vendor}"
performance="$device/power_dpm_force_performance_level"
profile="$device/pp_power_profile_mode"

[ -w "$performance" ] || continue
[ -w "$profile" ] || continue

echo manual > "$performance"
echo 2 > "$profile"
exit 0
done

attempt=$((attempt + 1))
sleep 0.1
done

echo "No configurable AMDGPU device found" >&2
exit 1
EOF

sudo chmod 755 /usr/local/sbin/kait2en-amdgpu-profile

# Boot-Unit

sudo tee /etc/systemd/system/kait2en-amdgpu-profile.service >/dev/null <<'EOF'
[Unit]
Description=Set AMDGPU power profile
After=systemd-modules-load.service
Before=display-manager.service

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/kait2en-amdgpu-profile
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOF

# Resume-Unit

sudo tee /etc/systemd/system/kait2en-amdgpu-profile-resume.service >/dev/null <<'EOF'
[Unit]
Description=Restore AMDGPU power profile after resume
Before=sleep.target
StopWhenUnneeded=yes

[Service]
Type=oneshot
ExecStart=/bin/true
ExecStop=/usr/local/sbin/kait2en-amdgpu-profile
RemainAfterExit=yes

[Install]
WantedBy=sleep.target
EOF

# activate

sudo systemctl daemon-reload
sudo systemctl enable --now kait2en-amdgpu-profile.service
sudo systemctl enable kait2en-amdgpu-profile-resume.service
``` 
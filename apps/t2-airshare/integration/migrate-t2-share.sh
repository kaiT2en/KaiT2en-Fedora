#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# T2 AirShare was called T2 Share. Removes the files installed under the old
# name and moves every user's activation of t2-share.service over to
# t2-airshare.service, a running user manager switches right away. The
# settings move on their own, see src/config.rs. Run as root by make install.
set -u
PREFIX=${1:-/usr/local}
USER_UNIT_DIR=$PREFIX/lib/systemd/user
NEW_UNIT=$USER_UNIT_DIR/t2-airshare.service

rm -f \
	"$PREFIX/bin/t2-share" \
	"$PREFIX/bin/t2share" \
	"$PREFIX/share/applications/org.t2share.gtk.desktop" \
	"$PREFIX/share/applications/org.t2share.send.desktop" \
	"$PREFIX/share/icons/hicolor/scalable/apps/org.t2share.gtk.svg" \
	"$PREFIX/share/kio/servicemenus/t2-share-send.desktop" \
	"$PREFIX/share/nemo/actions/t2-share-send.nemo_action" \
	"$PREFIX/share/nautilus-python/extensions/t2-share-send.py"
rm -rf "$PREFIX/share/licenses/t2-share"

while IFS=: read -r user _ uid _ _ home _; do
	((uid >= 1000)) || continue
	wants=$home/.config/systemd/user/graphical-session.target.wants
	[[ -L $wants/t2-share.service ]] || continue
	echo "moving $user from t2-share.service to t2-airshare.service"
	ln -sfn "$NEW_UNIT" "$wants/t2-airshare.service"
	chown -h "$uid:" "$wants/t2-airshare.service"
	rm -f "$wants/t2-share.service"
	if systemctl is-active --quiet "user@$uid.service"; then
		systemctl --user -M "$user@" stop t2-share.service 2>/dev/null
		systemctl --user -M "$user@" daemon-reload
		systemctl --user -M "$user@" start t2-airshare.service ||
			echo "could not start t2-airshare.service for $user, it starts with the next login"
	fi
done < <(getent passwd)

rm -f "$USER_UNIT_DIR/t2-share.service"
exit 0

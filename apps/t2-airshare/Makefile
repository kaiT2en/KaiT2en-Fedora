CARGO ?= cargo
INSTALL ?= install
GTK_UPDATE_ICON_CACHE ?= gtk-update-icon-cache

APP_ID := org.t2airshare.gtk
BIN_NAME := t2-airshare
CLI_NAME := t2airshare
BUILD_DIR := $(or $(CARGO_TARGET_DIR),target)/release
BIN_SRC := $(BUILD_DIR)/$(BIN_NAME)
CLI_SRC := $(BUILD_DIR)/$(CLI_NAME)
HOTSPOT_NAME := t2-quickshare-hotspot
HOTSPOT_SRC := $(BUILD_DIR)/$(HOTSPOT_NAME)
DESKTOP_SRC := $(APP_ID).desktop
ICON_SRC := assets/icons/hicolor/scalable/apps/$(APP_ID).svg
UNIT_SRC := systemd/t2-airshare.service

PREFIX ?= /usr/local
BIN_DIR := $(PREFIX)/bin
APPS_DIR := $(PREFIX)/share/applications
ICONS_DIR := $(PREFIX)/share/icons/hicolor/scalable/apps
ICON_THEME_DIR := $(PREFIX)/share/icons/hicolor
USER_UNIT_DIR := $(PREFIX)/lib/systemd/user
LIBEXEC_DIR := $(PREFIX)/libexec
SYSTEM_UNIT_DIR := $(PREFIX)/lib/systemd/system
# The system bus reads policies from /usr/share and /etc only, activation
# files also from /usr/local/share
DBUS_POLICY_DIR := /usr/share/dbus-1/system.d
DBUS_SERVICE_DIR := $(PREFIX)/share/dbus-1/system-services
LICENSE_DIR := $(PREFIX)/share/licenses/t2-airshare
SERVICEMENU_DIR := $(PREFIX)/share/kio/servicemenus
NEMO_ACTION_DIR := $(PREFIX)/share/nemo/actions
NAUTILUS_EXT_DIR := $(PREFIX)/share/nautilus-python/extensions
UPDATE_DESKTOP_DATABASE ?= update-desktop-database

.PHONY: build check-build install uninstall clean

build:
	$(CARGO) build --release --locked

check-build:
	@test -x "$(BIN_SRC)" -a -x "$(CLI_SRC)" -a -x "$(HOTSPOT_SRC)" || { echo "error: $(BIN_SRC), $(CLI_SRC) or $(HOTSPOT_SRC) is missing; run 'make build' as your normal user first" >&2; exit 1; }

install: check-build
	$(INSTALL) -d -o root -g root -m 0755 $(BIN_DIR) $(APPS_DIR) $(ICONS_DIR) $(ICON_THEME_DIR) $(USER_UNIT_DIR) $(LICENSE_DIR) \
		$(SERVICEMENU_DIR) $(NEMO_ACTION_DIR) $(NAUTILUS_EXT_DIR) \
		$(LIBEXEC_DIR) $(SYSTEM_UNIT_DIR) $(DBUS_POLICY_DIR) $(DBUS_SERVICE_DIR)
	$(INSTALL) -o root -g root -m 0755 $(BIN_SRC) $(BIN_DIR)/$(BIN_NAME)
	$(INSTALL) -o root -g root -m 0755 $(CLI_SRC) $(BIN_DIR)/$(CLI_NAME)
	@# a running helper keeps the old binary, it starts again on demand
	-systemctl stop $(HOTSPOT_NAME).service
	$(INSTALL) -o root -g root -m 0755 $(HOTSPOT_SRC) $(LIBEXEC_DIR)/$(HOTSPOT_NAME)
	$(INSTALL) -o root -g root -m 0644 systemd/$(HOTSPOT_NAME).service $(SYSTEM_UNIT_DIR)/$(HOTSPOT_NAME).service
	$(INSTALL) -o root -g root -m 0644 dbus/org.t2airshare.QuickShareHotspot.conf $(DBUS_POLICY_DIR)/org.t2airshare.QuickShareHotspot.conf
	$(INSTALL) -o root -g root -m 0644 dbus/org.t2airshare.QuickShareHotspot.service $(DBUS_SERVICE_DIR)/org.t2airshare.QuickShareHotspot.service
	systemctl daemon-reload
	-busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig
	$(INSTALL) -o root -g root -m 0644 $(DESKTOP_SRC) $(APPS_DIR)/$(APP_ID).desktop
	$(INSTALL) -o root -g root -m 0644 $(ICON_SRC) $(ICONS_DIR)/$(APP_ID).svg
	$(INSTALL) -o root -g root -m 0644 $(UNIT_SRC) $(USER_UNIT_DIR)/t2-airshare.service
	$(INSTALL) -o root -g root -m 0644 THIRD-PARTY-NOTICES.md proto/LICENSE-Apache-2.0 proto/LICENSE-Chromium $(LICENSE_DIR)/
	$(INSTALL) -o root -g root -m 0644 integration/org.t2airshare.send.desktop $(APPS_DIR)/org.t2airshare.send.desktop
	$(INSTALL) -o root -g root -m 0755 integration/t2-airshare-send.servicemenu.desktop $(SERVICEMENU_DIR)/t2-airshare-send.desktop
	$(INSTALL) -o root -g root -m 0644 integration/t2-airshare-send.nemo_action $(NEMO_ACTION_DIR)/t2-airshare-send.nemo_action
	$(INSTALL) -o root -g root -m 0644 integration/t2-airshare-send.py $(NAUTILUS_EXT_DIR)/t2-airshare-send.py
	@# The app was called T2 Share, take its files and user services over
	./integration/migrate-t2-share.sh $(PREFIX)
	$(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR)
	-$(UPDATE_DESKTOP_DATABASE) $(APPS_DIR)

uninstall:
	rm -f $(BIN_DIR)/$(BIN_NAME)
	rm -f $(BIN_DIR)/$(CLI_NAME)
	-systemctl stop $(HOTSPOT_NAME).service
	rm -f $(LIBEXEC_DIR)/$(HOTSPOT_NAME) $(SYSTEM_UNIT_DIR)/$(HOTSPOT_NAME).service \
		$(DBUS_POLICY_DIR)/org.t2airshare.QuickShareHotspot.conf $(DBUS_SERVICE_DIR)/org.t2airshare.QuickShareHotspot.service
	systemctl daemon-reload
	rm -f $(APPS_DIR)/$(APP_ID).desktop
	rm -f $(ICONS_DIR)/$(APP_ID).svg
	rm -f $(USER_UNIT_DIR)/t2-airshare.service
	rm -rf $(LICENSE_DIR)
	rm -f $(APPS_DIR)/org.t2airshare.send.desktop
	rm -f $(SERVICEMENU_DIR)/t2-airshare-send.desktop
	rm -f $(NEMO_ACTION_DIR)/t2-airshare-send.nemo_action
	rm -f $(NAUTILUS_EXT_DIR)/t2-airshare-send.py
	-$(UPDATE_DESKTOP_DATABASE) $(APPS_DIR)
	@if [ -d "$(ICON_THEME_DIR)" ]; then $(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR); fi

clean:
	$(CARGO) clean

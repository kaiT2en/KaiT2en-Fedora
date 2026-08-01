CARGO ?= cargo
INSTALL ?= install
GTK_UPDATE_ICON_CACHE ?= gtk-update-icon-cache

APP_ID := org.t2dgpucontrol.gtk
BIN_NAME := t2-dgpu-control
BUILD_DIR := target/release
BIN_SRC := $(BUILD_DIR)/$(BIN_NAME)
HELPER_SRC := contrib/t2-dgpu-control-helper
HELPER_NAME := t2-dgpu-control-helper
STATUS_HELPER_SRC := contrib/t2-dgpu-control-status
STATUS_HELPER_NAME := t2-dgpu-control-status
POLICY_SRC := contrib/org.t2dgpucontrol.policy
STATUS_POLICY_SRC := contrib/org.t2dgpucontrol.status.policy
LEGACY_UNITS := kait2en-dgpu-off.service kait2en-dgpu-suspend.service \
	kait2en-amdgpu-profile.service kait2en-amdgpu-profile-resume.service
ICON_SRC := assets/icons/hicolor/scalable/apps/$(APP_ID).svg

PREFIX ?= /usr/local
BIN_DIR := $(PREFIX)/bin
LIBEXEC_DIR := $(PREFIX)/libexec
APPS_DIR := $(PREFIX)/share/applications
ICONS_DIR := $(PREFIX)/share/icons/hicolor/scalable/apps
ICON_THEME_DIR := $(PREFIX)/share/icons/hicolor
POLICY_DIR ?= /usr/share/polkit-1/actions

.PHONY: build install uninstall clean

define remove_legacy_units
	-systemctl disable --now $(LEGACY_UNITS)
	$(RM) $(foreach unit,$(LEGACY_UNITS),/etc/systemd/system/$(unit))
	$(RM) $(foreach unit,$(LEGACY_UNITS),/usr/local/lib/systemd/system/$(unit))
	$(RM) $(foreach unit,$(LEGACY_UNITS),/usr/lib/systemd/system/$(unit))
	$(RM) -r /run/t2-dgpu-control
	systemctl daemon-reload
endef

build:
	$(CARGO) build --release

install:
	$(remove_legacy_units)
	$(INSTALL) -d -o root -g root -m 0755 $(BIN_DIR) $(LIBEXEC_DIR) $(APPS_DIR) $(ICONS_DIR) $(POLICY_DIR)
	$(INSTALL) -o root -g root -m 0755 $(BIN_SRC) $(BIN_DIR)/$(BIN_NAME)
	$(INSTALL) -o root -g root -m 0755 $(HELPER_SRC) $(LIBEXEC_DIR)/$(HELPER_NAME)
	$(INSTALL) -o root -g root -m 0755 $(STATUS_HELPER_SRC) $(LIBEXEC_DIR)/$(STATUS_HELPER_NAME)
	$(INSTALL) -o root -g root -m 0644 $(APP_ID).desktop $(APPS_DIR)/$(APP_ID).desktop
	$(INSTALL) -o root -g root -m 0644 $(ICON_SRC) $(ICONS_DIR)/$(APP_ID).svg
	$(INSTALL) -o root -g root -m 0644 $(POLICY_SRC) $(POLICY_DIR)/$(APP_ID).policy
	$(INSTALL) -o root -g root -m 0644 $(STATUS_POLICY_SRC) $(POLICY_DIR)/$(APP_ID).status.policy
	$(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR)
	systemctl daemon-reload

uninstall:
	$(remove_legacy_units)
	rm -f $(BIN_DIR)/$(BIN_NAME) $(LIBEXEC_DIR)/$(HELPER_NAME) $(LIBEXEC_DIR)/$(STATUS_HELPER_NAME)
	rm -f $(APPS_DIR)/$(APP_ID).desktop $(ICONS_DIR)/$(APP_ID).svg
	rm -f $(POLICY_DIR)/$(APP_ID).policy $(POLICY_DIR)/$(APP_ID).status.policy
	@if [ -d "$(ICON_THEME_DIR)" ]; then $(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR); fi
	systemctl daemon-reload

clean:
	$(CARGO) clean

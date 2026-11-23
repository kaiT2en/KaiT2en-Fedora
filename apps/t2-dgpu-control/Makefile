# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
CARGO ?= cargo
INSTALL ?= install
GTK_UPDATE_ICON_CACHE ?= gtk-update-icon-cache

APP_ID := org.t2dgpucontrol.gtk
BIN_NAME := t2-dgpu-control
BUILD_DIR := $(or $(CARGO_TARGET_DIR),target)/release
BIN_SRC := $(BUILD_DIR)/$(BIN_NAME)
HELPER_SRC := contrib/t2-dgpu-control-helper
HELPER_NAME := t2-dgpu-control-helper
STATUS_HELPER_SRC := contrib/t2-dgpu-control-status
STATUS_HELPER_NAME := t2-dgpu-control-status
POLICY_SRC := contrib/org.t2dgpucontrol.policy
STATUS_POLICY_SRC := contrib/org.t2dgpucontrol.status.policy
UNIT_NAME := kait2en-dgpu-off.service
UNIT_SRC := systemd/$(UNIT_NAME)
SUSPEND_UNIT_NAME := kait2en-dgpu-suspend.service
SUSPEND_UNIT_SRC := systemd/$(SUSPEND_UNIT_NAME)
PROFILE_UNIT_NAME := kait2en-amdgpu-profile.service
PROFILE_UNIT_SRC := systemd/$(PROFILE_UNIT_NAME)
PROFILE_RESUME_UNIT_NAME := kait2en-amdgpu-profile-resume.service
PROFILE_RESUME_UNIT_SRC := systemd/$(PROFILE_RESUME_UNIT_NAME)
ICON_SRC := assets/icons/hicolor/scalable/apps/$(APP_ID).svg

PREFIX ?= /usr/local
BIN_DIR := $(PREFIX)/bin
LIBEXEC_DIR := $(PREFIX)/libexec
APPS_DIR := $(PREFIX)/share/applications
ICONS_DIR := $(PREFIX)/share/icons/hicolor/scalable/apps
ICON_THEME_DIR := $(PREFIX)/share/icons/hicolor
UNIT_DIR := $(PREFIX)/lib/systemd/system
POLICY_DIR ?= /usr/share/polkit-1/actions

.PHONY: build check-build install uninstall clean

define install_unit
	@enabled=0; systemctl is-enabled --quiet $(1) && enabled=1; \
		$(RM) /etc/systemd/system/$(1); \
		$(INSTALL) -o root -g root -m 0644 $(2) $(UNIT_DIR)/$(1); \
		systemctl daemon-reload; \
		if [ $$enabled -eq 1 ]; then systemctl enable $(1); fi
endef

build:
	$(CARGO) build --release --locked

check-build:
	@test -x "$(BIN_SRC)" || { echo "error: $(BIN_SRC) is missing; run 'make build' as your normal user first" >&2; exit 1; }

install: check-build
	$(INSTALL) -d -o root -g root -m 0755 $(BIN_DIR) $(LIBEXEC_DIR) $(APPS_DIR) $(ICONS_DIR) $(UNIT_DIR) $(POLICY_DIR)
	$(INSTALL) -o root -g root -m 0755 $(BIN_SRC) $(BIN_DIR)/$(BIN_NAME)
	$(INSTALL) -o root -g root -m 0755 $(HELPER_SRC) $(LIBEXEC_DIR)/$(HELPER_NAME)
	$(INSTALL) -o root -g root -m 0755 $(STATUS_HELPER_SRC) $(LIBEXEC_DIR)/$(STATUS_HELPER_NAME)
	$(INSTALL) -o root -g root -m 0644 $(APP_ID).desktop $(APPS_DIR)/$(APP_ID).desktop
	$(INSTALL) -o root -g root -m 0644 $(ICON_SRC) $(ICONS_DIR)/$(APP_ID).svg
	$(call install_unit,$(UNIT_NAME),$(UNIT_SRC))
	$(call install_unit,$(SUSPEND_UNIT_NAME),$(SUSPEND_UNIT_SRC))
	$(call install_unit,$(PROFILE_UNIT_NAME),$(PROFILE_UNIT_SRC))
	$(call install_unit,$(PROFILE_RESUME_UNIT_NAME),$(PROFILE_RESUME_UNIT_SRC))
	@if systemctl is-enabled --quiet $(UNIT_NAME); then \
		systemctl enable $(SUSPEND_UNIT_NAME); \
	else \
		systemctl disable $(SUSPEND_UNIT_NAME); \
	fi
	$(INSTALL) -o root -g root -m 0644 $(POLICY_SRC) $(POLICY_DIR)/$(APP_ID).policy
	$(INSTALL) -o root -g root -m 0644 $(STATUS_POLICY_SRC) $(POLICY_DIR)/$(APP_ID).status.policy
	$(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR)
	systemctl daemon-reload

uninstall:
	systemctl disable --now $(UNIT_NAME) $(SUSPEND_UNIT_NAME) $(PROFILE_UNIT_NAME) $(PROFILE_RESUME_UNIT_NAME) || true
	rm -f $(BIN_DIR)/$(BIN_NAME) $(LIBEXEC_DIR)/$(HELPER_NAME) $(LIBEXEC_DIR)/$(STATUS_HELPER_NAME)
	rm -f $(APPS_DIR)/$(APP_ID).desktop $(ICONS_DIR)/$(APP_ID).svg
	rm -f $(UNIT_DIR)/$(UNIT_NAME) $(UNIT_DIR)/$(SUSPEND_UNIT_NAME)
	rm -f $(UNIT_DIR)/$(PROFILE_UNIT_NAME) $(UNIT_DIR)/$(PROFILE_RESUME_UNIT_NAME)
	rm -f $(POLICY_DIR)/$(APP_ID).policy $(POLICY_DIR)/$(APP_ID).status.policy
	@if [ -d "$(ICON_THEME_DIR)" ]; then $(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR); fi
	systemctl daemon-reload

clean:
	$(CARGO) clean

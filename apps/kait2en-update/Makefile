# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
CARGO ?= cargo
INSTALL ?= install
GTK_UPDATE_ICON_CACHE ?= gtk-update-icon-cache

APP_ID := org.kait2enupdate.gtk
BIN_NAME := kait2en-update
BUILD_DIR := $(or $(CARGO_TARGET_DIR),target)/release
BIN_SRC := $(BUILD_DIR)/$(BIN_NAME)
DESKTOP_SRC := $(APP_ID).desktop
ICON_SRC := $(APP_ID).svg
HELPER_SRC := kait2en-update-helper
POLICY_SRC := org.kait2enupdate.policy

PREFIX ?= /usr/local
BIN_DIR := $(PREFIX)/bin
LIBEXEC_DIR := $(PREFIX)/libexec
APPS_DIR := $(PREFIX)/share/applications
ICONS_DIR := $(PREFIX)/share/icons/hicolor/scalable/apps
ICON_THEME_DIR := $(PREFIX)/share/icons/hicolor
POLICY_DIR ?= /usr/share/polkit-1/actions

.PHONY: all build test check-build install uninstall clean rebuild

all: build

build:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --locked

check-build:
	@test -x "$(BIN_SRC)" || { echo "error: $(BIN_SRC) is missing; run 'make build' as your normal user first" >&2; exit 1; }

install: check-build
	$(INSTALL) -d -o root -g root -m 0755 $(BIN_DIR) $(LIBEXEC_DIR) $(APPS_DIR) $(ICONS_DIR) $(ICON_THEME_DIR) $(POLICY_DIR)
	$(INSTALL) -o root -g root -m 0755 $(BIN_SRC) $(BIN_DIR)/$(BIN_NAME)
	$(INSTALL) -o root -g root -m 0755 $(HELPER_SRC) $(LIBEXEC_DIR)/$(HELPER_SRC)
	$(INSTALL) -o root -g root -m 0644 $(POLICY_SRC) $(POLICY_DIR)/$(POLICY_SRC)
	$(INSTALL) -o root -g root -m 0644 $(DESKTOP_SRC) $(APPS_DIR)/$(APP_ID).desktop
	$(INSTALL) -o root -g root -m 0644 $(ICON_SRC) $(ICONS_DIR)/$(APP_ID).svg
	$(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR)

uninstall:
	rm -f $(BIN_DIR)/$(BIN_NAME) $(LIBEXEC_DIR)/$(HELPER_SRC) $(POLICY_DIR)/$(POLICY_SRC)
	rm -f $(APPS_DIR)/$(APP_ID).desktop $(ICONS_DIR)/$(APP_ID).svg
	@if [ -d "$(ICON_THEME_DIR)" ]; then $(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR); fi

clean:
	$(CARGO) clean

rebuild: clean build

CARGO ?= cargo
INSTALL ?= install
GTK_UPDATE_ICON_CACHE ?= gtk-update-icon-cache

APP_ID := org.t2fancontrol.gtk
BIN_NAME := t2-fancontrol-gtk
BUILD_DIR := target/release
BIN_SRC := $(BUILD_DIR)/$(BIN_NAME)
DESKTOP_SRC := $(APP_ID).desktop
ICON_SRC := assets/icons/hicolor/scalable/apps/$(APP_ID).svg
UNIT_SRC := systemd/t2-fancontrol.service

PREFIX ?= /usr/local
BIN_DIR := $(PREFIX)/bin
APPS_DIR := $(PREFIX)/share/applications
ICONS_DIR := $(PREFIX)/share/icons/hicolor/scalable/apps
ICON_THEME_DIR := $(PREFIX)/share/icons/hicolor
UNIT_DIR := $(PREFIX)/lib/systemd/system

USER_PREFIX := $(HOME)/.local
USER_BIN_DIR := $(USER_PREFIX)/bin
USER_APPS_DIR := $(USER_PREFIX)/share/applications
USER_ICONS_DIR := $(USER_PREFIX)/share/icons/hicolor/scalable/apps
USER_ICON_THEME_DIR := $(USER_PREFIX)/share/icons/hicolor

.PHONY: all build install uninstall install-user uninstall-user clean rebuild

all: build

build:
	$(CARGO) build --release

install:
	@if systemctl list-unit-files t2fanrd.service >/dev/null 2>&1; then systemctl disable --now t2fanrd.service || true; fi
	$(INSTALL) -d -o root -g root -m 0755 $(BIN_DIR) $(APPS_DIR) $(ICONS_DIR) $(ICON_THEME_DIR) $(UNIT_DIR)
	$(INSTALL) -o root -g root -m 0755 $(BIN_SRC) $(BIN_DIR)/$(BIN_NAME)
	$(INSTALL) -o root -g root -m 0644 $(DESKTOP_SRC) $(APPS_DIR)/$(APP_ID).desktop
	$(INSTALL) -o root -g root -m 0644 $(ICON_SRC) $(ICONS_DIR)/$(APP_ID).svg
	$(INSTALL) -o root -g root -m 0644 $(UNIT_SRC) $(UNIT_DIR)/t2-fancontrol.service
	$(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR)
	systemctl daemon-reload
	systemctl enable t2-fancontrol.service
	systemctl restart t2-fancontrol.service

uninstall:
	systemctl disable --now t2-fancontrol.service || true
	rm -f $(BIN_DIR)/$(BIN_NAME)
	rm -f $(APPS_DIR)/$(APP_ID).desktop
	rm -f $(ICONS_DIR)/$(APP_ID).svg
	rm -f $(UNIT_DIR)/t2-fancontrol.service
	@if [ -d "$(ICON_THEME_DIR)" ]; then $(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR); fi
	systemctl daemon-reload
	@if systemctl list-unit-files t2fanrd.service >/dev/null 2>&1; then systemctl enable --now t2fanrd.service || true; fi

install-user:
	$(INSTALL) -d -m 0755 $(USER_BIN_DIR) $(USER_APPS_DIR) $(USER_ICONS_DIR) $(USER_ICON_THEME_DIR)
	$(INSTALL) -m 0755 $(BIN_SRC) $(USER_BIN_DIR)/$(BIN_NAME)
	$(INSTALL) -m 0644 $(DESKTOP_SRC) $(USER_APPS_DIR)/$(APP_ID).desktop
	$(INSTALL) -m 0644 $(ICON_SRC) $(USER_ICONS_DIR)/$(APP_ID).svg
	$(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(USER_ICON_THEME_DIR)

uninstall-user:
	rm -f $(USER_BIN_DIR)/$(BIN_NAME)
	rm -f $(USER_APPS_DIR)/$(APP_ID).desktop
	rm -f $(USER_ICONS_DIR)/$(APP_ID).svg
	@if [ -d "$(USER_ICON_THEME_DIR)" ]; then $(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(USER_ICON_THEME_DIR); fi

clean:
	$(CARGO) clean

rebuild: clean build

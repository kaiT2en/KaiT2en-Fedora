CARGO ?= cargo
INSTALL ?= install
GTK_UPDATE_ICON_CACHE ?= gtk-update-icon-cache

APP_ID := org.t2smccontrol.gtk
BIN_NAME := t2-smc-control
BUILD_DIR := target/release
BIN_SRC := $(BUILD_DIR)/$(BIN_NAME)
DESKTOP_SRC := $(APP_ID).desktop
ICON_SRC := assets/icons/hicolor/scalable/apps/$(APP_ID).svg
ICON_THEME_SRC := assets/icons/hicolor/index.theme
UNIT_NAME := kait2en-t2-smc-charge-limit.service
UNIT_SRC := systemd/$(UNIT_NAME)

PREFIX ?= /usr/local
BIN_DIR := $(PREFIX)/bin
APPS_DIR := $(PREFIX)/share/applications
ICONS_DIR := $(PREFIX)/share/icons/hicolor/scalable/apps
ICON_THEME_DIR := $(PREFIX)/share/icons/hicolor
UNIT_DIR := $(PREFIX)/lib/systemd/system

.PHONY: build install uninstall clean

build:
	$(CARGO) build --release

install:
	$(INSTALL) -d -o root -g root -m 0755 $(BIN_DIR) $(APPS_DIR) $(ICONS_DIR) $(ICON_THEME_DIR) $(UNIT_DIR)
	$(INSTALL) -o root -g root -m 0755 $(BIN_SRC) $(BIN_DIR)/$(BIN_NAME)
	$(INSTALL) -o root -g root -m 0644 $(DESKTOP_SRC) $(APPS_DIR)/$(APP_ID).desktop
	$(INSTALL) -o root -g root -m 0644 $(ICON_THEME_SRC) $(ICON_THEME_DIR)/index.theme
	$(INSTALL) -o root -g root -m 0644 $(ICON_SRC) $(ICONS_DIR)/$(APP_ID).svg
	$(INSTALL) -o root -g root -m 0644 $(UNIT_SRC) $(UNIT_DIR)/$(UNIT_NAME)
	$(GTK_UPDATE_ICON_CACHE) $(ICON_THEME_DIR)
	systemctl daemon-reload
	systemctl enable $(UNIT_NAME)

uninstall:
	systemctl disable --now $(UNIT_NAME) || true
	rm -f $(BIN_DIR)/$(BIN_NAME)
	rm -f $(APPS_DIR)/$(APP_ID).desktop
	rm -f $(ICONS_DIR)/$(APP_ID).svg
	rm -f $(UNIT_DIR)/$(UNIT_NAME)
	@if [ -f "$(ICON_THEME_DIR)/index.theme" ]; then $(GTK_UPDATE_ICON_CACHE) $(ICON_THEME_DIR); fi
	systemctl daemon-reload

clean:
	$(CARGO) clean

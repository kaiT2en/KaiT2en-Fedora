# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
CARGO ?= cargo
INSTALL ?= install
PREFIX ?= /usr/local
DESTDIR ?=

BIN := kait2en-touchbar
BIN_SRC := $(or $(CARGO_TARGET_DIR),target)/release/$(BIN)
BIN_DIR := $(DESTDIR)$(PREFIX)/bin
USER_UNIT_DIR := $(DESTDIR)$(PREFIX)/lib/systemd/user
SYSTEM_UNIT_DIR := $(DESTDIR)$(PREFIX)/lib/systemd/system
UDEV_DIR := $(DESTDIR)$(PREFIX)/lib/udev/rules.d
# polkit reads rules only from /etc and /usr/share, and /etc/polkit-1/rules.d
# keeps its restrictive mode, so the rule is installed without install -d
POLKIT_RULE := $(DESTDIR)/etc/polkit-1/rules.d/50-kait2en-touchbar.rules
CONFIG_DIR := $(DESTDIR)/etc/kait2en
LICENSE_DIR := $(DESTDIR)$(PREFIX)/share/licenses/kait2en-touchbar

.PHONY: all build test check-build install uninstall clean
all: build

build:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --locked

check-build:
	@test -x "$(BIN_SRC)" || { echo "error: $(BIN_SRC) is missing; run make build first" >&2; exit 1; }

install: check-build
	$(INSTALL) -d -m 0755 $(BIN_DIR) $(USER_UNIT_DIR) $(SYSTEM_UNIT_DIR) $(UDEV_DIR) $(CONFIG_DIR) $(LICENSE_DIR)
	$(INSTALL) -m 0755 $(BIN_SRC) $(BIN_DIR)/$(BIN)
	$(INSTALL) -m 0644 integration/systemd/user/kait2en-touchbar.service $(USER_UNIT_DIR)/kait2en-touchbar.service
	$(INSTALL) -m 0644 integration/systemd/user/kait2en-touchbar-panel.service $(USER_UNIT_DIR)/kait2en-touchbar-panel.service
	$(INSTALL) -m 0644 integration/systemd/system/kait2en-touchbar-attach.service $(SYSTEM_UNIT_DIR)/kait2en-touchbar-attach.service
	$(INSTALL) -m 0644 integration/systemd/system/kait2en-touchbar-session@.service $(SYSTEM_UNIT_DIR)/kait2en-touchbar-session@.service
	$(INSTALL) -D -m 0644 integration/polkit/50-kait2en-touchbar.rules $(POLKIT_RULE)
	$(INSTALL) -m 0644 integration/udev/99-zz-kait2en-touchbar.rules $(UDEV_DIR)/99-zz-kait2en-touchbar.rules
	$(INSTALL) -m 0644 LICENSE $(LICENSE_DIR)/LICENSE
	$(INSTALL) -m 0644 THIRD-PARTY-NOTICES.md $(LICENSE_DIR)/THIRD-PARTY-NOTICES.md
	@test -e $(CONFIG_DIR)/touchbar.toml || $(INSTALL) -m 0644 config/touchbar.toml $(CONFIG_DIR)/touchbar.toml
	rm -f $(DESTDIR)$(PREFIX)/lib/modprobe.d/kait2en-touchbar.conf

uninstall:
	rm -f $(BIN_DIR)/$(BIN)
	rm -f $(USER_UNIT_DIR)/kait2en-touchbar.service
	rm -f $(USER_UNIT_DIR)/kait2en-touchbar-panel.service
	rm -f $(SYSTEM_UNIT_DIR)/kait2en-touchbar-attach.service
	rm -f $(SYSTEM_UNIT_DIR)/kait2en-touchbar-session@.service
	rm -f $(POLKIT_RULE)
	rm -f $(UDEV_DIR)/99-zz-kait2en-touchbar.rules
	rm -f $(DESTDIR)$(PREFIX)/lib/modprobe.d/kait2en-touchbar.conf
	rm -f $(LICENSE_DIR)/THIRD-PARTY-NOTICES.md

clean:
	$(CARGO) clean

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
CARGO ?= cargo
CARGO_FLAGS ?= --locked
PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
LIBEXECDIR ?= $(PREFIX)/libexec
SYSTEMD_UNIT_DIR ?= $(PREFIX)/lib/systemd/system
TARGET_DIR ?= $(or $(CARGO_TARGET_DIR),target)

.PHONY: all build test install install-bin install-integration clean
all: build
build:
	$(CARGO) build --release $(CARGO_FLAGS) --target-dir $(TARGET_DIR)
test:
	$(CARGO) test $(CARGO_FLAGS) --target-dir $(TARGET_DIR)
install: install-bin install-integration
install-bin:
	install -D -m 0755 $(TARGET_DIR)/release/t2remote $(DESTDIR)$(BINDIR)/t2remote
install-integration:
	install -d $(DESTDIR)$(SYSTEMD_UNIT_DIR)
	sed 's|@BINDIR@|$(BINDIR)|g' integration/systemd/kait2en-t2-remote.service >$(DESTDIR)$(SYSTEMD_UNIT_DIR)/kait2en-t2-remote.service
	chmod 0644 $(DESTDIR)$(SYSTEMD_UNIT_DIR)/kait2en-t2-remote.service
	install -D -m 0755 integration/systemd/t2-ave-suspend $(DESTDIR)$(LIBEXECDIR)/t2-services/sleep.d/t2-ave
clean:
	$(CARGO) clean --target-dir $(TARGET_DIR)

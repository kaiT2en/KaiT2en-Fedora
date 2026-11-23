# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
CARGO ?= cargo
CARGO_FLAGS ?= --locked
TARGET_DIR ?= $(or $(CARGO_TARGET_DIR),target)
INSTALL ?= install
PREFIX ?= /usr/local

BIN_NAMES := t2journal
BIN_DIR := $(PREFIX)/bin

.PHONY: build test check-build install uninstall clean

build:
	$(CARGO) build --release $(CARGO_FLAGS) --target-dir $(TARGET_DIR)

test:
	$(CARGO) test $(CARGO_FLAGS) --target-dir $(TARGET_DIR)

check-build:
	@for bin in $(BIN_NAMES); do \
		test -x "$(TARGET_DIR)/release/$$bin" || { echo "error: binary $$bin is missing; run 'make' first" >&2; exit 1; }; \
	done

install: check-build
	$(INSTALL) -d -m 0755 $(DESTDIR)$(BIN_DIR)
	@for bin in $(BIN_NAMES); do \
		$(INSTALL) -m 0755 "$(TARGET_DIR)/release/$$bin" "$(DESTDIR)$(BIN_DIR)/$$bin"; \
	done

uninstall:
	$(RM) $(addprefix $(DESTDIR)$(BIN_DIR)/,$(BIN_NAMES))

clean:
	$(CARGO) clean --target-dir $(TARGET_DIR)

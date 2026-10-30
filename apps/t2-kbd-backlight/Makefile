CARGO ?= cargo
INSTALL ?= install
PREFIX ?= /usr/local
DESTDIR ?=

BIN := t2-kbd-backlight
BIN_SRC := $(or $(CARGO_TARGET_DIR),target)/release/$(BIN)
BIN_DIR := $(DESTDIR)$(PREFIX)/bin
UNIT_DIR := $(DESTDIR)$(PREFIX)/lib/systemd/system
CONFIG_DIR := $(DESTDIR)/etc/kait2en

.PHONY: all build test check-build install uninstall clean
all: build

build:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --locked

check-build:
	@test -x "$(BIN_SRC)" || { echo "error: $(BIN_SRC) is missing; run make build first" >&2; exit 1; }

install: check-build
	$(INSTALL) -d -m 0755 $(BIN_DIR) $(UNIT_DIR) $(CONFIG_DIR)
	$(INSTALL) -m 0755 $(BIN_SRC) $(BIN_DIR)/$(BIN)
	$(INSTALL) -m 0644 systemd/$(BIN).service $(UNIT_DIR)/$(BIN).service
	@test -e $(CONFIG_DIR)/kbd-backlight.toml || $(INSTALL) -m 0644 config/kbd-backlight.toml $(CONFIG_DIR)/kbd-backlight.toml
	systemctl daemon-reload
	systemctl enable $(BIN).service
	systemctl restart $(BIN).service

uninstall:
	systemctl disable --now $(BIN).service || true
	rm -f $(BIN_DIR)/$(BIN)
	rm -f $(UNIT_DIR)/$(BIN).service
	systemctl daemon-reload

clean:
	$(CARGO) clean

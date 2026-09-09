CARGO ?= cargo
INSTALL ?= install
PREFIX ?= /usr/local

BIN_NAMES := t2journal t2remote
BIN_DIR := $(PREFIX)/bin

.PHONY: build test check-build install uninstall clean

build:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --locked

check-build:
	@for bin in $(BIN_NAMES); do \
		test -x "target/release/$$bin" || { echo "error: target/release/$$bin is missing; run 'make' first" >&2; exit 1; }; \
	done

install: check-build
	$(INSTALL) -d -m 0755 $(DESTDIR)$(BIN_DIR)
	@for bin in $(BIN_NAMES); do \
		$(INSTALL) -m 0755 "target/release/$$bin" "$(DESTDIR)$(BIN_DIR)/$$bin"; \
	done

uninstall:
	$(RM) $(addprefix $(DESTDIR)$(BIN_DIR)/,$(BIN_NAMES))

clean:
	$(CARGO) clean

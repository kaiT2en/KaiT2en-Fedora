CARGO ?= cargo
INSTALL ?= install
PREFIX ?= /usr/local

BIN_NAME := t2journal
BIN_SRC := target/release/$(BIN_NAME)
BIN_DIR := $(PREFIX)/bin

.PHONY: build test check-build install uninstall clean

build:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --locked

check-build:
	@test -x "$(BIN_SRC)" || { echo "error: $(BIN_SRC) is missing; run 'make' first" >&2; exit 1; }

install: check-build
	$(INSTALL) -d -m 0755 $(DESTDIR)$(BIN_DIR)
	$(INSTALL) -m 0755 $(BIN_SRC) $(DESTDIR)$(BIN_DIR)/$(BIN_NAME)

uninstall:
	$(RM) $(DESTDIR)$(BIN_DIR)/$(BIN_NAME)

clean:
	$(CARGO) clean

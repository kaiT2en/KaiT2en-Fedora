CARGO ?= cargo
INSTALL ?= install

BIN_NAME := t2-touchid
BIN_SRC := target/release/$(BIN_NAME)

PREFIX ?= /usr/local
BIN_DIR := $(PREFIX)/bin

.PHONY: all build install uninstall clean
all: build

build:
	$(CARGO) build --release

install: $(BIN_SRC)
	$(INSTALL) -d -o root -g root -m 0755 $(DESTDIR)$(BIN_DIR)
	$(INSTALL) -o root -g root -m 0755 $(BIN_SRC) $(DESTDIR)$(BIN_DIR)/$(BIN_NAME)

uninstall:
	rm -f $(DESTDIR)$(BIN_DIR)/$(BIN_NAME)

clean:
	$(CARGO) clean

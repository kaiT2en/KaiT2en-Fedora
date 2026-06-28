CARGO ?= cargo
INSTALL ?= install
INSTALL_OWNER ?= -o root -g root

APP_ID := org.t2gpuswitch.gtk
BIN_NAME := t2-gpu-switch
HELPER_NAME := t2-gpu-switch-helper
BUILD_DIR := target/release
BIN_SRC := $(BUILD_DIR)/$(BIN_NAME)
DESKTOP_SRC := $(APP_ID).desktop

PREFIX ?= /usr/local
BIN_DIR := $(PREFIX)/bin
LIBEXEC_DIR := $(PREFIX)/libexec
APPS_DIR := $(PREFIX)/share/applications

.PHONY: build install uninstall clean

build:
	$(CARGO) build --release

install:
	$(INSTALL) -d $(INSTALL_OWNER) -m 0755 $(BIN_DIR) $(LIBEXEC_DIR) $(APPS_DIR)
	$(INSTALL) $(INSTALL_OWNER) -m 0755 $(BIN_SRC) $(BIN_DIR)/$(BIN_NAME)
	$(INSTALL) $(INSTALL_OWNER) -m 0755 $(HELPER_NAME) $(LIBEXEC_DIR)/$(HELPER_NAME)
	$(INSTALL) $(INSTALL_OWNER) -m 0644 $(DESKTOP_SRC) $(APPS_DIR)/$(APP_ID).desktop

uninstall:
	rm -f $(BIN_DIR)/$(BIN_NAME)
	rm -f $(LIBEXEC_DIR)/$(HELPER_NAME)
	rm -f $(APPS_DIR)/$(APP_ID).desktop

clean:
	$(CARGO) clean

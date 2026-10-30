CARGO ?= cargo
INSTALL ?= install

BUILD_DIR := $(or $(CARGO_TARGET_DIR),target)/release
BIN_SRC := $(BUILD_DIR)/t2drop

PREFIX ?= /usr/local
BIN_DIR := $(PREFIX)/bin
SYSTEM_UNIT_DIR := $(PREFIX)/lib/systemd/system
# The system bus reads policies from /usr/share and /etc only, activation
# files also from /usr/local/share
DBUS_POLICY_DIR := /usr/share/dbus-1/system.d
DBUS_SERVICE_DIR := $(PREFIX)/share/dbus-1/system-services
# In /etc, after 10-t2-services.conf of t2-services, so += appends to its
# unmanaged-devices
NM_CONF_DIR := /etc/NetworkManager/conf.d

.PHONY: build check-build install uninstall clean

build:
	$(CARGO) build --release --locked

check-build:
	@test -x "$(BIN_SRC)" || { echo "error: $(BIN_SRC) is missing; run 'make build' as your normal user first" >&2; exit 1; }

install: check-build
	$(INSTALL) -d -o root -g root -m 0755 $(BIN_DIR) $(SYSTEM_UNIT_DIR) $(DBUS_POLICY_DIR) $(DBUS_SERVICE_DIR)
	$(INSTALL) -o root -g root -m 0755 $(BIN_SRC) $(BIN_DIR)/t2drop
	$(INSTALL) -o root -g root -m 0644 systemd/t2-drop.service $(SYSTEM_UNIT_DIR)/t2-drop.service
	$(INSTALL) -o root -g root -m 0644 dbus/org.t2drop.conf $(DBUS_POLICY_DIR)/org.t2drop.conf
	$(INSTALL) -o root -g root -m 0644 dbus/org.t2drop.service $(DBUS_SERVICE_DIR)/org.t2drop.service
	$(INSTALL) -d -o root -g root -m 0755 $(NM_CONF_DIR)
	$(INSTALL) -o root -g root -m 0644 networkmanager/90-t2-drop-awdl.conf $(NM_CONF_DIR)/90-t2-drop-awdl.conf
	-nmcli general reload conf
	systemctl daemon-reload
	-busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig
	@# A running service keeps the old binary until it exits, stop it now
	-systemctl stop t2-drop.service

uninstall:
	-systemctl stop t2-drop.service
	rm -f $(BIN_DIR)/t2drop $(SYSTEM_UNIT_DIR)/t2-drop.service \
		$(DBUS_POLICY_DIR)/org.t2drop.conf $(DBUS_SERVICE_DIR)/org.t2drop.service \
		$(NM_CONF_DIR)/90-t2-drop-awdl.conf
	systemctl daemon-reload
	-busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig

clean:
	$(CARGO) clean

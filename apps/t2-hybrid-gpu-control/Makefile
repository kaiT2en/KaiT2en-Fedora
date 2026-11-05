CARGO ?= cargo
INSTALL ?= install
GTK_UPDATE_ICON_CACHE ?= gtk-update-icon-cache

APP_ID := org.t2hybridgpucontrol.gtk
BIN_NAME := t2-hybrid-gpu-control
BUILD_DIR := $(or $(CARGO_TARGET_DIR),target)/release
BIN_SRC := $(BUILD_DIR)/$(BIN_NAME)
HELPER_SRC := contrib/t2-hybrid-gpu-control-helper
HELPER_NAME := t2-hybrid-gpu-control-helper
STATUS_HELPER_SRC := contrib/t2-hybrid-gpu-control-status
STATUS_HELPER_NAME := t2-hybrid-gpu-control-status
POLICY_SRC := contrib/org.t2hybridgpucontrol.policy
STATUS_POLICY_SRC := contrib/org.t2hybridgpucontrol.status.policy
LEGACY_UNITS := kait2en-dgpu-off.service kait2en-dgpu-suspend.service \
	kait2en-amdgpu-profile.service kait2en-amdgpu-profile-resume.service
ICON_SRC := assets/icons/hicolor/scalable/apps/$(APP_ID).svg
PREBOOT_TARGET := x86_64-unknown-uefi
PREBOOT_SRC := $(or $(CARGO_TARGET_DIR),preboot/target)/$(PREBOOT_TARGET)/release/t2-gpu-preboot.efi
PREBOOT_DIR ?= /boot/efi/EFI/kait2en

PREFIX ?= /usr/local
BIN_DIR := $(PREFIX)/bin
LIBEXEC_DIR := $(PREFIX)/libexec
APPS_DIR := $(PREFIX)/share/applications
ICONS_DIR := $(PREFIX)/share/icons/hicolor/scalable/apps
ICON_THEME_DIR := $(PREFIX)/share/icons/hicolor
POLICY_DIR ?= /usr/share/polkit-1/actions
ENV_SRC := contrib/90-kait2en-gsk-renderer.conf
ENV_DIR ?= /etc/environment.d
ENV_FILE := $(ENV_DIR)/90-kait2en-gsk-renderer.conf
FLATPAK_OVERRIDES := /var/lib/flatpak/overrides/global

.PHONY: build check-build install uninstall clean

define remove_legacy_units
	-systemctl disable --now $(LEGACY_UNITS)
	$(RM) $(foreach unit,$(LEGACY_UNITS),/etc/systemd/system/$(unit))
	$(RM) $(foreach unit,$(LEGACY_UNITS),/usr/local/lib/systemd/system/$(unit))
	$(RM) $(foreach unit,$(LEGACY_UNITS),/usr/lib/systemd/system/$(unit))
	$(RM) -r /run/t2-dgpu-control /run/t2-hybrid-gpu-control
	systemctl daemon-reload
endef

build:
	$(CARGO) build --release --locked
	$(CARGO) build --release --locked --manifest-path preboot/Cargo.toml --target $(PREBOOT_TARGET)

check-build:
	@test -x "$(BIN_SRC)" || { echo "error: $(BIN_SRC) is missing; run 'make build' as your normal user first" >&2; exit 1; }
	@test -f "$(PREBOOT_SRC)" || { echo "error: $(PREBOOT_SRC) is missing; run 'make build' as your normal user first" >&2; exit 1; }

install: check-build
	$(remove_legacy_units)
	$(INSTALL) -d -o root -g root -m 0755 $(BIN_DIR) $(LIBEXEC_DIR) $(APPS_DIR) $(ICONS_DIR) $(POLICY_DIR) $(ENV_DIR)
	$(INSTALL) -o root -g root -m 0755 $(BIN_SRC) $(BIN_DIR)/$(BIN_NAME)
	$(INSTALL) -o root -g root -m 0755 $(HELPER_SRC) $(LIBEXEC_DIR)/$(HELPER_NAME)
	$(INSTALL) -o root -g root -m 0755 $(STATUS_HELPER_SRC) $(LIBEXEC_DIR)/$(STATUS_HELPER_NAME)
	$(INSTALL) -o root -g root -m 0644 $(APP_ID).desktop $(APPS_DIR)/$(APP_ID).desktop
	$(INSTALL) -o root -g root -m 0644 $(ICON_SRC) $(ICONS_DIR)/$(APP_ID).svg
	$(INSTALL) -o root -g root -m 0644 $(POLICY_SRC) $(POLICY_DIR)/$(APP_ID).policy
	$(INSTALL) -o root -g root -m 0644 $(STATUS_POLICY_SRC) $(POLICY_DIR)/$(APP_ID).status.policy
	$(INSTALL) -o root -g root -m 0644 $(ENV_SRC) $(ENV_FILE)
	if command -v flatpak >/dev/null; then flatpak override --system --env=GSK_RENDERER=ngl; fi
	$(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR)
	systemctl daemon-reload
	# vfat rejects chmod, so plain copies instead of install.
	mkdir -p $(PREBOOT_DIR)
	cp $(PREBOOT_SRC) $(PREBOOT_DIR)/t2-gpu-preboot.efi
	-$(LIBEXEC_DIR)/$(HELPER_NAME) sync-preboot

uninstall:
	$(remove_legacy_units)
	if [ -x $(LIBEXEC_DIR)/$(HELPER_NAME) ]; then $(LIBEXEC_DIR)/$(HELPER_NAME) remove-preboot; fi
	rm -f $(BIN_DIR)/$(BIN_NAME) $(LIBEXEC_DIR)/$(HELPER_NAME) $(LIBEXEC_DIR)/$(STATUS_HELPER_NAME)
	rm -f $(APPS_DIR)/$(APP_ID).desktop $(ICONS_DIR)/$(APP_ID).svg
	rm -f $(POLICY_DIR)/$(APP_ID).policy $(POLICY_DIR)/$(APP_ID).status.policy
	rm -f $(ENV_FILE)
	if [ -f $(FLATPAK_OVERRIDES) ]; then sed -i '/^GSK_RENDERER=ngl$$/d' $(FLATPAK_OVERRIDES); fi
	@if [ -d "$(ICON_THEME_DIR)" ]; then $(GTK_UPDATE_ICON_CACHE) --force --ignore-theme-index $(ICON_THEME_DIR); fi
	systemctl daemon-reload

clean:
	$(CARGO) clean
	$(CARGO) clean --manifest-path preboot/Cargo.toml

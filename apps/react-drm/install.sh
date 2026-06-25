#!/usr/bin/env bash
#
# System integration of react-drm Touch Bar
#
# Author: André Eikmeyer (dev@deqrocks)
# Date: 2026-06-14
#
# This script is provided without warranty. Use it at your own risk.
# The author and project contributors are not responsible for data loss,
# hardware damage, system failure, or any other consequences of its use.

set -Eeuo pipefail
shopt -s nullglob

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || printf '%s' "$SCRIPT_DIR")"

readonly TOUCHBAR_VENDOR_ID="05ac"
readonly TOUCHBAR_PRODUCT_ID="8302"
readonly REQUIRED_TINY_DAEMONS=(tiny-dfr mac-touchbar-plus)
# Each entry is a '|'-separated group; the group is satisfied when at least one
# of its alternatives is loadable. The Touch Bar DRM driver ships under either
# name depending on the kernel/fork (appletbdrm upstream, t2bdrm on some forks).
readonly REQUIRED_KERNEL_MODULES=('appletbdrm|t2bdrm' hid-appletb-bl)
readonly COMMON_RUNTIME_PACKAGES=(brightnessctl cava)

ANALYSIS_MISSING_COMMANDS=()
ANALYSIS_MISSING_MODULES=()
ANALYSIS_CONFLICTING_PACKAGES=()
ANALYSIS_CONFLICTING_PROCESSES=()
ANALYSIS_CONFLICTING_UNITS=()
ANALYSIS_TOUCHBAR_USB_DEVICES=()
ANALYSIS_TOUCHBAR_DRM_CARDS=()
ANALYSIS_MISSING_USER_GROUPS=()
ANALYSIS_FEDORA_REPLACED_PACKAGES=()

OS_ID=""
OS_ID_LIKE=""
OS_VERSION_ID=""
OS_PRETTY_NAME=""
OS_SIGNATURE=""
PKG_MANAGER=""
DISTRO_FAMILY=""

SESSION_TYPE=""
CURRENT_DESKTOP=""
SESSION_DESKTOP=""
WINDOW_BACKEND=""
DESKTOP_SUPPORTED=1
DESKTOP_ABORT_REASON=""
NIXOS_DETECTED=0
UBUNTU_BASED=0
NEEDS_RELOGIN=0
DEPLOYMENT_MODE="new installation"

NEEDED_BUILD_PACKAGES=()
NEEDED_RUNTIME_PACKAGES=()
NEEDED_BACKEND_PACKAGES=()
NEEDED_PACKAGES=()
LOG_PHASE="install"

info(){ printf '[%s] %s\n' "$LOG_PHASE" "$*"; }
warn(){ printf '[%s] warning: %s\n' "$LOG_PHASE" "$*" >&2; }
fail(){ printf '[%s] error: %s\n' "$LOG_PHASE" "$1" >&2; exit 1; }
trap 'printf "[%s] fatal: line %s: %s\n" "$LOG_PHASE" "$LINENO" "$BASH_COMMAND" >&2; exit 1' ERR

analysis_section() {
  printf '\n[%s] %s\n' "$LOG_PHASE" "$1"
  printf '[%s] %s\n' "$LOG_PHASE" '------------------------------------------------------------'
}

analysis_value() {
  printf '[%s]   %-24s %s\n' "$LOG_PHASE" "$1:" "$2"
}

confirm_installation() {
  local answer
  [[ $EUID -ne 0 ]] || fail "run this installer as your regular user, not as root"
  cat <<'EOF'
react-drm replaces the existing Touch Bar interface.

The installation has three phases:

  1. Analysis
     Detect the distribution, desktop session, Touch Bar hardware, kernel
     modules and conflicting daemons. Dry-run the complete package transaction
     without changing the system. Verify that react-drm can be installed and
     that detected conflicting daemons can be removed. The results are shown at
     the end of this phase.

  2. Purge
     There will be an explicit confirmation prompt before removing any of your
     current services or daemons.
     It will stop, disable and remove tiny-dfr or mac-touchbar-plus if present.
     The Touch Bar will use its firmware interface until react-drm is deployed.

  3. Deploy
     Install build and runtime dependencies, build the current local react-drm
     source, install or update its udev rules and systemd user service, and add
     your user to the video and input groups if required. If group memberships
     change, you must log out and back in after installation.

The package manager refreshes repository metadata before installing packages.
On Arch-based systems, pacman performs a full system upgrade because partial
upgrades are unsupported.

This installer does not download react-drm source updates. To update an
existing installation, update the local react-drm source using the same method
used to obtain it, then run install.sh from the updated source. The current
local source is rebuilt and the running service is restarted.

The Touch Bar may show only its firmware controls between the purge and a
successful deployment. Purge starts only after analysis and package resolution
have completed successfully. The installer stops without removing a Touch Bar
daemon if either check fails.

If the analysis produces incorrect results or the installer behaves
unexpectedly, stop the installation and report the problem at:
https://github.com/dev-muhammad-adel/react-drm-for-touchbar/issues

This installer is provided without warranty and is used entirely at your own
risk. The author and project contributors are not responsible for data loss,
hardware damage, system failure, or any other consequences of its use.
EOF
  while true; do
    printf '\nType yes to continue, or no to cancel. Press Ctrl+C to abort: '
    IFS= read -r answer || fail "installation cancelled"
    case "$answer" in
      yes) break ;;
      no) fail "installation cancelled" ;;
      *) warn "please type yes or no" ;;
    esac
  done
  command -v sudo >/dev/null 2>&1 || fail "sudo is required"
  info "Acquiring administrative privileges"
  sudo -v || fail "unable to acquire administrative privileges"
}

confirm_purge() {
  local answer
  LOG_PHASE=purge
  if [[ ${#ANALYSIS_CONFLICTING_UNITS[@]} -eq 0 &&
        ${#ANALYSIS_CONFLICTING_PACKAGES[@]} -eq 0 &&
        ${#ANALYSIS_CONFLICTING_PROCESSES[@]} -eq 0 ]]; then
    cat <<'EOF'

Analysis and package resolution completed successfully.

No conflicting Touch Bar daemon was detected, so the purge phase will not
remove anything. The next phase installs dependencies and deploys react-drm.
EOF
    while true; do
      printf '\nType CONTINUE to start deployment, or no to cancel: '
      IFS= read -r answer || fail "installation cancelled before deployment"
      case "$answer" in
        CONTINUE) return ;;
        no) fail "installation cancelled before deployment" ;;
        *) warn "please type CONTINUE or no" ;;
      esac
    done
  fi

  cat <<'EOF'

Analysis and package resolution completed successfully.

The next phase stops, disables and removes every detected tiny-dfr or
mac-touchbar-plus installation. Package removal is destructive and this
installer cannot automatically restore the previous Touch Bar setup.

Review the analysis summary above before continuing.
EOF
  while true; do
    printf '\nType PURGE to remove conflicting Touch Bar daemons, or no to cancel: '
    IFS= read -r answer || fail "installation cancelled before purge"
    case "$answer" in
      PURGE) return ;;
      no) fail "installation cancelled before purge" ;;
      *) warn "please type PURGE or no" ;;
    esac
  done
}

source_os_release() {
  [[ -r /etc/os-release ]] || { fail "/etc/os-release is missing"; return; }
  # shellcheck disable=SC1091
  . /etc/os-release
  OS_ID=${ID:-}
  OS_ID_LIKE=${ID_LIKE:-}
  OS_VERSION_ID=${VERSION_ID:-}
  OS_PRETTY_NAME=${PRETTY_NAME:-${NAME:-unknown}}
  OS_SIGNATURE="${OS_ID} ${OS_ID_LIKE} ${OS_PRETTY_NAME}"
  OS_SIGNATURE="${OS_SIGNATURE,,}"
  if [[ "$OS_ID" == ubuntu || " ${OS_ID_LIKE,,} " == *" ubuntu "* ]]; then
    UBUNTU_BASED=1
  fi
}

detect_pkg_manager() {
  case "$OS_SIGNATURE" in
    *nixos*) PKG_MANAGER=nix; DISTRO_FAMILY=nix; NIXOS_DETECTED=1 ;;
    *fedora*) PKG_MANAGER=dnf; DISTRO_FAMILY=fedora ;;
    *debian*|*ubuntu*|*kubuntu*|*linuxmint*|*pop*|*elementary*) PKG_MANAGER=apt; DISTRO_FAMILY=debian ;;
    *arch*|*cachy*|*endeavouros*|*manjaro*) PKG_MANAGER=pacman; DISTRO_FAMILY=arch ;;
    *) DISTRO_FAMILY=unknown ;;
  esac
}

detect_session() {
  SESSION_TYPE=${XDG_SESSION_TYPE:-}
  CURRENT_DESKTOP=${XDG_CURRENT_DESKTOP:-}
  SESSION_DESKTOP=${DESKTOP_SESSION:-}
  if [[ -z "$SESSION_TYPE" && -n "${WAYLAND_DISPLAY:-}" ]]; then
    SESSION_TYPE=wayland
  elif [[ -z "$SESSION_TYPE" && -n "${DISPLAY:-}" ]]; then
    SESSION_TYPE=x11
  fi
  if [[ "$SESSION_TYPE" == x11 ]]; then
    WINDOW_BACKEND=xorg
    return
  fi
  if [[ "$SESSION_TYPE" != wayland ]]; then
    DESKTOP_SUPPORTED=0
    DESKTOP_ABORT_REASON="unable to detect an Xorg or Wayland session"
    return
  fi
  case "${CURRENT_DESKTOP,,} ${SESSION_DESKTOP,,}" in
    *gnome*) WINDOW_BACKEND=gnome ;;
    *kde*|*plasma*) WINDOW_BACKEND=plasma ;;
    *hyprland*) WINDOW_BACKEND=hyprland ;;
    *niri*) WINDOW_BACKEND=niri ;;
    *) WINDOW_BACKEND=unsupported; DESKTOP_SUPPORTED=0; DESKTOP_ABORT_REASON="no active-window backend exists for this Wayland desktop" ;;
  esac
}

detect_touchbar_hardware() {
  local dev card found=0
  for dev in /sys/bus/usb/devices/*; do
    [[ -r "$dev/idVendor" && -r "$dev/idProduct" ]] || continue
    [[ "$(cat "$dev/idVendor")" == "$TOUCHBAR_VENDOR_ID" && "$(cat "$dev/idProduct")" == "$TOUCHBAR_PRODUCT_ID" ]] || continue
    ANALYSIS_TOUCHBAR_USB_DEVICES+=("$dev"); found=1
  done
  [[ $found -eq 1 ]] || fail "Touch Bar hardware (05ac:8302) not found"
  for card in /sys/class/drm/card*; do
    [[ -e "$card/device/uevent" ]] || continue
    if grep -qi 'DRIVER=appletbdrm' "$card/device/uevent" 2>/dev/null; then
      ANALYSIS_TOUCHBAR_DRM_CARDS+=("$card")
    fi
  done
}

detect_required_commands() {
  local cmd
  for cmd in sudo systemctl systemd-analyze udevadm modinfo getent; do command -v "$cmd" >/dev/null 2>&1 || ANALYSIS_MISSING_COMMANDS+=("$cmd"); done
  case "$PKG_MANAGER" in
    dnf) command -v dnf >/dev/null 2>&1 || ANALYSIS_MISSING_COMMANDS+=("dnf") ;;
    apt)
      command -v apt-get >/dev/null 2>&1 || ANALYSIS_MISSING_COMMANDS+=("apt-get")
      command -v apt-cache >/dev/null 2>&1 || ANALYSIS_MISSING_COMMANDS+=("apt-cache")
      ;;
    pacman) command -v pacman >/dev/null 2>&1 || ANALYSIS_MISSING_COMMANDS+=("pacman") ;;
  esac
}

detect_user_groups() {
  local group
  for group in video input; do
    getent group "$group" >/dev/null || fail "required group does not exist: $group"
    if ! id -nG "$USER" | tr ' ' '\n' | grep -Fxq "$group"; then
      ANALYSIS_MISSING_USER_GROUPS+=("$group")
      NEEDS_RELOGIN=1
    elif ! id -nG | tr ' ' '\n' | grep -Fxq "$group"; then
      NEEDS_RELOGIN=1
    fi
  done
}

check_deploy_files() {
  local file
  [[ -w "$REPO_ROOT" ]] || fail "repository is not writable: $REPO_ROOT"
  for file in package.json package-lock.json system/99-react-drm.rules system/react-drm.service system/react-drm-tb-detach; do
    [[ -r "$REPO_ROOT/$file" ]] || fail "required deployment file is missing or unreadable: $file"
  done
  [[ -x "$REPO_ROOT/system/react-drm-tb-detach" ]] ||
    fail "required deployment helper is not executable: system/react-drm-tb-detach"
  udevadm verify "$REPO_ROOT/system/99-react-drm.rules" >/dev/null ||
    fail "the supplied udev rules are invalid"
  if [[ -e "$HOME/.config/systemd/user/react-drm.service" ]]; then
    DEPLOYMENT_MODE="update existing installation"
  fi
}

detect_kernel_modules() {
  local group alt found
  for group in "${REQUIRED_KERNEL_MODULES[@]}"; do
    found=0
    for alt in ${group//|/ }; do
      if modinfo "$alt" >/dev/null 2>&1; then found=1; break; fi
    done
    [[ $found -eq 1 ]] || ANALYSIS_MISSING_MODULES+=("$group")
  done
}

unit_file_exists() {
  local scope=$1 unit=$2 output status
  if [[ "$scope" == system ]]; then
    if output=$(systemctl list-unit-files --no-legend "$unit" 2>&1); then
      [[ -n "$output" ]]
      return
    else
      status=$?
    fi
  else
    if output=$(systemctl --user list-unit-files --no-legend "$unit" 2>&1); then
      [[ -n "$output" ]]
      return
    else
      status=$?
    fi
  fi
  if [[ $status -eq 1 && -z "$output" ]]; then
    return 1
  fi
  fail "unable to inspect $scope unit $unit: ${output:-exit status $status}"
}

detect_conflicts() {
  local pkg proc unit scope processes status
  ANALYSIS_CONFLICTING_PACKAGES=()
  ANALYSIS_CONFLICTING_PROCESSES=()
  ANALYSIS_CONFLICTING_UNITS=()
  for pkg in "${REQUIRED_TINY_DAEMONS[@]}"; do
    pkg_installed "$pkg" && ANALYSIS_CONFLICTING_PACKAGES+=("$pkg")
    if processes=$(pgrep -af "(^|/)${pkg}([[:space:]]|$)" 2>&1); then
      while IFS= read -r proc; do
        [[ -n "$proc" ]] && ANALYSIS_CONFLICTING_PROCESSES+=("$proc")
      done <<<"$processes"
    else
      status=$?
      [[ $status -eq 1 ]] || fail "unable to inspect running processes: ${processes:-exit status $status}"
    fi
    unit="${pkg}.service"
    for scope in system user; do
      if unit_file_exists "$scope" "$unit"; then
        ANALYSIS_CONFLICTING_UNITS+=("${scope}:${unit}")
      fi
    done
  done
}

pkg_installed() {
  case "$PKG_MANAGER" in
    dnf) rpm -q "$1" >/dev/null 2>&1 ;;
    apt) dpkg -s "$1" >/dev/null 2>&1 ;;
    pacman) pacman -Q "$1" >/dev/null 2>&1 ;;
    *) return 1 ;;
  esac
}

detect_package_sets() {
  NEEDED_RUNTIME_PACKAGES=("${COMMON_RUNTIME_PACKAGES[@]}")
  NEEDED_BACKEND_PACKAGES=()
  case "$DISTRO_FAMILY" in
    fedora) NEEDED_BUILD_PACKAGES=(nodejs22-bin nodejs22-npm-bin python3 gcc gcc-c++ make pkgconf-pkg-config systemd-devel libdrm-devel cairo-devel librsvg2-devel) ;;
    debian) NEEDED_BUILD_PACKAGES=(nodejs npm python3 g++ make pkg-config libsystemd-dev libdrm-dev libcairo2-dev librsvg2-dev) ;;
    arch) NEEDED_BUILD_PACKAGES=(nodejs npm python gcc make pkgconf systemd libdrm cairo librsvg) ;;
    nix|unknown) NEEDED_BUILD_PACKAGES=() ;;
  esac
  if [[ "$WINDOW_BACKEND" == xorg ]]; then
    case "$DISTRO_FAMILY" in
      fedora) NEEDED_BACKEND_PACKAGES=(xprop) ;;
      debian) NEEDED_BACKEND_PACKAGES=(x11-utils) ;;
      arch) NEEDED_BACKEND_PACKAGES=(xorg-xprop) ;;
    esac
  fi
  NEEDED_PACKAGES=("${NEEDED_BUILD_PACKAGES[@]}" "${NEEDED_RUNTIME_PACKAGES[@]}" "${NEEDED_BACKEND_PACKAGES[@]}")
}

detect_fedora_node_replacements() {
  local package
  ANALYSIS_FEDORA_REPLACED_PACKAGES=()
  [[ "$DISTRO_FAMILY" == fedora ]] || return 0
  for package in nodejs nodejs-libs nodejs-npm nodejs-docs nodejs-full-i18n; do
    if rpm -q "$package" >/dev/null 2>&1; then
      ANALYSIS_FEDORA_REPLACED_PACKAGES+=("$package")
    fi
  done
}

check_node_version() {
  local version
  case "$PKG_MANAGER" in
    dnf) return ;;
    apt)
      version=$(apt-cache policy nodejs | awk '/Candidate:/ { print $2; exit }')
      [[ -n "$version" && "$version" != "(none)" ]] || fail "no Node.js candidate is available"
      dpkg --compare-versions "$version" ge 20.19.0 ||
        fail "the available Node.js version ($version) is too old; react-drm requires Node.js 20.19.0 or newer"
      ;;
    pacman)
      version=$(pacman -Si nodejs 2>/dev/null | awk -F ': ' '/^Version/ { print $2; exit }')
      [[ -n "$version" ]] || fail "no Node.js candidate is available"
      ;;
  esac
}

dry_run_packages() {
  local output apt_policy status
  info "Resolving the package transaction"
  case "$PKG_MANAGER" in
    dnf)
      if [[ ${#ANALYSIS_FEDORA_REPLACED_PACKAGES[@]} -gt 0 ]]; then
        if output=$(LC_ALL=C dnf -q --assumeno do \
          --action=remove "${ANALYSIS_FEDORA_REPLACED_PACKAGES[@]}" \
          --action=install "${NEEDED_PACKAGES[@]}" 2>&1); then
          status=0
        else
          status=$?
        fi
      else
        if output=$(LC_ALL=C dnf -q --assumeno install "${NEEDED_PACKAGES[@]}" 2>&1); then
          status=0
        else
          status=$?
        fi
      fi
      if [[ $status -eq 0 ]]; then
        info "Package transaction resolved successfully"
        return
      fi
      if [[ $status -eq 1 ]] &&
         grep -Fq 'Transaction Summary:' <<<"$output" &&
         grep -Fq 'Operation aborted by the user.' <<<"$output"; then
        info "Package transaction resolved successfully"
        return
      fi
      printf '%s\n' "$output" >&2
      fail "the required package transaction cannot be resolved"
      ;;
    apt)
      if [[ $UBUNTU_BASED -eq 1 ]]; then
        apt_policy=$(apt-cache policy)
        grep -q 'c=universe' <<<"$apt_policy" ||
          fail "Ubuntu's universe repository is required; enable it with 'sudo add-apt-repository universe' and run 'sudo apt-get update'"
      fi
      apt-get --simulate install "${NEEDED_PACKAGES[@]}" >/dev/null ||
        fail "the required package transaction cannot be resolved"
      info "Package transaction resolved successfully"
      ;;
    pacman)
      pacman -Sp --needed --print-format '%n' "${NEEDED_PACKAGES[@]}" >/dev/null ||
        fail "the required package transaction cannot be resolved"
      info "Package transaction resolved successfully"
      ;;
  esac
}

print_analysis() {
  local proc

  analysis_section "Environment"
  analysis_value "Repository" "$REPO_ROOT"
  analysis_value "Operating system" "${OS_PRETTY_NAME:-unknown} (${OS_ID:-unknown})"
  analysis_value "Package manager" "${PKG_MANAGER:-unknown}"
  analysis_value "Session" "${SESSION_TYPE:-unknown} / ${CURRENT_DESKTOP:-unknown} / ${SESSION_DESKTOP:-unknown}"
  analysis_value "Window backend" "${WINDOW_BACKEND:-unknown}"

  analysis_section "Hardware"
  analysis_value "Touch Bar USB devices" "${#ANALYSIS_TOUCHBAR_USB_DEVICES[@]}"
  analysis_value "Touch Bar DRM cards" "${#ANALYSIS_TOUCHBAR_DRM_CARDS[@]}"
  analysis_value "Kernel modules" "${REQUIRED_KERNEL_MODULES[*]}"

  analysis_section "Packages"
  analysis_value "Build dependencies" "${NEEDED_BUILD_PACKAGES[*]:-none}"
  analysis_value "Runtime dependencies" "${NEEDED_RUNTIME_PACKAGES[*]:-none}"
  analysis_value "Backend dependencies" "${NEEDED_BACKEND_PACKAGES[*]:-none}"
  analysis_value "Fedora replacements" "${ANALYSIS_FEDORA_REPLACED_PACKAGES[*]:-none}"

  analysis_section "Planned Changes"
  analysis_value "Deployment mode" "$DEPLOYMENT_MODE"
  analysis_value "Source operation" "build current repository; no source download"
  analysis_value "User groups to add" "${ANALYSIS_MISSING_USER_GROUPS[*]:-none}"
  analysis_value "Packages to purge" "${ANALYSIS_CONFLICTING_PACKAGES[*]:-none}"
  analysis_value "Units to disable" "${ANALYSIS_CONFLICTING_UNITS[*]:-none}"
  analysis_value "Conflicting processes" "${#ANALYSIS_CONFLICTING_PROCESSES[@]}"

  [[ $NEEDS_RELOGIN -eq 0 ]] || warn "A logout and login will be required before react-drm can start"
  if [[ ${#ANALYSIS_CONFLICTING_PROCESSES[@]} -gt 0 ]]; then
    for proc in "${ANALYSIS_CONFLICTING_PROCESSES[@]}"; do
      printf '[%s]     %s\n' "$LOG_PHASE" "$proc"
    done
  fi
}

analyze() {
  LOG_PHASE=analysis
  [[ $EUID -ne 0 ]] || fail "run this installer as your regular user, not as root"
  source_os_release
  detect_pkg_manager
  detect_session
  if [[ $NIXOS_DETECTED -eq 1 ]]; then
    warn "NixOS is not handled by this installer. Follow the manual setup instructions."
    exit 2
  fi
  if [[ $DESKTOP_SUPPORTED -eq 0 ]]; then
    fail "$DESKTOP_ABORT_REASON; react-drm currently supports GNOME, Plasma, Hyprland and Niri on Wayland, plus Xorg"
  fi
  if [[ "$DISTRO_FAMILY" == fedora ]]; then
    [[ "$OS_VERSION_ID" =~ ^[0-9]+$ ]] || fail "unable to determine the Fedora version"
    (( OS_VERSION_ID >= 44 )) || fail "Fedora 44 or newer is required"
  fi
  [[ -n "$PKG_MANAGER" ]] || fail "unsupported distribution: ${OS_PRETTY_NAME:-unknown}"
  detect_required_commands
  [[ ${#ANALYSIS_MISSING_COMMANDS[@]} -eq 0 ]] || fail "missing required commands: ${ANALYSIS_MISSING_COMMANDS[*]}"
  detect_kernel_modules
  [[ ${#ANALYSIS_MISSING_MODULES[@]} -eq 0 ]] || fail "missing T2 kernel modules: ${ANALYSIS_MISSING_MODULES[*]}"
  check_deploy_files
  detect_user_groups
  detect_package_sets
  detect_fedora_node_replacements
  check_node_version
  detect_touchbar_hardware
  detect_conflicts
  print_analysis
  dry_run_packages
  analysis_section "Result"
  analysis_value "Analysis" "successful"
  analysis_value "Package transaction" "resolved successfully"
}

phase_purge() {
  local entry scope unit
  LOG_PHASE=purge

  if [[ ${#ANALYSIS_CONFLICTING_UNITS[@]} -eq 0 &&
        ${#ANALYSIS_CONFLICTING_PACKAGES[@]} -eq 0 &&
        ${#ANALYSIS_CONFLICTING_PROCESSES[@]} -eq 0 ]]; then
    info "No conflicting Touch Bar daemon found"
    return
  fi

  for entry in "${ANALYSIS_CONFLICTING_UNITS[@]}"; do
    scope=${entry%%:*}
    unit=${entry#*:}
    info "Disabling $scope unit $unit"
    if [[ "$scope" == system ]]; then
      sudo systemctl disable --now "$unit"
    else
      systemctl --user disable --now "$unit"
    fi
  done

  if [[ ${#ANALYSIS_CONFLICTING_PACKAGES[@]} -gt 0 ]]; then
    info "Removing packages: ${ANALYSIS_CONFLICTING_PACKAGES[*]}"
    case "$PKG_MANAGER" in
      dnf) sudo dnf remove -y "${ANALYSIS_CONFLICTING_PACKAGES[@]}" ;;
      apt) sudo apt-get purge -y "${ANALYSIS_CONFLICTING_PACKAGES[@]}" ;;
      pacman) sudo pacman -Rns --noconfirm "${ANALYSIS_CONFLICTING_PACKAGES[@]}" ;;
    esac
    sudo systemctl daemon-reload
    systemctl --user daemon-reload
  fi

  detect_conflicts
  [[ ${#ANALYSIS_CONFLICTING_PACKAGES[@]} -eq 0 ]] ||
    fail "conflicting packages remain installed: ${ANALYSIS_CONFLICTING_PACKAGES[*]}"
  [[ ${#ANALYSIS_CONFLICTING_UNITS[@]} -eq 0 ]] ||
    fail "conflicting units remain installed: ${ANALYSIS_CONFLICTING_UNITS[*]}"
  [[ ${#ANALYSIS_CONFLICTING_PROCESSES[@]} -eq 0 ]] ||
    fail "a conflicting daemon is still running; stop the manual installation and retry"

  info "Conflicting Touch Bar daemons removed"
}

systemd_escape_path() {
  local value=$1
  [[ "$value" != *$'\n'* && "$value" != *$'\r'* ]] ||
    fail "repository paths containing line breaks are not supported"
  value=${value//\\/\\x5c}
  value=${value//$'\t'/\\x09}
  value=${value// /\\x20}
  value=${value//\"/\\x22}
  value=${value//\'/\\x27}
  value=${value//%/%%}
  printf '%s' "$value"
}

install_dependencies() {
  info "Installing build and runtime dependencies"
  case "$PKG_MANAGER" in
    dnf)
      if [[ ${#ANALYSIS_FEDORA_REPLACED_PACKAGES[@]} -gt 0 ]]; then
        sudo dnf -y do \
          --action=remove "${ANALYSIS_FEDORA_REPLACED_PACKAGES[@]}" \
          --action=install "${NEEDED_PACKAGES[@]}"
      else
        sudo dnf install -y "${NEEDED_PACKAGES[@]}"
      fi
      ;;
    apt)
      sudo apt-get update
      sudo apt-get install -y "${NEEDED_PACKAGES[@]}"
      ;;
    pacman) sudo pacman -Syu --needed --noconfirm "${NEEDED_PACKAGES[@]}" ;;
  esac
  command -v node >/dev/null 2>&1 || fail "Node.js is unavailable after package installation"
  command -v npm >/dev/null 2>&1 || fail "npm is unavailable after package installation"
}

build_project() {
  info "Installing npm dependencies"
  (cd "$REPO_ROOT" && npm ci)
  info "Building react-drm and the control center"
  (cd "$REPO_ROOT/linux-touchbar-control-center" && npm run build)
}

configure_user_groups() {
  local groups
  if [[ ${#ANALYSIS_MISSING_USER_GROUPS[@]} -gt 0 ]]; then
    info "Adding $USER to groups: ${ANALYSIS_MISSING_USER_GROUPS[*]}"
    groups=$(IFS=,; printf '%s' "${ANALYSIS_MISSING_USER_GROUPS[*]}")
    sudo usermod -aG "$groups" "$USER"
    NEEDS_RELOGIN=1
  fi
}

install_udev_rules() {
  info "Installing udev rules"
  sudo install -d -o root -g root -m 0755 /etc/udev/rules.d
  sudo install -o root -g root -m 0644 "$REPO_ROOT/system/99-react-drm.rules" /etc/udev/rules.d/99-react-drm.rules
  sudo udevadm control --reload
  sudo udevadm trigger --action=add --subsystem-match=usb --subsystem-match=backlight
  sudo udevadm trigger --action=add --subsystem-match=misc --sysname-match=uinput
}

install_user_service() {
  local service_dir service_file temporary_file workdir_q start_q detach_q
  service_dir="$HOME/.config/systemd/user"
  service_file="$service_dir/react-drm.service"
  workdir_q=$(systemd_escape_path "$REPO_ROOT/linux-touchbar-control-center")
  start_q=$(systemd_escape_path "$REPO_ROOT/linux-touchbar-control-center/dist/index.js")
  detach_q=$(systemd_escape_path "$REPO_ROOT/system/react-drm-tb-detach")

  info "Installing systemd user service"
  install -d -m 0755 "$service_dir"
  temporary_file=$(mktemp --suffix=.service "$service_dir/react-drm-install.XXXXXX")
  if ! awk -v workdir="$workdir_q" -v start="$start_q" -v detach="$detach_q" '
    /^WorkingDirectory=/ { print "WorkingDirectory=" workdir; next }
    /^ExecStart=/ { print "ExecStart=node " start; next }
    /^ExecStopPost=/ { print "ExecStopPost=-" detach; next }
    { print }
  ' "$REPO_ROOT/system/react-drm.service" >"$temporary_file"; then
    rm -f "$temporary_file"
    fail "unable to generate the systemd user service"
  fi
  chmod 0644 "$temporary_file"
  if ! systemd-analyze --user verify "$temporary_file"; then
    rm -f "$temporary_file"
    fail "the generated systemd user service is invalid"
  fi
  if systemctl --user is-active --quiet react-drm.service; then
    info "Stopping the existing react-drm service"
    systemctl --user stop react-drm.service
    if systemctl --user is-active --quiet react-drm.service; then
      rm -f "$temporary_file"
      fail "the existing react-drm service did not stop"
    fi
  fi
  mv -f "$temporary_file" "$service_file"
  systemctl --user daemon-reload

  if [[ $NEEDS_RELOGIN -eq 1 ]]; then
    systemctl --user enable react-drm.service
    warn "react-drm is enabled but was not started; log out and back in to activate the new group memberships"
  else
    systemctl --user enable --now react-drm.service
    sleep 2
    systemctl --user is-active --quiet react-drm.service ||
      fail "react-drm failed to remain active; inspect it with 'journalctl --user -u react-drm.service -b'"
    info "react-drm service started"
  fi
}

phase_deploy() {
  LOG_PHASE=deploy
  info "Deployment mode: $DEPLOYMENT_MODE"
  info "Building and deploying current repository: $REPO_ROOT"
  install_dependencies
  build_project
  configure_user_groups
  install_udev_rules
  install_user_service
  info "Deployment completed successfully"
  if [[ $NEEDS_RELOGIN -eq 1 ]]; then
    warn "Log out of the desktop session and log back in to activate the video and input group memberships"
    warn "react-drm will start automatically after the next login"
  else
    info "react-drm is active; no logout is required"
  fi
}

main() {
  case "${1:-install}" in
    install) confirm_installation; analyze; confirm_purge; phase_purge; phase_deploy ;;
    analyze) analyze ;;
    purge) confirm_installation; analyze; confirm_purge; phase_purge ;;
    *) printf 'usage: %s [install|analyze|purge]\n' "${0##*/}" >&2; return 2 ;;
  esac
}

main "$@"

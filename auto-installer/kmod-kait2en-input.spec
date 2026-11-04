%global _build_id_links none
%global debug_package %{nil}

Name:           kmod-kait2en-input
Version:        0.1
Release:        2%{?dist}
Summary:        Temporary T2 Mac input drivers for the Fedora installer kernel
License:        GPL-2.0-only OR GPL-2.0-or-later
URL:            https://github.com/kait2en/KaiT2en-Fedora
Source0:        kait2en-input-%{version}.tar.gz

BuildRequires:  elfutils-libelf-devel
BuildRequires:  gcc
BuildRequires:  make
Requires:       kernel-core-uname-r = %{kernel_release}
Provides:       kernel-modules = %{kernel_release}
AutoReqProv:    no

%description
Input drivers used by Anaconda and the first Fedora boot on T2 Macs. The full
KaiT2en installer replaces this package with DKMS modules for an updated kernel.

%prep
%setup -q -n kait2en-input-%{version}

%build
make -C modules/t2bce_dma KVERSION=%{kernel_release}
make -C modules/t2bce_core KVERSION=%{kernel_release}
make -C modules/t2bce_vhci KVERSION=%{kernel_release}
make -C modules/t2touchbar KDIR=/usr/src/kernels/%{kernel_release} KVER=%{kernel_release}
make -C modules/hid_t2magicmouse KDIR=/usr/src/kernels/%{kernel_release} KVER=%{kernel_release}
make -C modules/t2_precision_trackpad KDIR=/usr/src/kernels/%{kernel_release} KVER=%{kernel_release}
make -C modules/t2_trackpad_actuator KDIR=/usr/src/kernels/%{kernel_release} KVER=%{kernel_release}

%install
install -d -m 0755 %{buildroot}/usr/lib/modules/%{kernel_release}/updates/kait2en
install -m 0644 modules/t2bce_dma/t2bce_dma.ko \
    %{buildroot}/usr/lib/modules/%{kernel_release}/updates/kait2en/
install -m 0644 modules/t2bce_core/t2bce_core.ko \
    %{buildroot}/usr/lib/modules/%{kernel_release}/updates/kait2en/
install -m 0644 modules/t2bce_vhci/t2bce_vhci.ko \
    %{buildroot}/usr/lib/modules/%{kernel_release}/updates/kait2en/
install -m 0644 modules/t2touchbar/t2hid.ko \
    %{buildroot}/usr/lib/modules/%{kernel_release}/updates/kait2en/
install -m 0644 modules/hid_t2magicmouse/hid_t2magicmouse.ko \
    %{buildroot}/usr/lib/modules/%{kernel_release}/updates/kait2en/

install -m 0644 modules/t2_precision_trackpad/t2_precision_trackpad.ko \
    %{buildroot}/usr/lib/modules/%{kernel_release}/updates/kait2en/
install -m 0644 modules/t2_trackpad_actuator/t2_trackpad_actuator.ko \
    %{buildroot}/usr/lib/modules/%{kernel_release}/updates/kait2en/

%post
/usr/sbin/depmod -a %{kernel_release} >/dev/null 2>&1 || :

%postun
/usr/sbin/depmod -a %{kernel_release} >/dev/null 2>&1 || :

%files
%license LICENSE
/usr/lib/modules/%{kernel_release}/updates/kait2en/*.ko

%changelog
* Tue Nov 03 2026 KaiT2en maintainers <maintainers@kait2en.invalid> - 0.1-2
- Include the precision trackpad and haptic actuator drivers

* Tue Jul 14 2026 KaiT2en maintainers <maintainers@kait2en.invalid> - 0.1-1
- Initial Fedora installer transition package

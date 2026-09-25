Name: t2-dsp
Version: 0.1.0
Release: 1%{?dist}
Summary: Model-specific host-side audio DSP for Apple T2 Macs
License: GPL-3.0-or-later
URL: https://github.com/kaiT2en/KaiT2en-Fedora
Source0: t2-dsp-%{version}.tar.gz
BuildArch: noarch
BuildRequires: make
BuildRequires: python3
Requires: pipewire
Requires: pipewire-pulseaudio
Requires: wireplumber >= 0.5.8
Requires: pipewire-module-filter-chain-lv2
Requires: lv2-bankstown
Requires: lsp-plugins-lv2
Requires: systemd-udev
Requires: bash
Requires: coreutils
Requires: grep
Requires: python3

%description
Host-side speaker and microphone DSP graphs, FIR data and automatic model
selection for Apple T2 Macs. Requires a compatible T2 audio driver and the
AppleT2x2/x4/x6 UCM profiles. No BridgeXPC or T2 service dependency.

%prep
%autosetup

%build
%make_build -C dsp

%check
make -C dsp test
bash -n dsp/integration/libexec/package-actions

%install
make -C dsp install PREFIX=%{_prefix} DATADIR=%{_datadir} LIBEXECDIR=%{_libexecdir} DESTDIR=%{buildroot}

%post -p /bin/bash
if ! bash %{_libexecdir}/t2-dsp/package-actions configure; then
    echo '[t2-dsp] error: lifecycle helper failed. Review DSP configuration before reboot' >&2
    echo 'configure helper failed' >> /var/log/t2-dsp-install.log || echo '[t2-dsp] error: cannot save failure report' >&2
fi
exit 0

%preun -p /bin/bash
if [ "$1" -eq 0 ]; then
    if ! bash %{_libexecdir}/t2-dsp/package-actions removed; then
        echo '[t2-dsp] error: cleanup helper failed' >&2
        echo 'cleanup helper failed' >> /var/log/t2-dsp-install.log || echo '[t2-dsp] error: cannot save report' >&2
    fi
fi
exit 0

%postun -p /bin/bash
if [ "$1" -eq 0 ]; then
    if [ -d /run/udev ] && ! udevadm control --reload-rules; then
        echo '[t2-dsp] error: udev rule reload failed. Reboot required' >&2
        echo 'udev rule reload after removal failed' >> /var/log/t2-dsp-install.log || echo '[t2-dsp] error: cannot save failure report' >&2
    fi
    echo '[t2-dsp] Reboot to finish removing DSP. Verified migration backups were cleaned. Review any reported leftovers.'
fi
exit 0

%files
%doc dsp/README.md
%license %{_datadir}/licenses/t2-dsp/
%{_datadir}/t2-dsp/
%{_datadir}/wireplumber/wireplumber.conf.d/51-t2-dsp.conf
%{_datadir}/wireplumber/scripts/t2-default-output.lua
%{_datadir}/pipewire/pipewire.conf.d/50-kait2en-quantum.conf
%{_prefix}/lib/udev/rules.d/89-t2-dsp.rules
%{_libexecdir}/t2-dsp/

%changelog
* Wed Sep 16 2026 André Eikmeyer <andre.eikmeyer@kait2en.org> - 0.1.0-1
- Package host-side DSP with runtime model selection and safe migration

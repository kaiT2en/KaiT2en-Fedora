# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
Name: t2-services-common
Version: 0.1.0
Release: 1%{?dist}
Summary: Shared Apple T2 network and suspend integration
License: GPL-3.0-or-later
URL: https://github.com/kaiT2en/KaiT2en-Fedora
Source0: t2-services-%{version}.tar.gz
BuildRequires: make
BuildRequires: python3
BuildRequires: systemd-rpm-macros
BuildArch: noarch
Requires: NetworkManager
Requires: systemd
Requires: systemd-udev
Requires: iproute
Requires: iputils
Requires: coreutils
Requires: bash
Requires: python3

%description
Shared Apple T2 network and suspend integration. This package does not require any other T2 feature package.

%prep
%autosetup -n t2-services-%{version}

%build
# No compiled code in the common runtime package.

%check
bash -n t2-services/shared/integration/libexec/t2-ncm-sleep
python3 -m unittest discover -s packaging/lifecycle -v

%install
make -C t2-services/shared install PREFIX=/usr DESTDIR=%{buildroot} SYSTEMD_UNIT_DIR=%{_unitdir} LIBEXECDIR=%{_libexecdir} UDEV_RULES_DIR=%{_udevrulesdir}

%post -p /bin/bash
if source %{_libexecdir}/t2-services/package-actions; then
    t2_run python3 %{_libexecdir}/t2-services/lifecycle.py t2-services-common migrate
    migration_status=$t2_last_status
    if [ -d /run/systemd/system ]; then
        t2_run systemctl daemon-reload
        if [ "$migration_status" -eq 0 ]; then
            if [ "$1" -eq 1 ] && ! systemctl is-enabled --quiet t2-services-suspend.service; then t2_run systemctl preset t2-services-suspend.service; fi
            # Never execute suspend/resume hooks during a package transaction.
        fi
    fi
    t2_summary
else
    echo '[t2-services] error: package reporter unavailable. Migration skipped' >&2
fi
exit 0

%preun -p /bin/bash
if [ "$1" -eq 0 ]; then
    if source %{_libexecdir}/t2-services/package-actions; then
        t2_run python3 %{_libexecdir}/t2-services/lifecycle.py t2-services-common remove
        t2_summary
    else
        echo '[t2-services] error: package reporter unavailable. Cleanup skipped' >&2
    fi
fi
exit 0

%postun -p /bin/bash
if [ -d /run/systemd/system ]; then
    if ! systemctl daemon-reload; then
        echo '[t2-services] error: systemd reload after removal failed' >&2
        echo 't2-services-common: systemd reload after removal failed' >> /var/log/t2-services-install.log || echo '[t2-services] error: cannot save report' >&2
    fi
fi
exit 0

%files
%license LICENSE
%doc t2-services/README.md
%{_libexecdir}/t2-services/t2-ncm-sleep
%{_libexecdir}/t2-services/package-actions
%{_libexecdir}/t2-services/lifecycle.py
%{_libexecdir}/t2-services/migration/
%{_unitdir}/t2-services-suspend.service
%config(noreplace) /etc/NetworkManager/conf.d/10-t2-services.conf
%{_udevrulesdir}/90-t2-services-ncm.rules

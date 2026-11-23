# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
Name: t2-journal
Version: 0.1.0
Release: 1%{?dist}
Summary: Apple T2 bridgeOS and Linux journal reader
License: GPL-3.0-or-later AND Apache-2.0
URL: https://github.com/kaiT2en/KaiT2en-Fedora
Source0: t2-services-%{version}.tar.gz
BuildRequires: make
BuildRequires: systemd-rpm-macros
BuildRequires: cargo
BuildRequires: rust
BuildRequires: gcc
Requires: t2-services-common >= 0.1.0
Requires: systemd

%description
Apple T2 bridgeOS and Linux journal reader. This package does not require any other T2 feature package.

%prep
%autosetup -n t2-services-%{version}

%build
make -C t2-services/t2-journal build CARGO_FLAGS='--frozen'

%check
make -C t2-services/t2-journal test CARGO_FLAGS='--frozen'

%install
make -C t2-services/t2-journal install PREFIX=/usr DESTDIR=%{buildroot}

%post -p /bin/bash
if source %{_libexecdir}/t2-services/package-actions; then
    t2_run python3 %{_libexecdir}/t2-services/lifecycle.py t2-journal migrate
    migration_status=$t2_last_status
    t2_summary
else
    echo '[t2-services] error: package reporter unavailable. Migration skipped' >&2
fi
exit 0

%preun -p /bin/bash
if [ "$1" -eq 0 ]; then
    if source %{_libexecdir}/t2-services/package-actions; then
        t2_run python3 %{_libexecdir}/t2-services/lifecycle.py t2-journal remove
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
        echo 't2-journal: systemd reload after removal failed' >> /var/log/t2-services-install.log || echo '[t2-services] error: cannot save report' >&2
    fi
fi
exit 0

%files
%license LICENSE
%doc t2-services/README.md
%{_bindir}/t2journal

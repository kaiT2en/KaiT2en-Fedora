Name: t2-ave
Version: 0.1.0
Release: 1%{?dist}
Summary: Apple T2 AVE service
License: GPL-3.0-or-later
URL: https://github.com/kaiT2en/KaiT2en-Fedora
Source0: t2-services-%{version}.tar.gz
BuildRequires: make
BuildRequires: systemd-rpm-macros
BuildRequires: cargo
BuildRequires: rust
BuildRequires: gcc
Requires: t2-services-common >= 0.1.0
Requires: kmod

%description
Apple T2 AVE service. This package does not require any other T2 feature package.
Compatible t2-ave and t2-sidecar kernel modules must be supplied by the
distribution.

%prep
%autosetup -n t2-services-%{version}

%build
make -C t2-services/t2-ave build CARGO_FLAGS='--frozen'

%check
make -C t2-services/t2-ave test CARGO_FLAGS='--frozen'

%install
make -C t2-services/t2-ave install PREFIX=/usr DESTDIR=%{buildroot} SYSTEMD_UNIT_DIR=%{_unitdir} LIBEXECDIR=%{_libexecdir}

%post -p /bin/bash
if source %{_libexecdir}/t2-services/package-actions; then
    t2_run python3 %{_libexecdir}/t2-services/lifecycle.py t2-ave migrate
    migration_status=$t2_last_status
    if [ -d /run/systemd/system ]; then
        t2_run systemctl daemon-reload
        if [ "$migration_status" -eq 0 ]; then
            if [ "$1" -eq 1 ] && ! systemctl is-enabled --quiet kait2en-t2-remote.service; then t2_run systemctl preset kait2en-t2-remote.service; fi
            t2_run systemctl try-restart kait2en-t2-remote.service
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
        t2_run python3 %{_libexecdir}/t2-services/lifecycle.py t2-ave remove
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
        echo 't2-ave: systemd reload after removal failed' >> /var/log/t2-services-install.log || echo '[t2-services] error: cannot save report' >&2
    fi
fi
exit 0

%files
%license LICENSE
%doc t2-services/README.md
%{_bindir}/t2remote
%{_unitdir}/kait2en-t2-remote.service
%{_libexecdir}/t2-services/sleep.d/t2-ave

PREFIX ?= /usr
DATADIR ?= $(PREFIX)/share
LIBEXECDIR ?= $(PREFIX)/libexec
UDEVDIR ?= $(PREFIX)/lib/udev/rules.d
PYTHON ?= python3

.PHONY: all build test install clean
all: build
build:
	$(PYTHON) tools/build.py --datadir '$(DATADIR)'
test:
	$(PYTHON) -m unittest discover -s tests -v
	$(PYTHON) -m unittest discover -s ../packaging/lifecycle -v
install: build
	install -d '$(DESTDIR)$(DATADIR)/t2-dsp/profiles'
	cp -r build/profiles/. '$(DESTDIR)$(DATADIR)/t2-dsp/profiles/'
	find '$(DESTDIR)$(DATADIR)/t2-dsp/profiles' -type d -exec chmod 0755 {} +
	find '$(DESTDIR)$(DATADIR)/t2-dsp/profiles' -type f -exec chmod 0644 {} +
	install -D -m 0644 build/51-t2-dsp.conf '$(DESTDIR)$(DATADIR)/wireplumber/wireplumber.conf.d/51-t2-dsp.conf'
	install -D -m 0644 integration/wireplumber/t2-default-output.lua '$(DESTDIR)$(DATADIR)/wireplumber/scripts/t2-default-output.lua'
	install -D -m 0644 build/89-t2-dsp.rules '$(DESTDIR)$(UDEVDIR)/89-t2-dsp.rules'
	install -D -m 0644 integration/pipewire/50-kait2en-quantum.conf '$(DESTDIR)$(DATADIR)/pipewire/pipewire.conf.d/50-kait2en-quantum.conf'
	install -D -m 0755 integration/libexec/package-actions '$(DESTDIR)$(LIBEXECDIR)/t2-dsp/package-actions'
	install -D -m 0755 ../packaging/lifecycle/kait2en-lifecycle.py '$(DESTDIR)$(LIBEXECDIR)/t2-dsp/lifecycle.py'
	install -D -m 0644 ../LICENSE '$(DESTDIR)$(DATADIR)/licenses/t2-dsp/LICENSE'
	install -m 0644 ../LICENSING.md ../LICENSES/GPL-3.0-or-later.txt '$(DESTDIR)$(DATADIR)/licenses/t2-dsp/'
clean:
	$(PYTHON) -c 'from pathlib import Path; import shutil; p = Path("build"); shutil.rmtree(p) if p.exists() else None'

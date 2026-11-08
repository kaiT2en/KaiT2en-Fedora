# KaiT2en Update

<img class="app-shot" src="img/apps/kait2en-update.webp" alt="KaiT2en Update listing the available updates">

Experimental. `kait2en-install` stays the recommended way to update, see
[Updating](../updating.md).

Compares your installation with GitHub and installs only what changed. A full
run of the installer builds every app, every module and the initramfs again.
Most updates only touch one or two of them.

- Shows the local checkout, the state on GitHub and the new commits.
- Lists the installer steps and apps whose sources changed, each with a
  switch and the files that caused it. Unselected steps are offered again on
  the next check.
- Shows a restart hint when kernel modules or the initramfs were updated.
- Offers to install everything when the installer itself changed.
- Adds or removes the optional parts, T2 Touch Bar, Automatic brightness and
  KaiT2en Vornehm. A switch only marks the change, Update carries it out.

Each step records the commit it was installed from in
`/var/lib/kait2en-installer/applied/`. Installations from before the app have
no records, so the first check compares with the local checkout. Changes to
the website or the docs install nothing.

The installation runs through a restricted helper that asks for your password
and only accepts the names of known installer steps and apps.

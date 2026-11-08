# T2 Kernel Builder

<img class="app-shot" src="img/apps/t2-kernel-builder.webp" alt="T2 Kernel Builder on the kernel source step">

Builds Fedora kernels with the T2 configuration and your own patches.

1. **Base configuration**: choose **Use Fedora default configuration** or
   **Run localmodconfig**. localmodconfig keeps only the modules loaded right
   now, so hardware that is unplugged during the build can lose its driver.
2. **Optional components**: the kernel's Kconfig tree. The T2 requirements are
   locked, other options can be changed.
3. Choose the patch folder with **Choose folder**. Patches are applied in
   alphabetical order, at least one is required.
4. **Build kernel** downloads the Fedora kernel source RPM and builds without
   root. By default all CPU threads but one are used.
5. **Install built kernel** installs the finished build and asks for your
   password. The kernel then appears in GRUB.

The Cleanup tab removes installed custom kernels and cached sources. The
running kernel and rescue images are protected.

Sources and build trees are kept in `~/.cache/t2-kernel-builder/build`.
Builds that finished before the app was closed are offered again under the
completed builds.

### This is not easy to explain

This one is a bit of a read, but without the context the rest is hard to
follow. Skip it if you want.

The Linux mainline kernel still lacks drivers for certain T2 hardware
components. Writing them is hard because Apple hardware is "thought
different". There are not many developers who still deal with T1/T2
hardware on Linux. Many moved on to Apple Silicon for understandable reasons.

Usually someone writes a driver, a community tests it and it gets submitted
to upstream maintainers. After that the driver would live in the Linux
kernel. And that means you can just install vanilla Fedora, Ubuntu, Arch or
whatever and the kernel will support your hardware natively.

Some T2 drivers indeed went that way. Some are still missing. And these are the most
complex drivers. Their authors knew the code and its architecture still had
issues, and unconventional hardware needs a lot of explaining upstream. So
they tried and failed, or never tried.

### Going upstream = moving against the flow

Traditional distros like CachyOS or T2Linux work around this by shipping patched
kernels. The patches are not created by the distro people. T2Linux collects
the code we talked earlier about and maintains it in a GitHub repo.
Also KAIT2EN provides some patches to that repo. Like t2bce and audio drivers.
Then these patches are compiled against the current kernel. This means they
integrate the drivers into the latest release. Distros like CachyOS or Omarchy make
use of the same T2Linux patches and also integrate them in their kernels.

Should be all good, right? You just choose your favorite distro and done.
The problem is that most devs stopped upstreaming their patches to the mainline
Linux kernel. Instead they submit their patches to the T2Linux patches repo.

This has some downsides.

- T2Linux would need to maintain the patches forever.
- You will lose the safety of the standard distribution kernel.
- Kernel updates are delayed.
- You are not talking to the actual developer when you file an issue.
- When T2Linux stops maintaining, T2 Macs will stop working on Linux.

As the provider of some of these drivers, we know that getting updates into
external distros takes time. Third-party maintainers lack T2-specific context,
which makes regressions more likely, and breaking changes are hard to
communicate down the chain.

With KAIT2EN we jump over the middlemen. It is a unified platform. We take care
that it works on your Mac. So we know your hardware, your command line, your
drivers, your systemd units, your udev rules, etc. Users can test, talk to devs
directly and we can submit to upstream. Third-party maintainers can choose to
package our code or just profit from upstreamed patches. Devs can directly PR
to us.

### Stream me up, Linus!

So KAIT2EN is specialized exclusively for T2 Macs. We deliver T2 drivers and
dedicated T2 utilities. For you this is like cherry-picking: standard upstream
kernels directly from Fedora, combined with immediate hardware fixes straight
from us. Yummy!

Out-of-tree modules let us test and iterate without kernel recompilations, so
fixes roll out in minutes. Literally. The goal is still to upstream every
driver and drop it here. When we are done, T2 people install Linux from
official sources like everyone else, and no specialized distro is needed.

### Yes, we know there is Apple Silicon

But someone needs to close the gap! We truly believe T2 MacBooks can make
perfect Linux laptops. Once everything is properly fixed, models like the
MacBook Pro 15,1 or MacBook Air 9,1 run cool, offer great battery life, and cost
very little. All while keeping Apple’s exceptional build quality, Retina
displays, and Touch Bar. These two models run perfectly on KAIT2EN. The
MacBookPro16,1 and MacBookPro16,4 join them with complete hybrid graphics and
suspend support, and the 15,2 also runs well. The 15,3 and the 15,1 Vega
models lack hybrid graphics for now, for want of testers.

So this is x86 architecture and we won't get anywhere near to what Asahi with
Apple Silicon can do. But the message is not to buy into T2 Macs. It's about
making them usable and using them sustainably. If you already own a T2 Mac, you will
appreciate it. Because you know and we know that this era of devices was always
kinda "meh!". Even at the time. But on Linux they are great. Even the
"portable egg fryer" MacBook Air 9,1 is.

And actually, before Apple began with their security chip shenanigans, Apple
computers have always been great for Linux. Let's put butterfly keyboards,
flexgate, staingate and whatnotgate aside now. They are still sexy. Aren't
they? And if you can get hold of one for cheap for study or travel, you will
appreciate when you just can run Linux on it natively. Without the need of
reading half the Internet about how to make it work. Somehow.

### Is the grass greener on the KAIT2EN side?

Our grass is KAIT2EN red. There is a lot of discussion and arguing involved
when you want to get things moving. It's the sound of grinding gears while
trying to find solutions for everyone. We move fast, and our frequent update
cycle might feel relentless and annoying. Staying informed means following
announcements in our Discord community or checking GitHub for updates. We
wouldn't recommend KAIT2EN to total Linux noobs. But we are surprised to see how
fast people grow with new tasks.

Updating is entirely up to you, but KAIT2EN is built for active testing, not
passive convenience. If you want to call us opinionated, then this is your
chance. We share this project because we need real-world testers to validate our
fixes on a base we know. Not updating will lead to a non-working Mac when
outdated DKMS modules stop compiling against an updated kernel that
contains new symbols.

This is something you should keep in mind before jumping in.

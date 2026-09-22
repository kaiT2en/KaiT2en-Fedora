### This Is Not Easy to Explain

This one is a bit of a read. But while writing, I discovered that non-nerds
wouldn't even understand what I'm talking about without knowing the context.
You can skip this if you want. But for many this may be valuable information.

The Linux mainline kernel still lacks drivers for certain T2 hardware
components. The thing with T2 drivers is that they require reverse engineering.
Which is particularly difficult on Apple hardware because Apple stuff is
"thought different". There are not many developers who still deal with T1/T2
hardware on Linux. Many moved on to Apple Silicon for understandable reasons.

The flow usually is that someone writes a driver by reverse engineering. In the
best case a community would test it and then it would be submitted to upstream
maintainers to check and merge. After that the driver would live in the Linux
kernel. And that means you can just install vanilla Fedora, Ubuntu, Arch or
whatever and the kernel will support your hardware natively.

Some T2 drivers indeed went that way. Some are still missing. And these are the most
complex drivers. Because of their complexity they are not easy to upstream.
The devs who wrote them knew there were still issues with the code and its
architecture. Upstreaming a driver like that would create plenty of friction and
work. That's like squeezing it in. So they tried and failed or even never tried.

Upstreaming Apple stuff is difficult. Because it's unconventional hardware.
It needs a lot of explaining why the code is like it is.

### Going Upstream = Moving Against The Flow

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

Since we are currently the provider of some of these missing drivers, we know
that getting updates into external distros takes time and can be difficult. Many
third-party maintainers also naturally lack deep T2-specific context, making
regressions more likely. The informational flow or communication can be
difficult when breaking changes are introduced. Maintainers have to deal with
incredibly complex workflows. The more you go upstream, the more complex it
gets.

With KAIT2EN we jump over the middlemen. It is a unified platform. We take care
that it works on your Mac. So we know your hardware, your command line, your
drivers, your systemd units, your udev rules, etc. Users can test, talk to devs
directly and we can submit to upstream. Third-party maintainers can choose to
package our code or just profit from upstreamed patches. Devs can directly PR
to us.

### Stream Me Up, Linus!

So KAIT2EN is specialized exclusively for T2 Macs. We deliver T2 drivers and
dedicated T2 utilities. For you this is like cherry-picking: standard upstream
kernels directly from Fedora, combined with immediate hardware fixes straight
from us. Yummy!

Because we rely on out-of-tree modules, we can test and iterate without full
kernel recompilations. This streamlined architecture lets us roll out fixes and
handle feature requests in minutes. Literally. All while working toward our
goal, which is upstreaming every driver into the official Linux kernel, while
dropping them downstream. This means, when we are done, T2 people can install
Linux from official sources just like everyone else. And specialized distros or
repos that are scattered all around the interwebs are no longer needed to
maintain the code.

### Yes, We Know There Is Apple Silicon

But someone needs to close the gap! We truly believe T2 MacBooks can make
perfect Linux laptops. Once everything is properly fixed, models like the
MacBook Pro 15,1 or MacBook Air 9,1 run cool, offer great battery life, and cost
very little. All while keeping Apple’s exceptional build quality, Retina
displays, and Touch Bar. (Mentioning these two models specifically, because they
are running perfectly on KAIT2EN. The MacBookPro16,1 and MacBookPro16,4 now join
them with complete hybrid graphics and suspend support, and the 15,2 also runs
well. The 15,3 and 15,1 vega models do not support hybrid graphics yet, due to
lack of testers.

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

### Is The Grass Greener On The KAIT2EN Side?

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

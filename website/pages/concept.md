### The best Linux laptops (for the buck)

Used T2 MacBooks offer remarkable hardware for their price: unibody
aluminium, Retina displays, the Touch Bar and unmatched build quality.
Until 2026 their Linux support was poor. Most T2 Macs share the same drivers,
so porting one meant porting a whole family of devices. That was the chance to
make them the best Linux laptops for the money.

### You run it, we fix it

That was the promise KAIT2EN started with. T2 Macs were missing drivers in
the mainline kernel, and the existing fixes reached users late, through
patched kernels maintained by third parties. We put the T2 drivers and fixes
into DKMS modules on stock Fedora and shipped them straight from the
developers. Fix by fix, regression by regression. Sometimes up to 10 updates
per day. And brave users tested.

It worked. A T2 Mac now reaches package C10 out of the box. Touch ID, hybrid
graphics, True Tone, AirDrop, DSP...you name it, we implemented or fixed it.
T2 Macs now work better than they did on macOS when these machines were new.
Fixed kernel code goes upstream. See the [patches page](patches.html).

Even the much maligned T2 chip turned out to be an ally. It gives us more
control over the hardware than the simple firmware of a generic PC. The SMC
manages the thermal budget for T2 Zone Power, and the T2 encodes video in
hardware. The bad reputation of this generation came from not knowing it.

Being a niche works in our favour too. Many of the drivers a T2 Mac needs
exist only for Apple hardware. A trackpad regression on some Dell or a
Realtek sound regression on some HP never reaches us.

### Current state

While we are submitting the patches to upstream, we are continuously polishing.
Fedora ships the kernel, KAIT2EN ships the T2 modules as DKMS packages. A fix
reaches your Mac without waiting for a new kernel, and when a Fedora kernel
update breaks something on T2 hardware, we can respond right away. Because we
don't need to build the kernel.

The goal has not changed. Once a fix lands upstream it leaves KAIT2EN, until
a T2 Mac needs nothing beyond a stock distribution.

### But is KaiT2en for you?

KAIT2EN is made for enthusiasts and developers. Everyone is welcome to join,
but you should be comfortable in a terminal. Know what you are signing up
for:

- KaiT2en is meant as "clean-room" for debugging. We expect you to use it 
  as-is. Don't expect us to support wild customization.
- KaiT2en was made for fast progress. Code it, ship it, get feedback. Not
  for user convenience.  
- Follow Discord or GitHub to know what changed.
- Stay up to date. Outdated DKMS modules can stop building against a newer
  Fedora kernel.
- When something breaks, tell us. You talk to the developers directly.

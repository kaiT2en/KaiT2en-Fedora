KDIR ?= /lib/modules/$(shell uname -r)/build
KVER ?= $(shell uname -r)
MODDEST ?= /lib/modules/$(KVER)/updates
PWD := $(shell pwd)

obj-m += t2hid.o
obj-m += t2touchbar_bl.o
obj-m += t2touchbar_kbd.o

ccflags-y += -I$(srctree)/drivers/hid

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

install: all
	install -d -o root -g root -m 0755 $(MODDEST)
	install -o root -g root -m 0644 t2hid.ko $(MODDEST)/
	install -o root -g root -m 0644 t2touchbar_bl.ko $(MODDEST)/
	install -o root -g root -m 0644 t2touchbar_kbd.ko $(MODDEST)/
	depmod -a $(KVER)

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean
	find $(PWD) -type f \( -name '*.o' -o -name '*.ko' -o -name '*.mod' -o -name '*.mod.c' -o -name '.*.cmd' -o -name 'Module.symvers' -o -name 'modules.order' \) -delete

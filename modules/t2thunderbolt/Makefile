KDIR ?= /lib/modules/$(shell uname -r)/build
PWD := $(shell pwd)
MODDIR ?= /lib/modules/$(shell uname -r)/updates

.PHONY: all clean install

all:
	$(MAKE) -C $(KDIR) M=$(PWD)/drivers/thunderbolt modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD)/drivers/thunderbolt clean
	find $(PWD)/drivers/thunderbolt -type f \( -name '*.o' -o -name '*.ko' -o -name '*.mod' -o -name '*.mod.c' -o -name '.*.cmd' -o -name 'Module.symvers' -o -name 'modules.order' \) -delete

install: all
	install -d -o root -g root -m 0755 $(MODDIR)
	install -o root -g root -m 0644 $(PWD)/drivers/thunderbolt/t2thunderbolt.ko $(MODDIR)/t2thunderbolt.ko
	depmod -a

obj-m += t2gmux.o

KDIR ?= /lib/modules/$(shell uname -r)/build
PWD := $(shell pwd)
KVER ?= $(shell uname -r)
MODDEST ?= /lib/modules/$(KVER)/updates

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean
	find $(PWD) -type f \( -name '*.o' -o -name '*.ko' -o -name '*.mod' -o -name '*.mod.c' -o -name '.*.cmd' -o -name 'Module.symvers' -o -name 'modules.order' \) -delete

install: all
	install -d -o root -g root -m 0755 $(MODDEST)
	install -o root -g root -m 0644 t2gmux.ko $(MODDEST)/
	depmod -a $(KVER)

.PHONY: all clean install

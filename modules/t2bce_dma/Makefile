modname := t2bce_dma
obj-m += $(modname).o

t2bce_dma-objs := queue.o

MY_CFLAGS += -DWITHOUT_NVME_PATCH
ccflags-y += ${MY_CFLAGS}
ccflags-y += -I$(src)/include
CC += ${MY_CFLAGS}

KVERSION := $(KERNELRELEASE)
ifeq ($(origin KERNELRELEASE), undefined)
KVERSION := $(shell uname -r)
endif

KDIR := /lib/modules/$(KVERSION)/build
MODDEST ?= /lib/modules/$(KVERSION)/updates
PWD := $(shell pwd)

.PHONY: all clean install

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean
	find $(PWD) -ignore_readdir_race -type f \( -name '*.o' -o -name '*.ko' -o -name '*.mod' -o -name '*.mod.c' -o -name '.*.cmd' -o -name 'Module.symvers' -o -name 'modules.order' \) -delete

install:
	@test -f t2bce_dma.ko || { echo "t2bce_dma.ko not found; run make first"; exit 1; }
	install -d -o root -g root -m 0755 $(MODDEST)
	install -o root -g root -m 0644 t2bce_dma.ko $(MODDEST)/t2bce_dma.ko
	depmod -a $(KVERSION)

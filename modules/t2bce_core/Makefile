modname := t2bce_core
obj-m += $(modname).o

t2bce_core-objs := t2bce_main.o mailbox.o transport.o

T2BCE_DMA_SRC ?= ../t2bce_dma
MY_CFLAGS += -DWITHOUT_NVME_PATCH
#MY_CFLAGS += -g -DDEBUG
ccflags-y += ${MY_CFLAGS}
ccflags-y += -I$(src)/include
ccflags-y += -I$(src)/$(T2BCE_DMA_SRC)/include
CC += ${MY_CFLAGS}

KVERSION := $(KERNELRELEASE)
ifeq ($(origin KERNELRELEASE), undefined)
KVERSION := $(shell uname -r)
endif

KDIR := /lib/modules/$(KVERSION)/build
MODDEST ?= /lib/modules/$(KVERSION)/updates
PWD := $(shell pwd)
T2BCE_DMA_SYMBOLS := $(abspath $(PWD)/$(T2BCE_DMA_SRC)/Module.symvers)

ifneq ($(wildcard $(T2BCE_DMA_SYMBOLS)),)
KBUILD_EXTRA_SYMBOLS += $(T2BCE_DMA_SYMBOLS)
endif

.PHONY: all clean install

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean
	find $(PWD) -ignore_readdir_race -type f \( -name '*.o' -o -name '*.ko' -o -name '*.mod' -o -name '*.mod.c' -o -name '.*.cmd' -o -name 'Module.symvers' -o -name 'modules.order' \) -delete

install:
	@test -f t2bce_core.ko || { echo "t2bce_core.ko not found; run make first"; exit 1; }
	install -d -o root -g root -m 0755 $(MODDEST)
	install -o root -g root -m 0644 t2bce_core.ko $(MODDEST)/t2bce_core.ko
	depmod -a $(KVERSION)

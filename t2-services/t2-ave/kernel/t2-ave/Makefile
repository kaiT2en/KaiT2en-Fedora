modname := t2-ave
obj-m += $(modname).o

t2-ave-objs := ave.o protocol.o encoder.o video.o

# The DKMS stack stages AVE inside the bridge tree, standalone repository builds
# use the bridge at its repository location.
T2_AVE_DIR := $(if $(src),$(abspath $(src)),$(CURDIR))
APPLE_T2_BRIDGE_SRC ?= $(if $(wildcard $(T2_AVE_DIR)/../include/linux/apple-t2-bridge.h),..,../../../../modules/apple-t2-bridge)
ccflags-y += -I$(src) -I$(src)/$(APPLE_T2_BRIDGE_SRC)/include

KVERSION := $(KERNELRELEASE)
ifeq ($(origin KERNELRELEASE), undefined)
KVERSION := $(shell uname -r)
endif

KDIR := /lib/modules/$(KVERSION)/build
MODDEST ?= /lib/modules/$(KVERSION)/updates
PWD := $(shell pwd)
APPLE_T2_BRIDGE_SYMBOLS := $(abspath $(T2_AVE_DIR)/$(APPLE_T2_BRIDGE_SRC)/Module.symvers)

ifneq ($(wildcard $(APPLE_T2_BRIDGE_SYMBOLS)),)
KBUILD_EXTRA_SYMBOLS += $(APPLE_T2_BRIDGE_SYMBOLS)
endif

.PHONY: all clean install

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean

install:
	@test -f t2-ave.ko || { echo "t2-ave.ko not found; run make first"; exit 1; }
	install -d -o root -g root -m 0755 $(MODDEST)
	install -o root -g root -m 0644 t2-ave.ko $(MODDEST)/t2-ave.ko
	depmod -a $(KVERSION)

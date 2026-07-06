modname := t2bce_audio
obj-m += $(modname).o

t2bce_audio-objs := audio.o protocol.o protocol_bce.o pcm.o

T2BCE_CORE_SRC ?= ../t2bce_core
MY_CFLAGS += -DWITHOUT_NVME_PATCH
ccflags-y += ${MY_CFLAGS}
ccflags-y += -I$(src) -I$(src)/$(T2BCE_CORE_SRC)/include
CC += ${MY_CFLAGS}

KVERSION := $(KERNELRELEASE)
ifeq ($(origin KERNELRELEASE), undefined)
KVERSION := $(shell uname -r)
endif

KDIR := /lib/modules/$(KVERSION)/build
MODDEST ?= /lib/modules/$(KVERSION)/updates
PWD := $(shell pwd)
T2BCE_CORE_SYMBOLS := $(abspath $(PWD)/$(T2BCE_CORE_SRC)/Module.symvers)

ifneq ($(wildcard $(T2BCE_CORE_SYMBOLS)),)
KBUILD_EXTRA_SYMBOLS += $(T2BCE_CORE_SYMBOLS)
endif

.PHONY: all clean install

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean

install:
	@test -f t2bce_audio.ko || { echo "t2bce_audio.ko not found; run make first"; exit 1; }
	install -d -o root -g root -m 0755 $(MODDEST)
	install -o root -g root -m 0644 t2bce_audio.ko $(MODDEST)/t2bce_audio.ko
	depmod -a $(KVERSION)

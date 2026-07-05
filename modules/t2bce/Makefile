modname := t2bce
obj-m += $(modname).o

AUDIO_SRC ?= ../t2audio
t2bce-objs := t2bce_main.o mailbox.o queue.o queue_dma.o transport.o vhci/vhci.o vhci/queue.o vhci/transfer.o $(AUDIO_SRC)/audio.o $(AUDIO_SRC)/protocol.o $(AUDIO_SRC)/protocol_bce.o $(AUDIO_SRC)/pcm.o

MY_CFLAGS += -DWITHOUT_NVME_PATCH
#MY_CFLAGS += -g -DDEBUG
ccflags-y += ${MY_CFLAGS}
ccflags-y += -I$(src)/include -I$(src)/$(AUDIO_SRC)
CC += ${MY_CFLAGS}

KVERSION := $(KERNELRELEASE)
ifeq ($(origin KERNELRELEASE), undefined)
KVERSION := $(shell uname -r)
endif

KDIR := /lib/modules/$(KVERSION)/build
MODDEST ?= /lib/modules/$(KVERSION)/updates
PWD := $(shell pwd)
AUDIO_DIR := $(abspath $(PWD)/$(AUDIO_SRC))

.PHONY: all clean install

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean
	find $(PWD) $(AUDIO_DIR) -type f \( -name '*.o' -o -name '*.ko' -o -name '*.mod' -o -name '*.mod.c' -o -name '.*.cmd' -o -name 'Module.symvers' -o -name 'modules.order' \) -delete

install:
	@test -f t2bce.ko || { echo "t2bce.ko not found; run make first"; exit 1; }
	install -d -o root -g root -m 0755 $(MODDEST)
	install -o root -g root -m 0644 t2bce.ko $(MODDEST)/t2bce.ko
	depmod -a $(KVERSION)

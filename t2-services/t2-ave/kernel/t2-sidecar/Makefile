modname := t2-sidecar
obj-m += $(modname).o

t2-sidecar-objs := usb.o session.o video.o bplist.o

# A plain USB driver for the device apple-t2-vhci-hcd exposes, no bridge symbols.
ccflags-y += -I$(src)

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

install:
	@test -f t2-sidecar.ko || { echo "t2-sidecar.ko not found; run make first"; exit 1; }
	install -d -o root -g root -m 0755 $(MODDEST)
	install -o root -g root -m 0644 t2-sidecar.ko $(MODDEST)/t2-sidecar.ko
	depmod -a $(KVERSION)

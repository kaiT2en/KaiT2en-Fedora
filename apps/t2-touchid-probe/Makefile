CC ?= cc
CFLAGS ?= -O2 -g
CFLAGS += -Wall -Wextra -Werror -pthread $(shell pkg-config --cflags libplist-2.0)
LDLIBS += $(shell pkg-config --libs libplist-2.0)

.PHONY: all clean
all: t2-touchid-probe

t2-touchid-probe: t2-touchid-probe.c
	$(CC) $(CFLAGS) -o $@ $< $(LDLIBS)

clean:
	rm -f t2-touchid-probe

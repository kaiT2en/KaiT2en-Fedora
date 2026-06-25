CC = gcc
CFLAGS = -O2
TARGET = hex2hcd
PREFIX = /usr/local


all: $(TARGET)

$(TARGET): $(TARGET).c

install: $(TARGET)
	install -d -o root -g root -m 0755 $(PREFIX)/bin
	install -o root -g root -m 0755 $(TARGET) $(PREFIX)/bin/$(TARGET)

uninstall:
	rm -f $(PREFIX)/bin/$(TARGET)

clean:
	rm -f $(TARGET)

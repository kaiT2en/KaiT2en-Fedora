// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
/* Periodically exercise the t2smc RTC/CLKL path. */

#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <fcntl.h>
#include <linux/rtc.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>

static void add_milliseconds(struct timespec *deadline, unsigned long ms)
{
	deadline->tv_sec += ms / 1000;
	deadline->tv_nsec += (long)(ms % 1000) * 1000000L;
	if (deadline->tv_nsec >= 1000000000L) {
		deadline->tv_sec++;
		deadline->tv_nsec -= 1000000000L;
	}
}

int main(int argc, char **argv)
{
	const char *device = argc > 1 ? argv[1] : "/dev/rtc0";
	unsigned long interval_ms = 1000;
	struct timespec deadline;
	char *end;
	int fd;

	if (argc > 2) {
		errno = 0;
		interval_ms = strtoul(argv[2], &end, 10);
		if (errno || *end || !interval_ms) {
			fprintf(stderr, "usage: %s [RTC_DEVICE [INTERVAL_MS]]\n",
				argv[0]);
			return EXIT_FAILURE;
		}
	}
	if (argc > 3) {
		fprintf(stderr, "usage: %s [RTC_DEVICE [INTERVAL_MS]]\n", argv[0]);
		return EXIT_FAILURE;
	}

	fd = open(device, O_RDONLY | O_CLOEXEC);
	if (fd < 0) {
		fprintf(stderr, "open %s: %s\n", device, strerror(errno));
		return EXIT_FAILURE;
	}

	if (clock_gettime(CLOCK_MONOTONIC, &deadline) < 0) {
		fprintf(stderr, "clock_gettime: %s\n", strerror(errno));
		close(fd);
		return EXIT_FAILURE;
	}

	for (;;) {
		struct rtc_time rtc;
		int ret;

		if (ioctl(fd, RTC_RD_TIME, &rtc) < 0) {
			fprintf(stderr, "RTC_RD_TIME on %s: %s\n",
				device, strerror(errno));
			close(fd);
			return EXIT_FAILURE;
		}

		printf("%04d-%02d-%02dT%02d:%02d:%02dZ\n",
		       rtc.tm_year + 1900, rtc.tm_mon + 1, rtc.tm_mday,
		       rtc.tm_hour, rtc.tm_min, rtc.tm_sec);
		fflush(stdout);

		add_milliseconds(&deadline, interval_ms);
		do {
			ret = clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME,
					      &deadline, NULL);
		} while (ret == EINTR);
		if (ret) {
			fprintf(stderr, "clock_nanosleep: %s\n", strerror(ret));
			close(fd);
			return EXIT_FAILURE;
		}
	}
}

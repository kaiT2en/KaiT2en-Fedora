// SPDX-License-Identifier: GPL-2.0-only
/*
 * BridgeXPC/BiometricKit probe for the Apple T2 Touch ID sensor. It reaches
 * bridgeOS over the CDC-NCM link, loads the sensor calibration and runs either
 * a presence scan or a verify-only match against the fingers enrolled on this
 * machine. It never enrolls and never touches /dev/t2sep; see README.md.
 */
#define _GNU_SOURCE

#include <arpa/inet.h>
#include <dirent.h>
#include <errno.h>
#include <getopt.h>
#include <ifaddrs.h>
#include <limits.h>
#include <net/if.h>
#include <netinet/in.h>
#include <plist/plist.h>
#include <poll.h>
#include <signal.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/random.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

#define BRIDGE_MAGIC 0xb892
#define BRIDGE_VERSION 1
#define BRIDGE_HELO 1
#define BRIDGE_MESSAGE 2
#define MAX_BODY (16U * 1024U * 1024U)
#define BIO_MAGIC 0x4d42
#define BIO_CANCEL 12
#define BIO_PRESENCE 0x26
#define BIO_USER_IDENTITIES 0x42
#define BIO_MATCH 0x04
#define BIO_SYSTEM_CONFIG 0x43
#define BIO_SKS_LOCK_STATE 0x27
#define BIO_LOAD_CATACOMB 0x40
#define BIO_LOAD_BIOLOCKOUT 0x4b
#define BIO_PROVISIONING_STATE 0x10
#define MATCH_INIT_SIZE 68U
#define IDENTITY_RECORD_SIZE 20U
#define MAX_IDENTITIES 10U
#define BIO_NIL_OUTPUT "d4161201-daf5-4bbd-ae4f-9bf319fabbe0"
#define EVENT_STATUS 0xe3ff8001U
#define EVENT_MATCH  0xe3ff8002U
#define EVENT_STATS  0xe3ff8004U
#define STATUS_FINGER_DOWN 63U
#define STATUS_FINGER_UP   64U
#define FIRST_DYNAMIC_PORT 49152
#define LAST_DYNAMIC_PORT 65535
#define MAX_RSD_CANDIDATES (LAST_DYNAMIC_PORT - FIRST_DYNAMIC_PORT + 1)
#define RSD_RESPONSE_MAX (1024U * 1024U)
#define SCAN_WORKERS 128
/* remoted publishes the service directory here. */
#define RSD_DIRECT_PORT 59602
/* Fixed Apple MAC ac:de:48:33:44:55 in EUI-64 form. */
#define DEFAULT_PEER "fe80::aede:48ff:fe33:4455"
#define APPLE_NCM_VENDOR "05ac"
#define APPLE_NCM_PRODUCT "8233"

static const uint8_t http2_preface[] = "PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
static const uint8_t rsd_setup_frames[] = {
	0x00,0x00,0x0c,0x04,0x00,0x00,0x00,0x00,0x00,0x00,0x03,0x00,0x00,0x00,0x64,0x00,0x04,0x01,0x00,0x00,0x00,
	0x00,0x00,0x04,0x08,0x00,0x00,0x00,0x00,0x00,0x00,0xff,0x00,0x01,
	0x00,0x00,0x00,0x01,0x04,0x00,0x00,0x00,0x01,
	0x00,0x00,0x2c,0x00,0x00,0x00,0x00,0x00,0x01,
	0x92,0x0b,0xb0,0x29,0x01,0x00,0x00,0x00,0x14,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
	0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x42,0x37,0x13,0x42,0x05,0x00,0x00,0x00,
	0x00,0xf0,0x00,0x00,0x04,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
	0x00,0x00,0x00,0x01,0x04,0x00,0x00,0x00,0x03,
	0x00,0x00,0x18,0x00,0x00,0x00,0x00,0x00,0x01,
	0x92,0x0b,0xb0,0x29,0x01,0x02,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
	0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
	0x00,0x00,0x18,0x00,0x00,0x00,0x00,0x00,0x03,
	0x92,0x0b,0xb0,0x29,0x01,0x00,0x40,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
	0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
};
static const uint8_t http2_settings_ack[] = { 0,0,0, 4,1, 0,0,0,0 };
static const uint8_t rsd_handshake_wrapper[] = {
	0x92,0x0b,0xb0,0x29,0x01,0x01,0x00,0x00,0xf0,0x00,0x00,0x00,
	0x00,0x00,0x00,0x00,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
	0x42,0x37,0x13,0x42,0x05,0x00,0x00,0x00,0x00,0xf0,0x00,0x00,
	0xe0,0x00,0x00,0x00,0x05,0x00,0x00,0x00,0x4d,0x65,0x73,0x73,
	0x61,0x67,0x65,0x54,0x79,0x70,0x65,0x00,0x00,0x90,0x00,0x00,
	0x0a,0x00,0x00,0x00,0x48,0x61,0x6e,0x64,0x73,0x68,0x61,0x6b,
	0x65,0x00,0x00,0x00,0x4d,0x65,0x73,0x73,0x61,0x67,0x69,0x6e,
	0x67,0x50,0x72,0x6f,0x74,0x6f,0x63,0x6f,0x6c,0x56,0x65,0x72,
	0x73,0x69,0x6f,0x6e,0x00,0x00,0x00,0x00,0x00,0x40,0x00,0x00,
	0x07,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x55,0x55,0x49,0x44,
	0x00,0x00,0x00,0x00,0x00,0xa0,0x00,0x00,0x00,0x11,0x22,0x33,
	0x44,0x55,0x46,0x77,0x88,0x99,0xaa,0xbb,0xcc,0xdd,0xee,0xff,
	0x50,0x72,0x6f,0x70,0x65,0x72,0x74,0x69,0x65,0x73,0x00,0x00,
	0x00,0xf0,0x00,0x00,0x4c,0x00,0x00,0x00,0x02,0x00,0x00,0x00,
	0x52,0x65,0x6d,0x6f,0x74,0x65,0x58,0x50,0x43,0x56,0x65,0x72,
	0x73,0x69,0x6f,0x6e,0x46,0x6c,0x61,0x67,0x73,0x00,0x00,0x00,
	0x00,0x40,0x00,0x00,0x06,0x00,0x00,0x00,0x00,0x00,0x00,0x01,
	0x53,0x65,0x6e,0x73,0x69,0x74,0x69,0x76,0x65,0x50,0x72,0x6f,
	0x70,0x65,0x72,0x74,0x69,0x65,0x73,0x56,0x69,0x73,0x69,0x62,
	0x6c,0x65,0x00,0x00,0x00,0x20,0x00,0x00,0x01,0x00,0x00,0x00,
	0x53,0x65,0x72,0x76,0x69,0x63,0x65,0x73,0x00,0x00,0x00,0x00,
	0x00,0xf0,0x00,0x00,0x04,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
};
_Static_assert(sizeof(rsd_setup_frames) == 171, "RemoteXPC setup fixture size");
_Static_assert(sizeof(rsd_handshake_wrapper) == 264, "RemoteXPC handshake fixture size");

static uint8_t enrolled_records[MAX_IDENTITIES * IDENTITY_RECORD_SIZE];
static size_t enrolled_records_length;
static uint8_t enrolled_uuids[MAX_IDENTITIES][16];
static unsigned enrolled_count;

struct __attribute__((packed)) bridge_header {
	uint16_t magic;
	uint16_t version;
	uint32_t type;
	uint64_t length;
};

struct __attribute__((packed)) bio_header {
	uint16_t magic;
	uint16_t command;
	uint16_t version;
	uint16_t value;
};

struct rsd_scan {
	struct in6_addr peer;
	unsigned scope;
	atomic_uint next_port;
	pthread_mutex_t lock;
	unsigned candidates[MAX_RSD_CANDIDATES];
	unsigned count;
};

static struct timespec probe_start;
static volatile sig_atomic_t stop_requested;

static void on_sigint(int signo) { (void)signo; stop_requested = 1; }

static double elapsed(void)
{
	struct timespec now;
	clock_gettime(CLOCK_MONOTONIC, &now);
	return (double)(now.tv_sec - probe_start.tv_sec) +
	       (double)(now.tv_nsec - probe_start.tv_nsec) / 1e9;
}

__attribute__((format(printf, 1, 2)))
static void event(const char *format, ...)
{
	va_list arguments;
	printf("t=%7.3f ", elapsed());
	va_start(arguments, format);
	vprintf(format, arguments);
	va_end(arguments);
	putchar('\n');
	fflush(stdout);
}

static uint16_t le16(uint16_t v) { return htole16(v); }
static uint32_t le32(uint32_t v) { return htole32(v); }
static uint64_t le64(uint64_t v) { return htole64(v); }

static int write_all(int fd, const void *buffer, size_t length)
{
	const uint8_t *p = buffer;
	while (length) {
		ssize_t done = send(fd, p, length, MSG_NOSIGNAL);
		if (done < 0) {
			if (errno == EINTR) continue;
			return -1;
		}
		p += done;
		length -= done;
	}
	return 0;
}

static int read_all(int fd, void *buffer, size_t length)
{
	uint8_t *p = buffer;
	while (length) {
		ssize_t done = recv(fd, p, length, 0);
		if (done == 0) { errno = ECONNRESET; return -1; }
		if (done < 0) {
			if (errno == EINTR) continue;
			return -1;
		}
		p += done;
		length -= done;
	}
	return 0;
}

static int receive_frame(int fd, uint32_t *type, uint8_t **body, uint32_t *length)
{
	struct bridge_header header;
	uint64_t body_length;
	if (read_all(fd, &header, sizeof(header))) return -1;
	if (le16toh(header.magic) != BRIDGE_MAGIC ||
	    le16toh(header.version) != BRIDGE_VERSION) {
		errno = EPROTO;
		return -1;
	}
	body_length = le64toh(header.length);
	if (body_length > MAX_BODY || body_length > UINT32_MAX) {
		errno = EMSGSIZE;
		return -1;
	}
	*body = malloc(body_length ? (size_t)body_length : 1);
	if (!*body) return -1;
	if (body_length && read_all(fd, *body, body_length)) {
		free(*body);
		*body = NULL;
		return -1;
	}
	*type = le32toh(header.type);
	*length = body_length;
	return 0;
}

static int send_frame(int fd, uint32_t type, const void *body, uint32_t length)
{
	struct bridge_header header = {
		.magic = le16(BRIDGE_MAGIC), .version = le16(BRIDGE_VERSION),
		.type = le32(type), .length = le64(length),
	};
	return write_all(fd, &header, sizeof(header)) ||
	       (length && write_all(fd, body, length));
}

static int send_plist(int fd, plist_t value)
{
	char *data = NULL;
	uint32_t length = 0;
	plist_err_t error = plist_to_bin(value, &data, &length);
	int ret;
	if (error != PLIST_ERR_SUCCESS) { errno = EPROTO; return -1; }
	ret = send_frame(fd, BRIDGE_MESSAGE, data, length);
	plist_mem_free(data);
	return ret;
}

static plist_t receive_plist(int fd, uint32_t expected_type)
{
	uint8_t *body = NULL;
	uint32_t type, length;
	plist_t value = NULL;
	plist_format_t format;
	if (receive_frame(fd, &type, &body, &length)) return NULL;
	if (type != expected_type ||
	    plist_from_memory((char *)body, length, &value, &format) != PLIST_ERR_SUCCESS) {
		free(body);
		errno = EPROTO;
		return NULL;
	}
	free(body);
	return value;
}

static void make_uuid(char output[37])
{
	uint8_t b[16];
	if (getrandom(b, sizeof(b), 0) != sizeof(b)) {
		perror("getrandom");
		exit(1);
	}
	b[6] = (b[6] & 0x0f) | 0x40;
	b[8] = (b[8] & 0x3f) | 0x80;
	snprintf(output, 37,
		 "%02X%02X%02X%02X-%02X%02X-%02X%02X-%02X%02X-%02X%02X%02X%02X%02X%02X",
		 b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
		 b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]);
}

static plist_t envelope(const char *reply_id, bool reply, plist_t payload)
{
	plist_t array = plist_new_array();
	plist_array_append_item(array, plist_new_uint(1));
	plist_array_append_item(array, plist_new_bool(reply));
	plist_array_append_item(array, plist_new_string(reply_id));
	plist_array_append_item(array, payload);
	return array;
}

static void describe_event(plist_t payload)
{
	plist_t item, data_node;
	char *data = NULL;
	uint64_t size = 0, method = 0;
	uint32_t event_type;
	if (!payload || plist_get_node_type(payload) != PLIST_ARRAY ||
	    plist_array_get_size(payload) != 5) {
		event("event=unknown");
		return;
	}
	item = plist_array_get_item(payload, 0);
	plist_get_uint_val(item, &method);
	data_node = plist_array_get_item(payload, 2);
	if (method != 9 || !data_node || plist_get_node_type(data_node) != PLIST_DATA) {
		event("event=unknown");
		return;
	}
	plist_get_data_val(data_node, &data, &size);
	if (!data || size < 24) {
		event("event=malformed");
		free(data);
		return;
	}
	memcpy(&event_type, data + 8, sizeof(event_type));
	event_type = le32toh(event_type);
	if (event_type == EVENT_STATUS && size >= 28) {
		uint32_t status;
		const char *name = "";
		memcpy(&status, data + 24, sizeof(status));
		status = le32toh(status);
		if (status == STATUS_FINGER_DOWN) name = " finger=down";
		else if (status == STATUS_FINGER_UP) name = " finger=up";
		event("event=status code=%u%s", status, name);
	} else if (event_type == EVENT_STATS) {
		event("event=statistics bytes=%llu",
		      (unsigned long long)(size - 24));
	} else if (event_type == EVENT_MATCH) {
		const uint8_t *body = (const uint8_t *)data + 24;
		size_t body_size = size - 24;
		int slot = -1;
		for (unsigned i = 0; i < enrolled_count && slot < 0; i++)
			for (size_t off = 0; off + 16 <= body_size; off++)
				if (!memcmp(body + off, enrolled_uuids[i], 16)) {
					slot = (int)i;
					break;
				}
		if (!enrolled_count)
			event("event=match-result bytes=%llu verdict=unknown reason=no-identities",
			      (unsigned long long)body_size);
		else if (slot >= 0)
			event("event=match-result bytes=%llu verdict=match slot=%d",
			      (unsigned long long)body_size, slot);
		else
			event("event=match-result bytes=%llu verdict=no-match",
			      (unsigned long long)body_size);
	} else {
		event("event=other type=0x%08x bytes=%llu", event_type,
		      (unsigned long long)(size - 24));
	}
	free(data);
}

static unsigned peer_bridge_version(const uint8_t *body, uint32_t length)
{
	static const char key[] = "\"BridgeXPCVersion\":";
	char *text, *position, *end;
	unsigned long value;

	text = malloc((size_t)length + 1);
	if (!text) return 0;
	memcpy(text, body, length);
	text[length] = '\0';
	position = strstr(text, key);
	if (!position) { free(text); return 0; }
	position += sizeof(key) - 1;
	value = strtoul(position, &end, 10);
	bool parsed = end != position;
	free(text);
	if (!parsed || !value || value > UINT16_MAX) return 0;
	return value;
}

static plist_t request(int fd, plist_t payload, bool show_events)
{
	char id[37];
	plist_t message;
	make_uuid(id);
	message = envelope(id, false, payload);
	if (send_plist(fd, message)) { plist_free(message); return NULL; }
	plist_free(message);
	for (;;) {
		plist_t incoming = receive_plist(fd, BRIDGE_MESSAGE);
		plist_t flag, reply_id, body;
		uint8_t is_reply = 0;
		char *received_id = NULL;
		if (!incoming) return NULL;
		if (plist_get_node_type(incoming) != PLIST_ARRAY ||
		    plist_array_get_size(incoming) != 4) {
			plist_free(incoming); errno = EPROTO; return NULL;
		}
		flag = plist_array_get_item(incoming, 1);
		reply_id = plist_array_get_item(incoming, 2);
		body = plist_array_get_item(incoming, 3);
		plist_get_bool_val(flag, &is_reply);
		plist_get_string_val(reply_id, &received_id);
		if (is_reply) {
			plist_t copy = NULL;
			if (received_id && !strcmp(received_id, id)) copy = plist_copy(body);
			free(received_id); plist_free(incoming);
			if (copy) return copy;
			errno = EPROTO; return NULL;
		}
		if (show_events) describe_event(body);
		if (!received_id) { plist_free(incoming); errno = EPROTO; return NULL; }
		message = envelope(received_id, true, plist_new_array());
		plist_array_append_item(plist_array_get_item(message, 3), plist_new_uint(0));
		free(received_id);
		if (send_plist(fd, message)) { plist_free(message); plist_free(incoming); return NULL; }
		plist_free(message); plist_free(incoming);
	}
}

static plist_t bio_request(int fd, uint16_t command, uint16_t value,
			   const void *data, size_t data_length,
			   uint32_t output_capacity)
{
	uint8_t *bytes;
	struct bio_header *header;
	plist_t payload = plist_new_array();
	plist_t reply;
	if (data_length > MAX_BODY - sizeof(*header)) { errno = EMSGSIZE; return NULL; }
	bytes = calloc(1, sizeof(*header) + data_length);
	if (!bytes) return NULL;
	header = (void *)bytes;
	header->magic = le16(BIO_MAGIC);
	header->command = le16(command);
	header->version = le16(1);
	header->value = le16(value);
	if (data_length) memcpy(bytes + sizeof(*header), data, data_length);
	plist_array_append_item(payload, plist_new_uint(3));
	plist_array_append_item(payload, plist_new_uint(0));
	plist_array_append_item(payload, plist_new_data((char *)bytes,
							 sizeof(*header) + data_length));
	plist_array_append_item(payload, plist_new_uint(output_capacity));
	reply = request(fd, payload, true);
	memset(bytes, 0, sizeof(*header) + data_length);
	free(bytes);
	return reply;
}

static bool reply_ok(plist_t reply)
{
	uint64_t status = UINT64_MAX;
	if (!reply || plist_get_node_type(reply) != PLIST_ARRAY ||
	    !plist_array_get_size(reply)) return false;
	plist_get_uint_val(plist_array_get_item(reply, 0), &status);
	return status == 0;
}

static uint64_t reply_status(plist_t reply)
{
	uint64_t status = UINT64_MAX;
	if (reply && plist_get_node_type(reply) == PLIST_ARRAY &&
	    plist_array_get_size(reply))
		plist_get_uint_val(plist_array_get_item(reply, 0), &status);
	return status;
}

static int reply_data(plist_t reply, char **data, uint64_t *length)
{
	plist_t value;
	if (!reply_ok(reply) || plist_array_get_size(reply) < 2) return -1;
	value = plist_array_get_item(reply, 1);
	if (!value || plist_get_node_type(value) != PLIST_DATA) return -1;
	plist_get_data_val(value, data, length);
	return *data ? 0 : -1;
}

static bool sysfs_value_is(const char *path, const char *expected)
{
	char buffer[64];
	size_t length;
	FILE *file = fopen(path, "r");
	if (!file) return false;
	length = fread(buffer, 1, sizeof(buffer) - 1, file);
	fclose(file);
	buffer[length] = '\0';
	while (length && (buffer[length - 1] == '\n' || buffer[length - 1] == ' '))
		buffer[--length] = '\0';
	return !strcasecmp(buffer, expected);
}

static bool is_apple_ncm(const char *name)
{
	char link[PATH_MAX], resolved[PATH_MAX];
	char candidate[PATH_MAX + 16];
	char *cut;
	snprintf(link, sizeof(link), "/sys/class/net/%s/device/driver", name);
	if (!realpath(link, resolved)) return false;
	cut = strrchr(resolved, '/');
	if (!cut || strcmp(cut + 1, "cdc_ncm")) return false;
	snprintf(link, sizeof(link), "/sys/class/net/%s/device", name);
	if (!realpath(link, resolved)) return false;
	for (;;) {
		snprintf(candidate, sizeof(candidate), "%s/idVendor", resolved);
		if (sysfs_value_is(candidate, APPLE_NCM_VENDOR)) {
			snprintf(candidate, sizeof(candidate), "%s/idProduct", resolved);
			if (sysfs_value_is(candidate, APPLE_NCM_PRODUCT)) return true;
		}
		cut = strrchr(resolved, '/');
		if (!cut || cut == resolved) return false;
		*cut = '\0';
	}
}

static int find_interface(char *out, size_t size)
{
	struct dirent *entry;
	unsigned found = 0;
	DIR *dir = opendir("/sys/class/net");
	if (!dir) { perror("/sys/class/net"); return -1; }
	while ((entry = readdir(dir))) {
		if (entry->d_name[0] == '.') continue;
		if (strlen(entry->d_name) >= size) continue;
		if (!is_apple_ncm(entry->d_name)) continue;
		if (!found++) memcpy(out, entry->d_name, strlen(entry->d_name) + 1);
	}
	closedir(dir);
	if (!found) {
		fprintf(stderr, "no Apple %s:%s CDC-NCM interface found; use -I\n",
			APPLE_NCM_VENDOR, APPLE_NCM_PRODUCT);
		return -1;
	}
	if (found > 1) {
		fprintf(stderr, "several Apple CDC-NCM interfaces found; use -I\n");
		return -1;
	}
	return 0;
}

static bool interface_has_link_local(const char *interface)
{
	char line[256], name[64];
	bool found = false;
	FILE *file = fopen("/proc/net/if_inet6", "r");
	if (!file) return true;
	while (fgets(line, sizeof(line), file)) {
		unsigned index, prefix, scope, flags;
		if (sscanf(line, "%*32s %x %x %x %x %63s",
			   &index, &prefix, &scope, &flags, name) != 5)
			continue;
		if (scope == 0x20 && !strcmp(name, interface)) { found = true; break; }
	}
	fclose(file);
	return found;
}

static int connect_peer(const char *host, const char *interface, unsigned port)
{
	struct sockaddr_in6 address = { .sin6_family = AF_INET6,
		.sin6_port = htons(port), .sin6_scope_id = if_nametoindex(interface) };
	int fd;
	if (!address.sin6_scope_id || inet_pton(AF_INET6, host, &address.sin6_addr) != 1) {
		errno = EINVAL; return -1;
	}
	fd = socket(AF_INET6, SOCK_STREAM, 0);
	if (fd < 0) return -1;
	if (connect(fd, (struct sockaddr *)&address, sizeof(address))) {
		close(fd); return -1;
	}
	return fd;
}

static int compare_ports_descending(const void *left, const void *right)
{
	unsigned a = *(const unsigned *)left;
	unsigned b = *(const unsigned *)right;
	return a < b ? 1 : a > b ? -1 : 0;
}

static void *scan_rsd_worker(void *opaque)
{
	struct rsd_scan *scan = opaque;
	unsigned port;
	while ((port = atomic_fetch_add(&scan->next_port, 1)) <= LAST_DYNAMIC_PORT) {
		struct sockaddr_in6 address = { .sin6_family = AF_INET6,
			.sin6_port = htons(port), .sin6_scope_id = scan->scope,
			.sin6_addr = scan->peer };
		struct pollfd item;
		uint8_t greeting[9];
		int error = 0;
		socklen_t error_size = sizeof(error);
		int fd = socket(AF_INET6, SOCK_STREAM | SOCK_NONBLOCK, 0);
		if (fd < 0) continue;
		if (connect(fd, (struct sockaddr *)&address, sizeof(address)) && errno != EINPROGRESS) {
			close(fd); continue;
		}
		item = (struct pollfd){ .fd = fd, .events = POLLIN | POLLOUT };
		if (poll(&item, 1, 250) <= 0 ||
		    getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &error_size) || error) {
			close(fd); continue;
		}
		item.events = POLLIN; item.revents = 0;
		if (poll(&item, 1, 500) > 0 && (item.revents & POLLIN) &&
		    recv(fd, greeting, sizeof(greeting), MSG_PEEK) == sizeof(greeting) &&
		    greeting[3] == 4 && !greeting[5] && !greeting[6] &&
		    !greeting[7] && !greeting[8]) {
			pthread_mutex_lock(&scan->lock);
			if (scan->count < MAX_RSD_CANDIDATES)
				scan->candidates[scan->count++] = port;
			pthread_mutex_unlock(&scan->lock);
		}
		close(fd);
	}
	return NULL;
}

static uint32_t be24(const uint8_t value[3])
{
	return ((uint32_t)value[0] << 16) | ((uint32_t)value[1] << 8) | value[2];
}

static int receive_http2_frame(int fd, uint8_t *type, uint8_t **body, uint32_t *length)
{
	uint8_t header[9];
	if (read_all(fd, header, sizeof(header))) return -1;
	*length = be24(header);
	*type = header[3];
	if (*length > RSD_RESPONSE_MAX) { errno = EMSGSIZE; return -1; }
	*body = malloc(*length ? *length : 1);
	if (!*body) return -1;
	if (*length && read_all(fd, *body, *length)) { free(*body); *body = NULL; return -1; }
	return 0;
}

static unsigned parse_rsd_biometric_port(const uint8_t *data, size_t length)
{
	static const char service[] = "com.apple.eos.BiometricKit";
	static const char port_key[] = "Port";
	const uint8_t *service_at = memmem(data, length, service, sizeof(service));
	const uint8_t *port_at;
	size_t remaining;
	uint32_t type;
	uint64_t port;

	if (!service_at) return 0;
	remaining = length - (size_t)(service_at - data);
	port_at = memmem(service_at, remaining, port_key, sizeof(port_key));
	if (!port_at || length - (size_t)(port_at - data) < 20) return 0;
	/* Aligned XPC key, followed by UINT64 type 0x4000 and the value. */
	for (const uint8_t *p = port_at + sizeof(port_key); p + 12 <= data + length && p < port_at + 16; p++) {
		memcpy(&type, p, sizeof(type));
		type = le32toh(type);
		if (type == 0x4000) {
			memcpy(&port, p + 4, sizeof(port));
			port = le64toh(port);
			if (port >= FIRST_DYNAMIC_PORT && port <= LAST_DYNAMIC_PORT) return port;
		} else if (type == 0x9000 && p + 8 <= data + length) {
			uint32_t string_length;
			char text[8] = { };
			char *end;
			memcpy(&string_length, p + 4, sizeof(string_length));
			string_length = le32toh(string_length);
			if (string_length < 2 || string_length > sizeof(text) ||
			    p + 8 + string_length > data + length)
				continue;
			memcpy(text, p + 8, string_length - 1);
			port = strtoul(text, &end, 10);
			if (*end == '\0' && port >= FIRST_DYNAMIC_PORT && port <= LAST_DYNAMIC_PORT)
				return port;
		}
	}
	return 0;
}

static unsigned activate_rsd(const char *host, const char *interface, unsigned rsd_port)
{
	uint8_t *frame = NULL, *response = NULL;
	uint8_t type;
	uint32_t length;
	size_t response_length = 0;
	struct timeval timeout = { .tv_sec = 3 };
	uint8_t data_header[9] = { 0,1,8, 0,0, 0,0,0,1 };
	uint8_t handshake[sizeof(rsd_handshake_wrapper)];
	unsigned port = 0;
	int fd = connect_peer(host, interface, rsd_port);
	if (fd < 0) return 0;
	setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
	memcpy(handshake, rsd_handshake_wrapper, sizeof(handshake));
	if (getrandom(handshake + 128, 16, 0) != 16 ||
	    write_all(fd, http2_preface, sizeof(http2_preface) - 1) ||
	    write_all(fd, rsd_setup_frames, sizeof(rsd_setup_frames))) goto out;
	/* The peer may grant a window before its SETTINGS frame. */
	for (unsigned i = 0; i < 8; i++) {
		if (receive_http2_frame(fd, &type, &frame, &length)) goto out;
		free(frame); frame = NULL;
		if (type == 4) break;
	}
	if (write_all(fd, http2_settings_ack, sizeof(http2_settings_ack))) goto out;
	data_header[0] = (sizeof(handshake) >> 16) & 0xff;
	data_header[1] = (sizeof(handshake) >> 8) & 0xff;
	data_header[2] = sizeof(handshake) & 0xff;
	if (write_all(fd, data_header, sizeof(data_header)) ||
	    write_all(fd, handshake, sizeof(handshake))) goto out;
	response = malloc(RSD_RESPONSE_MAX);
	if (!response) goto out;
	for (unsigned i = 0; i < 64; i++) {
		if (receive_http2_frame(fd, &type, &frame, &length)) goto out;
		if (type == 0 && length && response_length + length <= RSD_RESPONSE_MAX) {
			memcpy(response + response_length, frame, length);
			response_length += length;
			port = parse_rsd_biometric_port(response, response_length);
		}
		free(frame); frame = NULL;
		if (port) break;
	}
out:
	free(frame);
	free(response);
	close(fd);
	return port;
}

static int discover_peer(const char *host, const char *interface, unsigned *found_port)
{
	struct in6_addr peer;
	unsigned scope = if_nametoindex(interface);
	unsigned rsd_candidates[MAX_RSD_CANDIDATES];
	unsigned rsd_count = 0;
	const char *forced_rsd = getenv("T2_TOUCHID_RSD_PORT");

	if (!scope || inet_pton(AF_INET6, host, &peer) != 1) { errno = EINVAL; return -1; }
	if (forced_rsd && *forced_rsd) {
		unsigned long value = strtoul(forced_rsd, NULL, 10);
		if (!value || value > 65535) { errno = EINVAL; return -1; }
		rsd_candidates[rsd_count++] = value;
		goto activate_candidates;
	}
	rsd_candidates[rsd_count++] = RSD_DIRECT_PORT;
	{
		unsigned service_port = activate_rsd(host, interface, RSD_DIRECT_PORT);
		if (service_port) goto activate_candidates;
		rsd_count = 0;
		event("event=remote-xpc-direct-failed port=%u", RSD_DIRECT_PORT);
	}
	{
		struct rsd_scan scan = { .peer = peer, .scope = scope };
		pthread_t workers[SCAN_WORKERS];
		unsigned started = 0;
		atomic_init(&scan.next_port, FIRST_DYNAMIC_PORT);
		pthread_mutex_init(&scan.lock, NULL);
		for (; started < SCAN_WORKERS; started++)
			if (pthread_create(&workers[started], NULL, scan_rsd_worker, &scan))
				break;
		for (unsigned i = 0; i < started; i++) pthread_join(workers[i], NULL);
		pthread_mutex_destroy(&scan.lock);
		memcpy(rsd_candidates, scan.candidates,
		       scan.count * sizeof(rsd_candidates[0]));
		rsd_count = scan.count;
	}
	if (rsd_count) goto activate_candidates;
	errno = ENOENT;
	return -1;
activate_candidates:
	qsort(rsd_candidates, rsd_count, sizeof(rsd_candidates[0]),
	      compare_ports_descending);
	for (unsigned i = 0; i < rsd_count; i++) {
		event("event=remote-xpc-candidate port=%u", rsd_candidates[i]);
		fflush(stdout);
		unsigned service_port = activate_rsd(host, interface, rsd_candidates[i]);
		if (service_port) {
			int fd;
			for (unsigned retry = 0; retry < 10; retry++) {
				fd = connect_peer(host, interface, service_port);
				if (fd >= 0) {
					*found_port = service_port;
					return fd;
				}
				usleep(100000);
			}
		}
		fprintf(stderr, "remote-xpc: candidate %u did not advertise BiometricKit: %s\n",
			rsd_candidates[i], strerror(errno));
	}
	errno = ENOENT;
	return -1;
}

static int read_identity_inventory(int fd, uint32_t user_id)
{
	uint32_t wire_user_id = htole32(user_id);
	char *records = NULL;
	uint64_t length = 0;
	plist_t reply;

	reply = bio_request(fd, BIO_USER_IDENTITIES, 0, &wire_user_id,
			    sizeof(wire_user_id),
			    IDENTITY_RECORD_SIZE * MAX_IDENTITIES);
	if (reply_data(reply, &records, &length)) {
		plist_t output = reply && plist_get_node_type(reply) == PLIST_ARRAY &&
			plist_array_get_size(reply) > 1 ?
			plist_array_get_item(reply, 1) : NULL;
		char *sentinel = NULL;
		if (reply_ok(reply) && output &&
		    plist_get_node_type(output) == PLIST_STRING)
			plist_get_string_val(output, &sentinel);
		if (sentinel && !strcmp(sentinel, BIO_NIL_OUTPUT)) {
			event("event=identity-inventory uid=%u records=0 bytes=0 output=nil",
			      user_id);
			free(sentinel);
			plist_free(reply);
			return 0;
		}
		free(sentinel);
		fprintf(stderr,
			"warning: identity inventory for uid=%u unavailable: status=0x%08llx\n",
			user_id, (unsigned long long)(reply_status(reply) & UINT32_MAX));
		if (reply) plist_free(reply);
		return -1;
	}
	if (length % IDENTITY_RECORD_SIZE ||
	    length > IDENTITY_RECORD_SIZE * MAX_IDENTITIES) {
		fprintf(stderr, "warning: malformed identity inventory: bytes=%llu\n",
			(unsigned long long)length);
		free(records);
		plist_free(reply);
		errno = EPROTO;
		return -1;
	}
	enrolled_records_length = length;
	enrolled_count = 0;
	memcpy(enrolled_records, records, length);
	event("event=identity-inventory uid=%u records=%llu bytes=%llu",
	      user_id, (unsigned long long)(length / IDENTITY_RECORD_SIZE),
	      (unsigned long long)length);
	for (uint64_t offset = 0; offset < length; offset += IDENTITY_RECORD_SIZE) {
		uint32_t prefix_user_id, suffix_user_id;
		const uint8_t *record = (const uint8_t *)records + offset;
		const uint8_t *uuid;
		const char *layout;
		memcpy(&prefix_user_id, record, sizeof(prefix_user_id));
		memcpy(&suffix_user_id, record + 16, sizeof(suffix_user_id));
		prefix_user_id = le32toh(prefix_user_id);
		suffix_user_id = le32toh(suffix_user_id);
		if (prefix_user_id == user_id) {
			layout = "prefix";
			uuid = record + 4;
		} else if (suffix_user_id == user_id) {
			layout = "suffix";
			uuid = record;
		} else {
			event("event=identity-record slot=%llu uid-layout=unknown",
			      (unsigned long long)(offset / IDENTITY_RECORD_SIZE));
			continue;
		}
		memcpy(enrolled_uuids[enrolled_count++], uuid, 16);
		event("event=identity-record slot=%llu uid=%u uid-layout=%s uuid-prefix=%02x%02x%02x%02x",
		      (unsigned long long)(offset / IDENTITY_RECORD_SIZE), user_id,
		      layout, uuid[0], uuid[1], uuid[2], uuid[3]);
	}
	memset(records, 0, length);
	free(records);
	plist_free(reply);
	return 0;
}

/* cmd_match_mode_in_v1_t: flags, user id, 60 reserved bytes, then the
 * selected identity records without a count in front of them. */
static int load_catacomb_file(int fd, const char *path, uint16_t op, bool required)
{
	uint8_t buf[64 * 1024];
	size_t n;
	plist_t reply;
	FILE *f = fopen(path, "rb");
	if (!f) {
		if (required) fprintf(stderr, "open %s: %s\n", path, strerror(errno));
		return required ? -1 : 0;
	}
	n = fread(buf, 1, sizeof(buf), f);
	fclose(f);
	if (!n) { fprintf(stderr, "empty %s\n", path); return -1; }
	reply = bio_request(fd, op, 0, buf, n, 0);
	if (!reply_ok(reply)) {
		event("event=catacomb-load file=%s op=0x%x bytes=%zu status=0x%08llx",
		      path, op, n, (unsigned long long)(reply_status(reply) & UINT32_MAX));
		if (reply) plist_free(reply);
		return -1;
	}
	event("event=catacomb-load file=%s op=0x%x bytes=%zu status=ok", path, op, n);
	plist_free(reply);
	return 0;
}

static int load_catacomb(int fd, const char *dir, uint32_t user_id)
{
	char path[PATH_MAX];
	snprintf(path, sizeof(path), "%s/master.bin", dir);
	if (load_catacomb_file(fd, path, BIO_LOAD_CATACOMB, true)) return -1;
	snprintf(path, sizeof(path), "%s/user_%u.bin", dir, user_id);
	if (load_catacomb_file(fd, path, BIO_LOAD_CATACOMB, true)) return -1;
	snprintf(path, sizeof(path), "%s/biolockout.bin", dir);
	return load_catacomb_file(fd, path, BIO_LOAD_BIOLOCKOUT, false);
}

static void read_sks_lock_state(int fd, uint32_t user_id, const char *when)
{
	uint32_t wire = htole32(user_id);
	char *out = NULL;
	uint64_t len = 0;
	plist_t reply = bio_request(fd, BIO_SKS_LOCK_STATE, 0, &wire, sizeof(wire), 4);
	if (reply_data(reply, &out, &len) || len != 4) {
		event("event=sks-lock-state when=%s error status=0x%08llx", when,
		      (unsigned long long)(reply_status(reply) & UINT32_MAX));
	} else {
		uint32_t state;
		memcpy(&state, out, 4);
		event("event=sks-lock-state when=%s uid=%u value=0x%08x", when,
		      user_id, le32toh(state));
	}
	free(out);
	if (reply) plist_free(reply);
}

/* handleMatch gates the SEP image comparison on _provisioningState == 2.
 * Read it here so we can tell whether our loaded state satisfies that gate
 * before blaming the match flags. The argument shape is not documented, so
 * try no argument first and fall back to the 4-byte user id. */
static void read_provisioning_state(int fd, uint32_t user_id, const char *when)
{
	uint32_t wire = htole32(user_id);
	char *out = NULL;
	uint64_t len = 0;
	plist_t reply = bio_request(fd, BIO_PROVISIONING_STATE, 0, NULL, 0, 64);
	uint64_t status = reply_status(reply);

	if (status != 0) {
		if (reply) plist_free(reply);
		reply = bio_request(fd, BIO_PROVISIONING_STATE, 0, &wire,
				    sizeof(wire), 64);
		status = reply_status(reply);
	}
	if (!reply_data(reply, &out, &len) && len) {
		char hex[64 * 3 + 1];
		uint64_t i;
		uint32_t state = 0;
		for (i = 0; i < len && i < 64; i++)
			sprintf(hex + i * 3, "%02x ", (uint8_t)out[i]);
		hex[i ? i * 3 - 1 : 0] = 0;
		if (len >= 4) memcpy(&state, out, 4);
		event("event=provisioning-state when=%s uid=%u status=0x%08llx state=%u bytes=%llu data=%s",
		      when, user_id, (unsigned long long)(status & UINT32_MAX),
		      le32toh(state), (unsigned long long)len, hex);
	} else {
		/* The reply may carry the state as an integer instead of a data
		 * blob; surface the reply shape so we can see what it returns. */
		uint64_t items = reply && plist_get_node_type(reply) == PLIST_ARRAY ?
			plist_array_get_size(reply) : 0;
		plist_t item = items > 1 ? plist_array_get_item(reply, 1) : NULL;
		uint64_t value = 0;
		bool is_uint = item && plist_get_node_type(item) == PLIST_UINT;
		if (is_uint) plist_get_uint_val(item, &value);
		event("event=provisioning-state when=%s uid=%u status=0x%08llx items=%llu %s=%llu",
		      when, user_id, (unsigned long long)(status & UINT32_MAX),
		      (unsigned long long)items, is_uint ? "state" : "no-data",
		      (unsigned long long)value);
	}
	free(out);
	if (reply) plist_free(reply);
}

static int start_match(int fd, uint32_t user_id, uint32_t processed_flags,
		       uint32_t credential_set)
{
	uint8_t payload[MATCH_INIT_SIZE + sizeof(enrolled_records)];
	size_t length = MATCH_INIT_SIZE;
	uint32_t value;
	plist_t reply;

	memset(payload, 0, sizeof(payload));
	value = le32(processed_flags);
	memcpy(payload, &value, sizeof(value));
	value = le32(credential_set);
	memcpy(payload + 4, &value, sizeof(value));
	if (enrolled_records_length) {
		memcpy(payload + length, enrolled_records, enrolled_records_length);
		length += enrolled_records_length;
	}
	read_sks_lock_state(fd, user_id, "before-match");
	event("event=match-start uid=%u flags=0x%x credential-set=0x%x identities=%u bytes=%zu",
	      user_id, processed_flags, credential_set, enrolled_count, length);
	reply = bio_request(fd, BIO_MATCH, 0, payload, length, 0);
	if (!reply_ok(reply)) {
		fprintf(stderr, "match rejected: status=0x%08llx\n",
			(unsigned long long)(reply_status(reply) & UINT32_MAX));
		if (reply) plist_free(reply);
		return -1;
	}
	plist_free(reply);
	return 0;
}

static int read_system_config(int fd)
{
	char *out = NULL;
	uint64_t len = 0;
	plist_t reply = bio_request(fd, BIO_SYSTEM_CONFIG, 0, NULL, 0, 28);
	if (reply_data(reply, &out, &len)) {
		event("event=system-config error status=0x%08llx",
		      (unsigned long long)(reply_status(reply) & UINT32_MAX));
		if (reply) plist_free(reply);
		return -1;
	}
	{
		char hex[28 * 3 + 1];
		size_t i;
		for (i = 0; i < len && i < 28; i++)
			sprintf(hex + i * 3, "%02x ", (uint8_t)out[i]);
		hex[i ? i * 3 - 1 : 0] = 0;
		event("event=system-config bytes=%llu data=%s",
		      (unsigned long long)len, hex);
	}
	free(out);
	plist_free(reply);
	return 0;
}

static int start_sensor_observation(int fd, uint32_t user_id, bool match, bool config, uint32_t match_flags, uint32_t credential_set, const char *catacomb_dir)
{
	plist_t request_value, reply = NULL;
	char *fdr = NULL, *readiness_data = NULL;
	uint64_t fdr_length = 0, readiness_length = 0;
	uint8_t readiness = 0;
	int ret = -1;
	request_value = plist_new_array();
	plist_array_append_item(request_value, plist_new_uint(1));
	reply = request(fd, request_value, true);
	if (!reply_ok(reply)) { fprintf(stderr, "BiometricKit service is not open\n"); goto out; }
	plist_free(reply); reply = NULL;

	event("event=phase step=service-open");
	reply = bio_request(fd, 2, 2, NULL, 0, 0);
	if (!reply_ok(reply)) { fprintf(stderr, "sensor reset rejected\n"); goto out; }
	event("event=phase step=sensor-reset");
	plist_free(reply); reply = NULL;

	reply = bio_request(fd, BIO_CANCEL, 0, NULL, 0, 0);
	if (!reply_ok(reply)) { fprintf(stderr, "post-reset cancel rejected\n"); goto out; }
	event("event=phase step=cancel");
	plist_free(reply); reply = NULL;

	reply = bio_request(fd, 0x53, 0, NULL, 0, 1);
	if (reply_data(reply, &readiness_data, &readiness_length) ||
	    readiness_length != 1) {
		fprintf(stderr, "sensor readiness query failed\n");
		goto out;
	}
	readiness = readiness_data[0];
	free(readiness_data); readiness_data = NULL; readiness_length = 0;
	plist_free(reply); reply = NULL;
	if (!readiness) { fprintf(stderr, "sensor is not ready\n"); goto out; }
	event("event=phase step=readiness ready=%u", readiness);

	request_value = plist_new_array();
	plist_array_append_item(request_value, plist_new_uint(11));
	reply = request(fd, request_value, true);
	if (!reply || plist_get_node_type(reply) != PLIST_ARRAY ||
	    plist_array_get_size(reply) != 1 ||
	    plist_get_node_type(plist_array_get_item(reply, 0)) != PLIST_DATA) {
		fprintf(stderr, "bridgeOS returned no FDR calibration\n");
		goto out;
	}
	plist_get_data_val(plist_array_get_item(reply, 0), &fdr, &fdr_length);
	plist_free(reply); reply = NULL;
	if (!fdr || !fdr_length) { fprintf(stderr, "FDR calibration is empty\n"); goto out; }
	event("event=phase step=fdr-fetch bytes=%llu", (unsigned long long)fdr_length);

	reply = bio_request(fd, 0x20, 3, fdr, fdr_length, 0);
	memset(fdr, 0, fdr_length); free(fdr); fdr = NULL;
	if (!reply_ok(reply)) { fprintf(stderr, "FDR calibration load rejected\n"); goto out; }
	plist_free(reply); reply = NULL;
	event("event=phase step=fdr-load");

	read_identity_inventory(fd, user_id);
	read_provisioning_state(fd, user_id, "before-catacomb");

	if (catacomb_dir && load_catacomb(fd, catacomb_dir, user_id)) {
		fprintf(stderr, "catacomb load failed\n");
		goto out;
	}
	if (catacomb_dir)
		read_provisioning_state(fd, user_id, "after-catacomb");
	if (config) {
		ret = read_system_config(fd);
		goto out;
	}
	if (match) {
		ret = start_match(fd, user_id, match_flags, credential_set);
		goto out;
	}
	reply = bio_request(fd, BIO_PRESENCE, 0, NULL, 0, 0);
	if (!reply_ok(reply)) { fprintf(stderr, "presence request rejected\n"); goto out; }
	ret = 0;
out:
	if (reply) plist_free(reply);
	if (fdr) { memset(fdr, 0, fdr_length); free(fdr); }
	free(readiness_data);
	return ret;
}

int main(int argc, char **argv)
{
	int fd, option, seconds = 15;
	bool match = false, config = false;
	unsigned long match_flags = 0;
	unsigned long credential_set = 0xffffffff;
	const char *catacomb_dir = NULL;
	unsigned long user_id = 501;
	unsigned long port = 0;
	unsigned discovered_port = 0;
	uint32_t type, length;
	unsigned bridge_version;
	uint8_t *body = NULL;
	plist_t value, reply;
	char helo[256];
	char interface[64] = "";
	const char *host = DEFAULT_PEER;
	struct pollfd pollfd;
	time_t deadline;

	clock_gettime(CLOCK_MONOTONIC, &probe_start);
	signal(SIGINT, on_sigint);
	while ((option = getopt(argc, argv, "I:H:p:t:u:mcf:k:C:")) != -1) {
		char *end;
		switch (option) {
		case 'I':
			snprintf(interface, sizeof(interface), "%s", optarg);
			break;
		case 'H':
			host = optarg;
			break;
		case 'p':
			port = strtoul(optarg, &end, 10);
			if (*end || !port || port > 65535) return 2;
			break;
		case 't':
			seconds = (int)strtol(optarg, &end, 10);
			if (*end || seconds < 1 || seconds > 300) return 2;
			break;
		case 'u':
			user_id = strtoul(optarg, &end, 10);
			if (*end || user_id > UINT32_MAX) return 2;
			break;
		case 'm':
			match = true;
			break;
		case 'f':
			match_flags = strtoul(optarg, NULL, 0);
			if (match_flags > UINT32_MAX) return 2;
			break;
		case 'k':
			credential_set = strtoul(optarg, NULL, 0);
			if (credential_set > UINT32_MAX) return 2;
			break;
		case 'c':
			config = true;
			break;
		case 'C':
			catacomb_dir = optarg;
			break;
		default:
			fprintf(stderr,
				"usage: %s [-I INTERFACE] [-H PEER] [-p PORT] [-t SECONDS] [-u MACOS_UID] [-m]\n"
				"  -I  Apple CDC-NCM interface     (default: autodetected)\n"
				"  -H  T2 link-local address       (default: %s)\n"
				"  -p  BiometricKit port           (default: discovered)\n"
				"  -t  observation seconds, 1-300  (default: 15)\n"
				"  -u  macOS user ID               (default: 501)\n"
				"  -m  run a match instead of a presence scan\n"
				"  -f  processed match flags (default 0; 1 for unlock match)\n"
				"  -k  credential-set handle       (default 0xffffffff, none)\n"
				"  -C  load catacomb blobs from DIR before matching\n"
				"  -c  read GetSystemProtectedConfig and exit\n",
				argv[0], DEFAULT_PEER);
			return 2;
		}
	}
	if (!interface[0] && find_interface(interface, sizeof(interface)))
		return 1;
	if (!interface_has_link_local(interface)) {
		fprintf(stderr,
			"interface %s has no IPv6 link-local address; enable IPv6 on the CDC-NCM link\n",
			interface);
		return 1;
	}
	event("event=link interface=%s peer=%s", interface, host);
	if (!port) {
		event("event=discovering-biometric-service");
		fd = discover_peer(host, interface, &discovered_port);
		if (fd >= 0) event("event=service-found port=%u", discovered_port);
	} else {
		fd = connect_peer(host, interface, port);
	}
	if (fd < 0) {
		if (!port && errno == ENOENT)
			fprintf(stderr, "discovery: BiometricKit service was not activated\n");
		else
			perror("connect");
		return 1;
	}
	if (receive_frame(fd, &type, &body, &length) || type != BRIDGE_HELO) {
		perror("receive HELO"); return 1;
	}
	bridge_version = peer_bridge_version(body, length);
	free(body);
	if (!bridge_version) { fprintf(stderr, "invalid peer HELO\n"); return 1; }
	snprintf(helo, sizeof(helo),
		 "{\"MaxSupportedProtocolVersion\":1,\"OSBuild\":\"Linux\","
		 "\"BridgeXPCVersion\":%u,\"ProcessName\":\"t2-touchid-probe\"}",
		 bridge_version);
	if (send_frame(fd, BRIDGE_HELO, helo, strlen(helo))) { perror("send HELO"); return 1; }
	value = plist_new_array(); plist_array_append_item(value, plist_new_uint(0));
	reply = request(fd, value, false);
	if (!reply || plist_array_get_size(reply) != 2) { fprintf(stderr, "getBridgeVersion failed\n"); return 1; }
	plist_free(reply);
	value = plist_new_array(); plist_array_append_item(value, plist_new_uint(10));
	plist_array_append_item(value, plist_new_uint(2));
	reply = request(fd, value, false);
	if (!reply_ok(reply)) { fprintf(stderr, "setClientVersion failed\n"); return 1; }
	plist_free(reply);
	if (start_sensor_observation(fd, (uint32_t)user_id, match, config,
				    (uint32_t)match_flags, (uint32_t)credential_set,
				    catacomb_dir)) return 1;
	if (config) { close(fd); return 0; }
	event(match ? "event=waiting-for-match" : "event=waiting-for-finger");
	deadline = time(NULL) + seconds;
	pollfd.fd = fd; pollfd.events = POLLIN;
	while (time(NULL) < deadline && !stop_requested) {
		int ready = poll(&pollfd, 1, 250);
		if (ready < 0 && errno == EINTR) continue;
		if (ready < 0) { perror("poll"); break; }
		if (ready > 0) {
			plist_t incoming = receive_plist(fd, BRIDGE_MESSAGE);
			plist_t id, payload, ack;
			char *id_text = NULL;
			if (!incoming) { perror("event"); break; }
			if (plist_array_get_size(incoming) != 4) { plist_free(incoming); break; }
			id = plist_array_get_item(incoming, 2);
			payload = plist_array_get_item(incoming, 3);
			describe_event(payload);
			plist_get_string_val(id, &id_text);
			if (id_text) {
				plist_t result = plist_new_array(); plist_array_append_item(result, plist_new_uint(0));
				ack = envelope(id_text, true, result); send_plist(fd, ack); plist_free(ack); free(id_text);
			}
			plist_free(incoming);
		}
	}
	reply = bio_request(fd, BIO_CANCEL, 0, NULL, 0, 0);
	if (reply) plist_free(reply);
	if (match) read_sks_lock_state(fd, (uint32_t)user_id, "after-match");
	close(fd);
	return 0;
}

// SPDX-License-Identifier: GPL-2.0-only
/*
 * t2smc - Minimal SMC driver for T2 Macs
 *
 * Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
 * Copyright (C) 2026 Atharva Tiwari <atharvatiwarilinuxdev@gmail.com>
 * 
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#define T2SMC_VERSION "0.0.4"

#include <linux/delay.h>
#include <linux/acpi.h>
#include <linux/completion.h>
#include <linux/kernel.h>
#include <linux/bitops.h>
#include <linux/math64.h>
#include <linux/miscdevice.h>
#include <linux/slab.h>
#include <linux/module.h>
#include <linux/mutex.h>
#include <linux/hwmon.h>
#include <linux/hwmon-sysfs.h>
#include <linux/interrupt.h>
#include <linux/io.h>
#include <linux/err.h>
#include <linux/ktime.h>
#include <linux/power_supply.h>
#include <linux/platform_device.h>
#include <linux/suspend.h>
#include <linux/rtc.h>
#include <linux/string.h>
#include <linux/uaccess.h>
#include <linux/watchdog.h>
#include <linux/workqueue.h>

/* MMIO register offsets for T2 SMC interface */
#define T2SMC_IOMEM_KEY_DATA      0x0000
#define T2SMC_IOMEM_KEY_STATUS    0x4005
#define T2SMC_IOMEM_KEY_NAME      0x0078
#define T2SMC_IOMEM_KEY_DATA_LEN  0x007D
#define T2SMC_IOMEM_KEY_SMC_ID    0x007E
#define T2SMC_IOMEM_KEY_CMD       0x007F
#define T2SMC_IOMEM_MIN_SIZE      0x4006
#define T2SMC_IOMEM_INT_STATUS    0x4000

/* I/O port window, only used to fetch the event ID of an interrupt */
#define T2SMC_PORT_MIN_SIZE       32
#define T2SMC_PORT_EVENT          0x1f
#define T2SMC_PORT_THERMAL_CPU    0x18  /* thermal levels */
#define T2SMC_PORT_THERMAL_IO     0x19
#define T2SMC_PORT_THERMAL_GPU    0x1a
#define T2SMC_PORT_PLIMIT         0x10  /* CPU, GPU, memory, idle steps */
#define T2SMC_PORT_PLIMIT_GPU     0x14

/* SMC log text that comes with event 0x4c */
#define T2SMC_IOMEM_LOG           0x0080
#define T2SMC_LOG_LEN             0x7f

/* SMC event IDs delivered by interrupt while NTOK is set, names as in AppleSMC */
#define T2SMC_EVENT_SHUTDOWN       0x40  /* ShutdownImminent */
#define T2SMC_EVENT_BRIDGEOS_PANIC 0x41  /* BridgeOSPanic */
#define T2SMC_EVENT_KEY_DONE       0x4b  /* KeyDone, every finished command */
#define T2SMC_EVENT_LOG            0x4c  /* LogMessage */
#define T2SMC_EVENT_THERMAL_LEVEL  0x54  /* PThermalLevelChanged */
#define T2SMC_EVENT_THERMAL_CONFIG 0x55  /* SMC_Thermal_Config_Notification */
#define T2SMC_EVENT_PLIMIT         0x80  /* PLimitChange */

/* Key type info in MMIO (after GET_KEY_TYPE_CMD) */
#define T2SMC_IOMEM_KEY_TYPE_CODE      0
#define T2SMC_IOMEM_KEY_TYPE_DATA_LEN  5
#define T2SMC_IOMEM_KEY_TYPE_FLAGS     6

#define T2SMC_CMD_TIMEOUT_MS    1000  /* as AppleSMC waitForKeyDone */
#define T2SMC_NTOK_DRAIN_TRIES  8     /* as AppleSMC before writing NTOK */
#define T2SMC_NTOK_DRAIN_MS     20

/* SMC commands */
#define T2SMC_READ_CMD               0x10
#define T2SMC_WRITE_CMD              0x11
#define T2SMC_GET_KEY_BY_INDEX_CMD   0x12
#define T2SMC_GET_KEY_TYPE_CMD       0x13

/* Known keys */
#define KEY_COUNT_KEY   "#KEY"  /* r-o ui32 */
#define FANS_COUNT      "FNum"  /* r-o ui8  */
#define FANS_MANUAL     "FS! "  /* r-w ui16 (legacy) */
#define T2SMC_RTC_COUNTER  "CLKM"  /* r-o 48-bit 32768 Hz counter */
#define T2SMC_RTC_LATCH    "CLKL"  /* r-w latched 48-bit counter */
#define T2SMC_RTC_OFFSET   "CLKO"  /* r-w 48-bit offset */
#define T2SMC_RTC_RATE     "CLKR"  /* r-o 32-bit ticks per second */
/*
 * powerd sets the charge limit on T2 Macs through CHLS: byte 0 holds the SoC
 * limit, 0 for none, byte 1 flags. BCLM is the charge limit of Macs before
 * the T2. The SMC still honours it, but macOS does not write it on T2 Macs.
 */
#define T2SMC_CHARGE_LIMIT_LEGACY "BCLM"
#define T2SMC_CHARGE_LIMIT        "CHLS"
/* Gauge data powerd decides the calibration charge on, one key per cell */
#define T2SMC_BATTERY_QMAX      "BQX%u"
#define T2SMC_BATTERY_DOD0      "BQD%u"
#define T2SMC_BATTERY_MAX_CELLS 4
#define T2SMC_BATTERY_STATUS    "BNCR"
#define T2SMC_BATTERY_CAPACITY  "BRSC"
#define T2SMC_BATTERY_VOLTAGE   "B0AV"
#define T2SMC_BATTERY_CURRENT   "B0AC"
#define T2SMC_BATTERY_POWER     "B0AP"
#define T2SMC_BATTERY_FULL      "B0FC"
#define T2SMC_BATTERY_REMAINING "B0RM"
#define T2SMC_BATTERY_CYCLES    "B0CT"
#define T2SMC_ADAPTER_CURRENT   "ID0R"
#define T2SMC_ADAPTER_POWER_OLD "PD0R"
#define T2SMC_ADAPTER_POWER     "PDTR"
#define T2SMC_ADAPTER_VOLTAGE   "VD0R"
#define T2SMC_BATTERY_TIME_TO_EMPTY "B0TE"
#define T2SMC_BATTERY_TIME_TO_FULL  "B0TF"
#define T2SMC_CHARGE_CURRENT    "CHBI"
#define T2SMC_CHARGE_VOLTAGE    "CHBV"
#define T2SMC_CELL_VOLTAGE_MAX  "BCMV"
#define T2SMC_SHUTDOWN_CAUSE    "MSSD"  /* r-w s8 cause of the last shutdown */
#define T2SMC_SHUTDOWN_FLAG     "MSSW"  /* r-w flag that confirms cause -64 */
/* x86 state keys that bridgeOS saves into its panic log */
#define T2SMC_X86_POWER_STATE   "MSPP"
#define T2SMC_X86_SYSTEM_STATE  "MSPR"
#define T2SMC_X86_EFI_STATE     "EFBS"
#define T2SMC_X86_TRANSITIONS   "MSPU"  /* previous power transitions */
#define T2SMC_NOTIFY            "NTOK"  /* r-w flag, enables event interrupts */
#define T2SMC_WDT_TIMER         "OSWD"  /* r-w watchdog timeout in seconds */
#define T2SMC_WDT_LEGACY_TIMER  "NATi"  /* r-w legacy watchdog timeout */
#define T2SMC_WDT_LEGACY_MODE   "NATJ"  /* r-w legacy watchdog action */
/*
 * Keys that select the PLimit data layout and hold the GPU step counts.
 * macOS uses HE0N and HI0N for the discrete and
 * the integrated GPU, or HE2N and HE0N when HE2N exists.
 */
#define T2SMC_PLIMIT_GEN        "RGEN"
#define T2SMC_PLIMIT_VERSION    "DPLM"
#define T2SMC_PLIMIT_EGPU_STEPS "HE0N"
#define T2SMC_PLIMIT_IGPU_STEPS "HI0N"
#define T2SMC_PLIMIT_EGPU3_STEPS "HE2N"
enum { T2SMC_DGPU, T2SMC_IGPU, T2SMC_GPUS };
/*
 * macOS writes the turbo P-state count to MSTS and
 * the non-turbo P-states above CPUFloor to MSPC, in that order.
 */
#define T2SMC_CPU_TURBO_STATES  "MSTS"
#define T2SMC_CPU_FLOOR_STATES  "MSPC"
#define FLOAT_TYPE      "flt "

#define T2SMC_RTC_BYTES      6
#define T2SMC_RTC_BITS       (8 * T2SMC_RTC_BYTES)
#define T2SMC_RTC_SEC_SHIFT  15
#define T2SMC_RTC_DEFAULT_RATE  BIT(T2SMC_RTC_SEC_SHIFT)
#define T2SMC_RTC_MASK       GENMASK_ULL(T2SMC_RTC_BITS - 1, 0)
#define T2SMC_RTC_LATCH_OUTER_RETRIES  4
#define T2SMC_RTC_LATCH_WRITE_RETRIES  40
#define T2SMC_RTC_LATCH_READ_RETRIES   20
/* Sampling the system clock and latching the counter are a few ms apart */
#define T2SMC_RTC_SYNC_TOLERANCE_MS    50
#define T2SMC_WDT_LEGACY_RESTART 2
/* Shutdown causes that macOS treats specially */
#define T2SMC_CAUSE_FLAGGED      0xc0  /* -64, only valid while MSSW is 1 */
#define T2SMC_CAUSE_ONE_SHOT     0xc2  /* -62, reset to 6 once reported */
#define T2SMC_CAUSE_RESET        6
#define T2SMC_WDT_DEFAULT_TIMEOUT 60
#define T2SMC_WDT_HW_MAX_TIMEOUT 255  /* the SMC takes at most one byte */
#define T2SMC_MAX_FANS           10
#define T2SMC_FAN_LABEL_LEN      12

static int wdt_timeout = T2SMC_WDT_DEFAULT_TIMEOUT;
module_param(wdt_timeout, int, 0444);
MODULE_PARM_DESC(wdt_timeout, "Watchdog timeout in seconds (default="
		 __MODULE_STRING(T2SMC_WDT_DEFAULT_TIMEOUT) ")");;

static bool nowayout = WATCHDOG_NOWAYOUT;
module_param(nowayout, bool, 0444);
MODULE_PARM_DESC(nowayout, "Watchdog cannot be stopped once started (default="
		 __MODULE_STRING(WATCHDOG_NOWAYOUT) ")");

/* Fan speed key formats */
static const char *const fan_speed_fmt[] = {
	"F%dAc",  /* actual speed      - idx 0 */
	"F%dMn",  /* minimum speed     - idx 1 */
	"F%dMx",  /* maximum speed     - idx 2 */
	"F%dSf",  /* safe speed        - idx 3 */
	"F%dTg",  /* target speed (rw) - idx 4 */
};
#define FAN_MANUAL_FMT  "F%dMd"  /* T2 per-fan manual mode key */

#define INIT_TIMEOUT_MSECS  5000
#define INIT_WAIT_MSECS     50

/* -- SMC entry cache entry -- */
struct t2smc_entry {
	char key[5];
	u8   valid;
	u8   len;
	char type[5];
	u8   flags;
};

/* Sensor keys of one hwmon channel type, discovered at probe */
struct t2smc_sensors {
	unsigned int count;
	char (*keys)[5];
};

/* -- Main device structure -- */
struct t2smc_device {
	struct acpi_device *adev;
	struct device *dev;

	/* MMIO */
	bool iomem_ok;
	void __iomem *iomem;
	u32 iomem_addr, iomem_size;

	/* Event interrupt */
	u16 port_base;
	bool has_port;
	bool has_events;             /* IRQ and I/O port window are set up */
	bool cmd_irq;                /* commands wait for KeyDone by interrupt */
	struct completion cmd_done;
	struct work_struct sync_work;  /* storage sync on ShutdownImminent */
	struct work_struct thermal_work;  /* notifies thermal level readers */
	u8 thermal_level[3];
	struct work_struct plimit_work;   /* notifies PLimit readers */
	u8 gpu_steps[T2SMC_GPUS];        /* set by user space, 0 keeps the SMC preset */
	u8 cpu_floor[2];                 /* MSTS and MSPC from user space */
	bool cpu_floor_set;
	u8 plimit_version;               /* data layout of the PLimit words, 0 if unknown */

	/* Key cache */
	struct mutex mutex;
	unsigned int key_count;
	unsigned int fan_count;
	struct t2smc_entry *cache;
	struct t2smc_sensors temp;
	struct t2smc_sensors curr;
	struct t2smc_sensors in;
	struct t2smc_sensors power;

	/* Fans */
	char fan_labels[T2SMC_MAX_FANS][T2SMC_FAN_LABEL_LEN + 1];
	unsigned long fan_manual; /* fans set to manual mode via fanN_target */
	unsigned int fan_target[T2SMC_MAX_FANS];

	/* Watchdog */
	struct watchdog_device wdd;
	bool has_wdt;
	bool has_oswd;
	bool wdt_suspended;

	struct rtc_device *rtc_dev;
	bool has_rtc_latch;
	s64 rtc_offset;
	u32 rtc_rate;
	struct device *hwmon_dev;
	bool has_chls;
	struct mutex charge_lock;       /* guards the two fields below */
	u8 charge_limit;                /* user limit in percent, 100 for none */
	bool charge_limit_suspended;    /* lifted for a calibration charge */

	/* guards @battery against the async attach from power_event_work */
	struct mutex battery_lock;
	struct power_supply *battery;

	struct notifier_block power_supply_nb;
	struct work_struct power_event_work;
	bool power_notifier_registered;
	atomic64_t power_event_count;
	u64 power_last_event_ns;
	u8 power_status;
	bool power_status_valid;

	/* /dev/smc raw key access */
	struct miscdevice miscdev;
};

/* -- MMIO helpers -- */
static inline void iomem_clear_status(struct t2smc_device *t2)
{
	reinit_completion(&t2->cmd_done);
	if (ioread8(t2->iomem + T2SMC_IOMEM_KEY_STATUS))
		iowrite8(0, t2->iomem + T2SMC_IOMEM_KEY_STATUS);
}

static inline bool iomem_cmd_done(struct t2smc_device *t2)
{
	return ioread8(t2->iomem + T2SMC_IOMEM_KEY_STATUS) & 0x20;
}

/*
 * With SMC events enabled the KeyDone wait sleeps until the KeyDone
 * interrupt, otherwise it polls the key status every millisecond.
 */
static int iomem_wait_poll(struct t2smc_device *t2)
{
	int ms;

	for (ms = 0; ms < T2SMC_CMD_TIMEOUT_MS; ms++) {
		if (iomem_cmd_done(t2))
			return 0;
		usleep_range(1000, 2000);
	}
	if (iomem_cmd_done(t2))
		return 0;
	dev_warn(t2->dev, "%s: timeout\n", __func__);
	return -ETIMEDOUT;
}

/*
 * Like waitForKeyDoneUsingInterrupt: a missing interrupt reverts to polling
 * until NTOK is written again, and a command whose status already reports
 * KeyDone still succeeds. The status is checked after every wakeup because a
 * late interrupt of the previous command can complete the wait early.
 */
static int iomem_wait_irq(struct t2smc_device *t2)
{
	unsigned long left = msecs_to_jiffies(T2SMC_CMD_TIMEOUT_MS);

	for (;;) {
		left = wait_for_completion_timeout(&t2->cmd_done, left);
		if (!left)
			break;
		if (iomem_cmd_done(t2))
			return 0;
		reinit_completion(&t2->cmd_done);
		/* KeyDone may have arrived between the check and the reinit */
		if (iomem_cmd_done(t2))
			return 0;
	}

	WRITE_ONCE(t2->cmd_irq, false);
	dev_dbg(t2->dev, "no KeyDone interrupt, reverting to polling\n");
	if (iomem_cmd_done(t2))
		return 0;
	return -ETIMEDOUT;
}

static int iomem_wait_read(struct t2smc_device *t2)
{
	if (READ_ONCE(t2->cmd_irq))
		return iomem_wait_irq(t2);
	return iomem_wait_poll(t2);
}

/* -- MMIO SMC read/write -- */
static int iomem_read_smc(struct t2smc_device *t2,
			   u8 cmd, const char *key, u8 *buffer, u8 len)
{
	u8 err, remote_len;
	u32 key_int;

	memcpy(&key_int, key, sizeof(key_int));
	iomem_clear_status(t2);
	iowrite32(key_int, t2->iomem + T2SMC_IOMEM_KEY_NAME);
	iowrite8(0, t2->iomem + T2SMC_IOMEM_KEY_SMC_ID);
	iowrite8(cmd, t2->iomem + T2SMC_IOMEM_KEY_CMD);

	if (iomem_wait_read(t2))
		return -EIO;

	err = ioread8(t2->iomem + T2SMC_IOMEM_KEY_CMD);
	if (err != 0) {
		pr_debug("read_smc_mmio(%x %.4s) failed: %u\n",
			cmd, key, err);
		return -EIO;
	}

	if (cmd == T2SMC_READ_CMD) {
		remote_len = ioread8(t2->iomem + T2SMC_IOMEM_KEY_DATA_LEN);
		if (remote_len != len) {
			dev_warn(t2->dev,
				 "read_smc_mmio(%x %.4s): len mismatch (remote=%u, req=%u)\n",
				 cmd, key, remote_len, len);
			return -EINVAL;
		}
	} else {
		remote_len = len;
	}

	memcpy_fromio(buffer, t2->iomem + T2SMC_IOMEM_KEY_DATA, remote_len);
	return 0;
}

static int iomem_write_smc(struct t2smc_device *t2,
			    u8 cmd, const char *key, const u8 *buffer, u8 len)
{
	u8 err;
	u32 key_int;

	memcpy(&key_int, key, sizeof(key_int));
	iomem_clear_status(t2);
	iowrite32(key_int, t2->iomem + T2SMC_IOMEM_KEY_NAME);
	memcpy_toio(t2->iomem + T2SMC_IOMEM_KEY_DATA, buffer, len);
	iowrite8(len, t2->iomem + T2SMC_IOMEM_KEY_DATA_LEN);
	iowrite8(0, t2->iomem + T2SMC_IOMEM_KEY_SMC_ID);
	iowrite8(cmd, t2->iomem + T2SMC_IOMEM_KEY_CMD);

	if (iomem_wait_read(t2))
		return -EIO;

	err = ioread8(t2->iomem + T2SMC_IOMEM_KEY_CMD);
	if (err != 0) {
		pr_debug("write_smc_mmio(%x %.4s) failed: %u\n",
			cmd, key, err);
		return -EIO;
	}
	return 0;
}

static int iomem_get_key_info(struct t2smc_device *t2,
			       const char *key, struct t2smc_entry *info)
{
	u8 err;
	u32 key_int, type;

	memcpy(&key_int, key, sizeof(key_int));
	iomem_clear_status(t2);
	iowrite32(key_int, t2->iomem + T2SMC_IOMEM_KEY_NAME);
	iowrite8(0, t2->iomem + T2SMC_IOMEM_KEY_SMC_ID);
	iowrite8(T2SMC_GET_KEY_TYPE_CMD, t2->iomem + T2SMC_IOMEM_KEY_CMD);

	if (iomem_wait_read(t2))
		return -EIO;

	err = ioread8(t2->iomem + T2SMC_IOMEM_KEY_CMD);
	if (err != 0) {
		pr_debug("get_key_type_mmio(%.4s) failed: %u\n", key, err);
		return -EIO;
	}

	info->len   = ioread8(t2->iomem + T2SMC_IOMEM_KEY_TYPE_DATA_LEN);
	type = ioread32(t2->iomem + T2SMC_IOMEM_KEY_TYPE_CODE);
	memcpy(info->type, &type, sizeof(type));
	info->flags = ioread8(t2->iomem + T2SMC_IOMEM_KEY_TYPE_FLAGS);

	pr_debug("get_key_type_mmio(%.4s): len=%u type=%.4s flags=%x\n",
		key, info->len, info->type, info->flags);
	return 0;
}

/* -- High-level SMC access (mutex protected) -- */
static int read_smc(struct t2smc_device *t2, const char *key,
		     u8 *buffer, u8 len)
{
	return iomem_read_smc(t2, T2SMC_READ_CMD, key, buffer, len);
}

static int write_smc(struct t2smc_device *t2, const char *key,
		      const u8 *buffer, u8 len)
{
	return iomem_write_smc(t2, T2SMC_WRITE_CMD, key, buffer, len);
}

static int get_smc_key_by_index(struct t2smc_device *t2,
				 unsigned int index, char *key)
{
	__be32 be = cpu_to_be32(index);
	return iomem_read_smc(t2, T2SMC_GET_KEY_BY_INDEX_CMD,
			      (const char *)&be, (u8 *)key, 4);
}

/* -- Key cache -- */
static struct t2smc_entry *t2smc_get_entry_by_index(struct t2smc_device *t2,
						     int index)
{
	struct t2smc_entry *cache = &t2->cache[index];
	char key[4];
	int ret;

	if (cache->valid)
		return cache;

	mutex_lock(&t2->mutex);
	if (cache->valid)
		goto out;

	ret = get_smc_key_by_index(t2, index, key);
	if (ret)
		goto out;
	memcpy(cache->key, key, 4);

	ret = iomem_get_key_info(t2, key, cache);
	if (ret)
		goto out;
	cache->valid = true;

out:
	mutex_unlock(&t2->mutex);
	if (ret)
		return ERR_PTR(ret);
	return cache;
}

static int t2smc_get_lower_bound(struct t2smc_device *t2,
				  unsigned int *lo, const char *key)
{
	int begin = 0, end = t2->key_count;

	while (begin != end) {
		int middle = begin + (end - begin) / 2;
		struct t2smc_entry *entry = t2smc_get_entry_by_index(t2, middle);
		if (IS_ERR(entry)) {
			*lo = 0;
			return PTR_ERR(entry);
		}
		if (strcmp(entry->key, key) < 0)
			begin = middle + 1;
		else
			end = middle;
	}
	*lo = begin;
	return 0;
}

static int t2smc_get_upper_bound(struct t2smc_device *t2,
				  unsigned int *hi, const char *key)
{
	int begin = 0, end = t2->key_count;

	while (begin != end) {
		int middle = begin + (end - begin) / 2;
		struct t2smc_entry *entry = t2smc_get_entry_by_index(t2, middle);
		if (IS_ERR(entry)) {
			*hi = t2->key_count;
			return PTR_ERR(entry);
		}
		if (strcmp(key, entry->key) < 0)
			end = middle;
		else
			begin = middle + 1;
	}
	*hi = begin;
	return 0;
}

static struct t2smc_entry *t2smc_get_entry_by_key(struct t2smc_device *t2,
						    const char *key)
{
	int begin, end, ret;

	ret = t2smc_get_lower_bound(t2, &begin, key);
	if (ret)
		return ERR_PTR(ret);
	ret = t2smc_get_upper_bound(t2, &end, key);
	if (ret)
		return ERR_PTR(ret);
	if (end == begin)
		return ERR_PTR(-ENOENT);
	if (end - begin != 1)
		return ERR_PTR(-EUCLEAN);

	return t2smc_get_entry_by_index(t2, begin);
}

static int t2smc_read_key(struct t2smc_device *t2,
			   const char *key, u8 *buffer, u8 len)
{
	struct t2smc_entry *entry;
	int ret;

	entry = t2smc_get_entry_by_key(t2, key);
	if (IS_ERR(entry))
		return PTR_ERR(entry);

	if (entry->len != len)
		return -EINVAL;

	mutex_lock(&t2->mutex);
	ret = read_smc(t2, key, buffer, len);
	mutex_unlock(&t2->mutex);
	return ret;
}

static int t2smc_write_key(struct t2smc_device *t2,
			    const char *key, const u8 *buffer, u8 len)
{
	struct t2smc_entry *entry;
	int ret;

	entry = t2smc_get_entry_by_key(t2, key);
	if (IS_ERR(entry))
		return PTR_ERR(entry);

	if (entry->len != len)
		return -EINVAL;

	mutex_lock(&t2->mutex);
	ret = write_smc(t2, key, buffer, len);
	mutex_unlock(&t2->mutex);
	return ret;
}

static int t2smc_has_key(struct t2smc_device *t2,
			  const char *key, bool *present)
{
	struct t2smc_entry *entry;

	entry = t2smc_get_entry_by_key(t2, key);
	if (IS_ERR(entry)) {
		if (PTR_ERR(entry) == -ENOENT) {
			*present = false;
			return 0;
		}
		return PTR_ERR(entry);
	}
	*present = true;
	return 0;
}

static int t2smc_read_be16(struct t2smc_device *t2, const char *key, u16 *val)
{
	__be16 raw;
	int ret;

	ret = t2smc_read_key(t2, key, (u8 *)&raw, sizeof(raw));
	if (!ret)
		*val = be16_to_cpu(raw);
	return ret;
}

static int t2smc_read_be16_signed(struct t2smc_device *t2, const char *key,
				  s16 *val)
{
	u16 raw;
	int ret;

	ret = t2smc_read_be16(t2, key, &raw);
	if (!ret)
		*val = (s16)raw;
	return ret;
}

static int t2smc_hex_digit(char digit)
{
	if (digit >= '0' && digit <= '9')
		return digit - '0';
	if (digit >= 'a' && digit <= 'f')
		return digit - 'a' + 10;
	if (digit >= 'A' && digit <= 'F')
		return digit - 'A' + 10;
	return -EINVAL;
}

/* Fractional bits of a 16-bit "spXY"/"fpXY" fixed point key, or an error */
static int t2smc_fixed_point_bits(const struct t2smc_entry *entry,
				  bool *is_signed)
{
	int integer_bits, fractional_bits;

	if (entry->len != 2 ||
	    (strncmp(entry->type, "sp", 2) && strncmp(entry->type, "fp", 2)))
		return -EOPNOTSUPP;

	*is_signed = entry->type[0] == 's';
	integer_bits = t2smc_hex_digit(entry->type[2]);
	fractional_bits = t2smc_hex_digit(entry->type[3]);
	if (integer_bits < 0 || fractional_bits < 0 ||
	    integer_bits + fractional_bits != (*is_signed ? 15 : 16))
		return -EOPNOTSUPP;
	return fractional_bits;
}

static bool t2smc_is_int_type(const struct t2smc_entry *entry)
{
	return (entry->type[0] == 'u' || entry->type[0] == 's') &&
	       entry->type[1] == 'i' &&
	       (entry->len == 1 || entry->len == 2 || entry->len == 4);
}

/* Keys whose value carries a unit: floats and fixed point numbers */
static bool t2smc_is_sensor_type(const struct t2smc_entry *entry)
{
	bool is_signed;

	if (!strcmp(entry->type, FLOAT_TYPE))
		return entry->len == 4;
	return t2smc_fixed_point_bits(entry, &is_signed) >= 0;
}

/*
 * Read a numeric key and multiply it by @scale. Floats and fixed point
 * numbers keep their fraction until scaled. Integers are big endian.
 */
static int t2smc_read_scaled(struct t2smc_device *t2, const char *key,
			     long scale, long *val)
{
	struct t2smc_entry *entry;
	bool is_signed;
	u8 buf[4];
	u32 raw;
	u64 magnitude;
	int exponent, fractional_bits, i;
	int ret;

	entry = t2smc_get_entry_by_key(t2, key);
	if (IS_ERR(entry))
		return PTR_ERR(entry);

	if (!strcmp(entry->type, FLOAT_TYPE)) {
		ret = t2smc_read_key(t2, key, buf, sizeof(buf));
		if (ret)
			return ret;

		memcpy(&raw, buf, sizeof(raw));
		magnitude = (raw & GENMASK(22, 0)) | BIT(23);
		exponent = ((raw >> 23) & 0xff) - 127 - 23;
		magnitude *= scale;

		if (exponent < -63)
			magnitude = 0;
		else if (exponent < 0)
			magnitude >>= -exponent;
		else if (exponent < 63)
			magnitude <<= exponent;
		else
			magnitude = LONG_MAX;

		*val = min_t(u64, magnitude, LONG_MAX);
		if (raw & BIT(31))
			*val = -*val;
		return 0;
	}

	fractional_bits = t2smc_fixed_point_bits(entry, &is_signed);
	if (fractional_bits >= 0) {
		u16 fixed;

		ret = t2smc_read_be16(t2, key, &fixed);
		if (ret)
			return ret;
		if (is_signed)
			*val = mult_frac((s16)fixed, scale, BIT(fractional_bits));
		else
			*val = mult_frac(fixed, scale, BIT(fractional_bits));
		return 0;
	}

	if (t2smc_is_int_type(entry)) {
		ret = t2smc_read_key(t2, key, buf, entry->len);
		if (ret)
			return ret;

		raw = 0;
		for (i = 0; i < entry->len; i++)
			raw = raw << 8 | buf[i];
		if (entry->type[0] == 's')
			*val = (long)sign_extend32(raw, entry->len * 8 - 1) * scale;
		else
			*val = (long)raw * scale;
		return 0;
	}

	return -EOPNOTSUPP;
}

/* Write an unsigned integer key big endian in the size the SMC reports */
static int t2smc_write_uint(struct t2smc_device *t2, const char *key, u32 val)
{
	struct t2smc_entry *entry;
	u8 buf[4];
	int i;

	entry = t2smc_get_entry_by_key(t2, key);
	if (IS_ERR(entry))
		return PTR_ERR(entry);
	if (!entry->len || entry->len > sizeof(buf))
		return -EINVAL;

	for (i = entry->len - 1; i >= 0; i--, val >>= 8)
		buf[i] = val & 0xff;
	return t2smc_write_key(t2, key, buf, entry->len);
}

/* -- T2 float conversion (fans use IEEE 754 "flt " type on T2) -- */
static inline u32 float_to_u32(u32 d)
{
	u8 sign = (u8)((d >> 31) & 1);
	s32 exp = (s32)((d >> 23) & 0xff) - 0x7f;
	u32 fr = d & ((1u << 23) - 1);

	if (sign || exp < 0)
		return 0;
	return (u32)((1u << exp) + (fr >> (23 - exp)));
}

static inline u32 u32_to_float(u32 d)
{
	u32 dc = d, bc = 0, exp;

	if (!d)
		return 0;
	while (dc >>= 1)
		++bc;
	exp = 0x7f + bc;
	return (u32)((exp << 23) |
		     ((d << (23 - (exp - 0x7f))) & ((1u << 23) - 1)));
}

/* -- Initialization -- */
/* Collect all keys in [@first, @last) that decode to a unit value */
static int t2smc_discover_sensors(struct t2smc_device *t2, const char *first,
				  const char *last, struct t2smc_sensors *set)
{
	unsigned int i, begin, end, n = 0;
	int ret;

	ret = t2smc_get_lower_bound(t2, &begin, first);
	if (ret)
		return ret;
	ret = t2smc_get_lower_bound(t2, &end, last);
	if (ret)
		return ret;
	if (begin == end)
		return 0;

	set->keys = kcalloc(end - begin, sizeof(set->keys[0]), GFP_KERNEL);
	if (!set->keys)
		return -ENOMEM;

	for (i = begin; i < end; i++) {
		struct t2smc_entry *entry = t2smc_get_entry_by_index(t2, i);

		if (IS_ERR(entry) || !t2smc_is_sensor_type(entry))
			continue;
		memcpy(set->keys[n++], entry->key, 4);
	}
	set->count = n;
	return 0;
}

static void t2smc_free_sensors(struct t2smc_device *t2)
{
	struct t2smc_sensors *sets[] = { &t2->temp, &t2->curr, &t2->in,
					 &t2->power };
	int i;

	for (i = 0; i < ARRAY_SIZE(sets); i++) {
		kfree(sets[i]->keys);
		sets[i]->keys = NULL;
		sets[i]->count = 0;
	}
}

/* -- Initialization -- */
static void t2smc_init_charge_limit(struct t2smc_device *t2);

static int t2smc_init_keycache(struct t2smc_device *t2)
{
	unsigned int count;
	__be32 be;
	u8 tmp[1];
	int ret;

	ret = read_smc(t2, KEY_COUNT_KEY, (u8 *)&be, 4);
	if (ret)
		return ret;
	count = be32_to_cpu(be);

	t2->cache = kcalloc(count, sizeof(*t2->cache), GFP_KERNEL);
	if (!t2->cache)
		return -ENOMEM;
	t2->key_count = count;

	/* Discover fan count */
	ret = t2smc_read_key(t2, FANS_COUNT, tmp, 1);
	if (ret)
		return ret;
	t2->fan_count = min_t(unsigned int, tmp[0], T2SMC_MAX_FANS);

	/* Sensors by key prefix: Temperature, current (I), voltage, power */
	ret = t2smc_discover_sensors(t2, "T", "U", &t2->temp);
	if (ret)
		return ret;
	ret = t2smc_discover_sensors(t2, "I", "J", &t2->curr);
	if (ret)
		return ret;
	ret = t2smc_discover_sensors(t2, "V", "W", &t2->in);
	if (ret)
		return ret;
	ret = t2smc_discover_sensors(t2, "P", "Q", &t2->power);
	if (ret)
		return ret;

	ret = t2smc_has_key(t2, T2SMC_CHARGE_LIMIT, &t2->has_chls);
	if (ret)
		return ret;
	t2smc_init_charge_limit(t2);

	dev_info(t2->dev,
		 "initialized: keys=%u fans=%u temps=%u currents=%u voltages=%u power=%u\n",
		 t2->key_count, t2->fan_count, t2->temp.count, t2->curr.count,
		 t2->in.count, t2->power.count);
	return 0;
}

static int t2smc_setup_events(struct platform_device *pdev,
			      struct t2smc_device *t2);

static int t2smc_try_enable_iomem(struct platform_device *pdev,
				  struct t2smc_device *t2)
{
	u8 test_val, ldkn_version;
	int ret;

	pr_debug("Trying to enable MMIO communication\n");
	/* Unmapped by t2smc_devm_cleanup after the IRQ is gone */
	t2->iomem = ioremap(t2->iomem_addr, t2->iomem_size);
	if (!t2->iomem)
		return -ENXIO;

	test_val = ioread8(t2->iomem + T2SMC_IOMEM_KEY_STATUS);
	if (test_val == 0xff) {
		dev_warn(t2->dev, "iomem init failed: status=0xff (is %x)\n",
			 test_val);
		return -ENXIO;
	}

	/* Enable events first so that later commands can wait by interrupt */
	ret = t2smc_setup_events(pdev, t2);
	if (ret)
		return ret;

	/* Verify communication works by reading LDKN key */
	if (iomem_read_smc(t2, T2SMC_READ_CMD, "LDKN", &ldkn_version, 1)) {
		dev_warn(t2->dev, "iomem init failed: LDKN read failed\n");
		return -ENXIO;
	}
	if (ldkn_version < 2) {
		dev_warn(t2->dev, "iomem init failed: LDKN version %u < 2\n",
			 ldkn_version);
		return -ENXIO;
	}

	dev_info(t2->dev, "MMIO interface enabled (LDKN v%u)\n", ldkn_version);
	t2->iomem_ok = true;
	return 0;
}

/* -- ACPI resource walk -- */
static acpi_status t2smc_walk_resources(struct acpi_resource *res, void *data)
{
	struct t2smc_device *t2 = data;

	switch (res->type) {
	case ACPI_RESOURCE_TYPE_IO:
		if (!t2->has_port &&
		    res->data.io.address_length >= T2SMC_PORT_MIN_SIZE) {
			t2->port_base = res->data.io.minimum;
			t2->has_port = true;
		}
		return AE_OK;

	case ACPI_RESOURCE_TYPE_FIXED_MEMORY32:
		if (!t2->iomem_ok) {
			if (res->data.fixed_memory32.address_length <
			    T2SMC_IOMEM_MIN_SIZE) {
				dev_warn(t2->dev,
					 "iomem too small: %u\n",
					 res->data.fixed_memory32.address_length);
				return AE_OK;
			}
			t2->iomem_addr = res->data.fixed_memory32.address;
			t2->iomem_size = res->data.fixed_memory32.address_length;
		}
		return AE_OK;

	case ACPI_RESOURCE_TYPE_END_TAG:
		if (t2->iomem_addr)
			return AE_OK;
		return AE_NOT_FOUND;

	default:
		return AE_OK;
	}
}

/* -- Fan speed r/w -- */
static int t2smc_read_fan(struct t2smc_device *t2, int fan_idx, int option,
			   unsigned int *speed)
{
	struct t2smc_entry *entry;
	char key[5];
	u8 buffer[4];
	int ret;

	scnprintf(key, sizeof(key), fan_speed_fmt[option], fan_idx);
	entry = t2smc_get_entry_by_key(t2, key);
	if (IS_ERR(entry))
		return PTR_ERR(entry);
	if (!strcmp(entry->type, FLOAT_TYPE)) {
		u32 raw;

		ret = t2smc_read_key(t2, key, (u8 *)&raw, 4);
		if (ret)
			return ret;
		*speed = float_to_u32(raw);
	} else {
		ret = t2smc_read_key(t2, key, buffer, 2);
		if (ret)
			return ret;
		*speed = ((buffer[0] << 8 | buffer[1]) >> 2);
	}
	return 0;
}

static int t2smc_write_fan(struct t2smc_device *t2, int fan_idx, int option,
			    unsigned int speed)
{
	struct t2smc_entry *entry;
	char key[5];
	u8 buffer[4];

	scnprintf(key, sizeof(key), fan_speed_fmt[option], fan_idx);
	entry = t2smc_get_entry_by_key(t2, key);
	if (IS_ERR(entry))
		return PTR_ERR(entry);

	if (!strcmp(entry->type, FLOAT_TYPE)) {
		u32 fval = u32_to_float(speed);

		memcpy(buffer, &fval, sizeof(fval));
		return t2smc_write_key(t2, key, buffer, 4);
	} else {
		buffer[0] = (speed >> 6) & 0xff;
		buffer[1] = (speed << 2) & 0xff;
		return t2smc_write_key(t2, key, buffer, 2);
	}
}

static int t2smc_write_fan_manual(struct t2smc_device *t2, int fan_idx,
				   unsigned int manual)
{
	char key[5];
	bool has_fmd;
	u8 buf[2];
	int ret;

	scnprintf(key, sizeof(key), FAN_MANUAL_FMT, fan_idx);
	ret = t2smc_has_key(t2, key, &has_fmd);
	if (ret)
		return ret;

	if (has_fmd) {
		buf[0] = manual ? 1 : 0;
		return t2smc_write_key(t2, key, buf, 1);
	} else {
		unsigned int val;
		ret = t2smc_read_key(t2, FANS_MANUAL, buf, 2);
		if (ret)
			return ret;
		val = (buf[0] << 8 | buf[1]);
		if (manual)
			val |= (0x01 << fan_idx);
		else
			val &= ~(0x01 << fan_idx);
		buf[0] = (val >> 8) & 0xff;
		buf[1] = val & 0xff;
		return t2smc_write_key(t2, FANS_MANUAL, buf, 2);
	}
}

/* -- hwmon interface -- */
#define T2SMC_FAN_OPT_ACTUAL  0
#define T2SMC_FAN_OPT_MIN     1
#define T2SMC_FAN_OPT_MAX     2
#define T2SMC_FAN_OPT_SAFE    3
#define T2SMC_FAN_OPT_TARGET  4

/* Sensor set and scale to hwmon units for a channel type */
static const struct t2smc_sensors *
t2smc_sensors_for(struct t2smc_device *t2, enum hwmon_sensor_types type,
		  long *scale)
{
	switch (type) {
	case hwmon_temp:
		*scale = 1000;		/* millidegree Celsius */
		return &t2->temp;
	case hwmon_curr:
		*scale = 1000;		/* milliampere */
		return &t2->curr;
	case hwmon_in:
		*scale = 1000;		/* millivolt */
		return &t2->in;
	case hwmon_power:
		*scale = 1000000;	/* microwatt */
		return &t2->power;
	default:
		return NULL;
	}
}

static bool t2smc_is_input_attr(enum hwmon_sensor_types type, u32 attr)
{
	return (type == hwmon_temp && attr == hwmon_temp_input) ||
	       (type == hwmon_curr && attr == hwmon_curr_input) ||
	       (type == hwmon_in && attr == hwmon_in_input) ||
	       (type == hwmon_power && attr == hwmon_power_input);
}

static bool t2smc_is_label_attr(enum hwmon_sensor_types type, u32 attr)
{
	return (type == hwmon_temp && attr == hwmon_temp_label) ||
	       (type == hwmon_curr && attr == hwmon_curr_label) ||
	       (type == hwmon_in && attr == hwmon_in_label) ||
	       (type == hwmon_power && attr == hwmon_power_label);
}

static int t2smc_hwmon_read(struct device *dev, enum hwmon_sensor_types type,
			     u32 attr, int channel, long *val)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	const struct t2smc_sensors *set;
	unsigned int speed;
	long scale;
	int ret;

	if (type == hwmon_fan) {
		switch (attr) {
		case hwmon_fan_input:
			ret = t2smc_read_fan(t2, channel, T2SMC_FAN_OPT_ACTUAL, &speed);
			break;
		case hwmon_fan_min:
			ret = t2smc_read_fan(t2, channel, T2SMC_FAN_OPT_MIN, &speed);
			break;
		case hwmon_fan_max:
			ret = t2smc_read_fan(t2, channel, T2SMC_FAN_OPT_MAX, &speed);
			break;
		case hwmon_fan_target:
			ret = t2smc_read_fan(t2, channel, T2SMC_FAN_OPT_TARGET, &speed);
			break;
		default:
			return -EOPNOTSUPP;
		}
		if (ret)
			return ret;
		*val = (long)speed;
		return 0;
	}

	set = t2smc_sensors_for(t2, type, &scale);
	if (!set || !t2smc_is_input_attr(type, attr))
		return -EOPNOTSUPP;
	if (channel >= set->count)
		return -EINVAL;
	return t2smc_read_scaled(t2, set->keys[channel], scale, val);
}

static int t2smc_hwmon_write(struct device *dev, enum hwmon_sensor_types type,
			      u32 attr, int channel, long val)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	unsigned int speed;
	int ret;

	switch (type) {
	case hwmon_fan:
		switch (attr) {
		case hwmon_fan_min:
			if (val < 0)
				return -EINVAL;
			speed = (unsigned int)val;
			return t2smc_write_fan(t2, channel, T2SMC_FAN_OPT_MIN, speed);
		case hwmon_fan_target:
			if (val < 0)
				return -EINVAL;
			/* 0 hands the fan back to the SMC fan control */
			if (!val) {
				clear_bit(channel, &t2->fan_manual);
				return t2smc_write_fan_manual(t2, channel, 0);
			}
			speed = (unsigned int)val;
			/* Enter manual mode before setting target speed */
			ret = t2smc_write_fan_manual(t2, channel, 1);
			if (ret)
				return ret;
			ret = t2smc_write_fan(t2, channel, T2SMC_FAN_OPT_TARGET, speed);
			if (ret)
				return ret;
			/* Remembered to restore manual mode after resume */
			WRITE_ONCE(t2->fan_target[channel], speed);
			set_bit(channel, &t2->fan_manual);
			return 0;
		default:
			return -EOPNOTSUPP;
		}

	default:
		return -EOPNOTSUPP;
	}
}

static umode_t t2smc_hwmon_is_visible(const void *drvdata,
				       enum hwmon_sensor_types type,
				       u32 attr, int channel)
{
	switch (type) {
	case hwmon_temp:
	case hwmon_curr:
	case hwmon_in:
	case hwmon_power:
		return 0444;
	case hwmon_fan:
		switch (attr) {
		case hwmon_fan_min:
		case hwmon_fan_target:
			return 0644;
		default:
			return 0444;
		}
	default:
		return 0;
	}
}


static int t2smc_hwmon_read_string(struct device *dev,
				   enum hwmon_sensor_types type, u32 attr,
				   int channel, const char **str)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	const struct t2smc_sensors *set;
	long scale;

	if (type == hwmon_fan && attr == hwmon_fan_label) {
		if (channel >= t2->fan_count)
			return -EINVAL;
		*str = t2->fan_labels[channel];
		return 0;
	}

	set = t2smc_sensors_for(t2, type, &scale);
	if (!set || !t2smc_is_label_attr(type, attr))
		return -EOPNOTSUPP;
	if (channel >= set->count)
		return -EINVAL;
	*str = set->keys[channel];
	return 0;
}


static const struct hwmon_ops t2smc_hwmon_ops = {
	.is_visible = t2smc_hwmon_is_visible,
	.read       = t2smc_hwmon_read,
	.write      = t2smc_hwmon_write,
	.read_string = t2smc_hwmon_read_string,
};

/* -- Battery charge limit and gauge data as extra hwmon attribute group -- */
static int t2smc_write_chls(struct t2smc_device *t2)
{
	u8 buf[2] = { 0, 0 };

	lockdep_assert_held(&t2->charge_lock);

	if (!t2->charge_limit_suspended && t2->charge_limit < 100)
		buf[0] = t2->charge_limit;
	return t2smc_write_key(t2, T2SMC_CHARGE_LIMIT, buf, sizeof(buf));
}

/*
 * An SMC reset or a T2 panic clears CHLS. Report what the SMC holds, not the
 * limit set last. A suspended limit keeps CHLS at 0 on purpose.
 */
static int t2smc_read_charge_limit(struct t2smc_device *t2, u8 *limit)
{
	u8 buf[2];
	int ret;

	lockdep_assert_held(&t2->charge_lock);

	if (!t2->charge_limit_suspended) {
		ret = t2smc_read_key(t2, T2SMC_CHARGE_LIMIT, buf, sizeof(buf));
		if (ret)
			return ret;
		t2->charge_limit = buf[0] > 0 && buf[0] < 100 ? buf[0] : 100;
	}
	*limit = t2->charge_limit;
	return 0;
}

/*
 * The SMC keeps CHLS and BCLM over reboots and across operating systems.
 * Take the limit over from CHLS and clear a BCLM limit left behind, macOS on
 * T2 Macs has no way to lift it.
 */
static void t2smc_init_charge_limit(struct t2smc_device *t2)
{
	u8 buf[2], bclm, full = 100;
	bool has_bclm;

	t2->charge_limit = 100;
	if (!t2->has_chls)
		return;

	if (!t2smc_read_key(t2, T2SMC_CHARGE_LIMIT, buf, sizeof(buf)) &&
	    buf[0] > 0 && buf[0] < 100)
		t2->charge_limit = buf[0];

	if (t2smc_has_key(t2, T2SMC_CHARGE_LIMIT_LEGACY, &has_bclm) || !has_bclm ||
	    t2smc_read_key(t2, T2SMC_CHARGE_LIMIT_LEGACY, &bclm, 1) || bclm == full)
		return;
	if (t2smc_write_key(t2, T2SMC_CHARGE_LIMIT_LEGACY, &full, 1))
		dev_warn(t2->dev, "failed to clear the BCLM charge limit %u\n", bclm);
	else
		dev_info(t2->dev, "cleared the BCLM charge limit %u\n", bclm);
}

static int t2smc_set_charge_limit(struct t2smc_device *t2, u8 val)
{
	u8 cur;
	int ret;

	if (!t2->has_chls)
		return -ENODEV;
	if (val < 1 || val > 100)
		return -EINVAL;

	/* Udev rules without ACTION rewrite the limit on every change uevent */
	mutex_lock(&t2->charge_lock);
	if (!t2smc_read_charge_limit(t2, &cur) && cur == val) {
		mutex_unlock(&t2->charge_lock);
		return 0;
	}
	t2->charge_limit = val;
	ret = t2smc_write_chls(t2);
	mutex_unlock(&t2->charge_lock);
	if (ret)
		return ret;

	mutex_lock(&t2->battery_lock);
	if (t2->battery)
		power_supply_changed(t2->battery);
	mutex_unlock(&t2->battery_lock);

	return 0;
}

static ssize_t charge_limit_show(struct device *dev,
				  struct device_attribute *attr, char *buf)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	u8 limit;
	int ret;

	mutex_lock(&t2->charge_lock);
	ret = t2smc_read_charge_limit(t2, &limit);
	mutex_unlock(&t2->charge_lock);
	return ret ? ret : sysfs_emit(buf, "%u\n", limit);
}

static ssize_t charge_limit_store(struct device *dev,
				   struct device_attribute *attr,
				   const char *buf, size_t count)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	u8 val;
	int ret;

	if (kstrtou8(buf, 10, &val) < 0)
		return -EINVAL;

	ret = t2smc_set_charge_limit(t2, val);
	if (ret)
		return ret;
	return count;
}

static DEVICE_ATTR(battery_charge_limit, 0644,
		   charge_limit_show, charge_limit_store);

/*
 * Like the gauging mitigation of powerd, user space lifts the limit for a
 * full charge from time to time. The user limit stays as it is.
 */
static ssize_t battery_charge_limit_suspended_show(struct device *dev,
						   struct device_attribute *attr,
						   char *buf)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);

	return sysfs_emit(buf, "%u\n", READ_ONCE(t2->charge_limit_suspended));
}

static ssize_t battery_charge_limit_suspended_store(struct device *dev,
						    struct device_attribute *attr,
						    const char *buf, size_t count)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	bool suspended;
	int ret;

	ret = kstrtobool(buf, &suspended);
	if (ret)
		return ret;

	mutex_lock(&t2->charge_lock);
	t2->charge_limit_suspended = suspended;
	ret = t2smc_write_chls(t2);
	mutex_unlock(&t2->charge_lock);
	return ret ? ret : count;
}
static DEVICE_ATTR_RW(battery_charge_limit_suspended);

/* One value per cell, the SMC keys are big endian ui16 */
static ssize_t t2smc_cell_values_show(struct t2smc_device *t2, const char *fmt,
				      char *buf)
{
	char key[5];
	ssize_t len = 0;
	unsigned int cell;
	u16 val;

	for (cell = 1; cell <= T2SMC_BATTERY_MAX_CELLS; cell++) {
		snprintf(key, sizeof(key), fmt, cell);
		if (t2smc_read_be16(t2, key, &val))
			break;
		len += sysfs_emit_at(buf, len, "%s%u", len ? " " : "", val);
	}
	if (!len)
		return -ENODEV;
	return len + sysfs_emit_at(buf, len, "\n");
}

static ssize_t smc_battery_qmax_mah_show(struct device *dev,
					 struct device_attribute *attr, char *buf)
{
	return t2smc_cell_values_show(dev_get_drvdata(dev), T2SMC_BATTERY_QMAX, buf);
}
static DEVICE_ATTR_RO(smc_battery_qmax_mah);

static ssize_t smc_battery_dod0_show(struct device *dev,
				     struct device_attribute *attr, char *buf)
{
	return t2smc_cell_values_show(dev_get_drvdata(dev), T2SMC_BATTERY_DOD0, buf);
}
static DEVICE_ATTR_RO(smc_battery_dod0);

static struct attribute *t2smc_bclm_attrs[] = {
	&dev_attr_battery_charge_limit.attr,
	&dev_attr_battery_charge_limit_suspended.attr,
	&dev_attr_smc_battery_qmax_mah.attr,
	&dev_attr_smc_battery_dod0.attr,
	NULL,
};

static umode_t t2smc_bclm_is_visible(struct kobject *kobj,
				     struct attribute *attr, int index)
{
	struct t2smc_device *t2 = dev_get_drvdata(kobj_to_dev(kobj));
	bool present;

	if (attr == &dev_attr_smc_battery_qmax_mah.attr)
		return t2smc_has_key(t2, "BQX1", &present) || !present ? 0 : attr->mode;
	if (attr == &dev_attr_smc_battery_dod0.attr)
		return t2smc_has_key(t2, "BQD1", &present) || !present ? 0 : attr->mode;
	return t2->has_chls ? attr->mode : 0;
}

static const struct attribute_group t2smc_bclm_group = {
	.attrs = t2smc_bclm_attrs,
	.is_visible = t2smc_bclm_is_visible,
};

static int t2smc_psy_ext_get(struct power_supply *psy,
			     const struct power_supply_ext *ext,
			     void *data, enum power_supply_property psp,
			     union power_supply_propval *val)
{
	struct t2smc_device *t2 = data;
	u8 limit;
	int ret;

	if (psp != POWER_SUPPLY_PROP_CHARGE_CONTROL_END_THRESHOLD)
		return -EINVAL;

	mutex_lock(&t2->charge_lock);
	ret = t2smc_read_charge_limit(t2, &limit);
	mutex_unlock(&t2->charge_lock);
	if (ret)
		return ret;
	val->intval = limit;
	return 0;
}

static int t2smc_psy_ext_set(struct power_supply *psy,
			     const struct power_supply_ext *ext,
			     void *data, enum power_supply_property psp,
			     const union power_supply_propval *val)
{
	struct t2smc_device *t2 = data;

	if (psp != POWER_SUPPLY_PROP_CHARGE_CONTROL_END_THRESHOLD)
		return -EINVAL;
	if (val->intval < 1 || val->intval > 100)
		return -EINVAL;

	return t2smc_set_charge_limit(t2, val->intval);
}

static int t2smc_psy_ext_is_writeable(struct power_supply *psy,
				      const struct power_supply_ext *ext,
				      void *data, enum power_supply_property psp)
{
	return psp == POWER_SUPPLY_PROP_CHARGE_CONTROL_END_THRESHOLD;
}

static const enum power_supply_property t2smc_psy_ext_props[] = {
	POWER_SUPPLY_PROP_CHARGE_CONTROL_END_THRESHOLD,
};

static const struct power_supply_ext t2smc_psy_ext = {
	.name                  = "t2smc-charge-control",
	.properties            = t2smc_psy_ext_props,
	.num_properties        = ARRAY_SIZE(t2smc_psy_ext_props),
	.get_property          = t2smc_psy_ext_get,
	.set_property          = t2smc_psy_ext_set,
	.property_is_writeable = t2smc_psy_ext_is_writeable,
};

static void t2smc_attach_battery(struct t2smc_device *t2)
{
	struct power_supply *psy;
	int ret;

	if (!t2->has_chls)
		return;

	mutex_lock(&t2->battery_lock);
	if (t2->battery)
		goto out;

	psy = power_supply_get_by_name("BAT0");
	if (!psy)
		goto out;

	ret = power_supply_register_extension(psy, &t2smc_psy_ext, t2->dev, t2);
	if (ret) {
		dev_warn(t2->dev, "charge control extension failed: %d\n", ret);
		power_supply_put(psy);
		goto out;
	}

	t2->battery = psy;
	dev_info(t2->dev, "charge control attached to BAT0\n");
out:
	mutex_unlock(&t2->battery_lock);
}

/* unregister outside battery_lock: the setter runs under the psy extensions_sem */
static void t2smc_detach_battery(void *data)
{
	struct t2smc_device *t2 = data;
	struct power_supply *psy;

	mutex_lock(&t2->battery_lock);
	psy = t2->battery;
	t2->battery = NULL;
	mutex_unlock(&t2->battery_lock);

	if (!psy)
		return;

	power_supply_unregister_extension(psy, &t2smc_psy_ext);
	power_supply_put(psy);
}

enum t2smc_power_attr {
	T2SMC_POWER_EVENT_COUNT,
	T2SMC_POWER_LAST_EVENT_NS,
	T2SMC_POWER_STATUS,
	T2SMC_POWER_CAPACITY,
	T2SMC_POWER_BATTERY_VOLTAGE,
	T2SMC_POWER_BATTERY_CURRENT,
	T2SMC_POWER_BATTERY_POWER,
	T2SMC_POWER_CHARGE_FULL,
	T2SMC_POWER_CHARGE_NOW,
	T2SMC_POWER_CYCLE_COUNT,
	T2SMC_POWER_ADAPTER_VOLTAGE,
	T2SMC_POWER_ADAPTER_CURRENT,
	T2SMC_POWER_ADAPTER_POWER,
	T2SMC_POWER_TIME_TO_EMPTY,
	T2SMC_POWER_TIME_TO_FULL,
	T2SMC_POWER_CHARGE_CURRENT,
	T2SMC_POWER_CHARGE_VOLTAGE,
	T2SMC_POWER_CELL_VOLTAGE_MAX,
};

static ssize_t t2smc_power_show(struct device *dev,
				struct device_attribute *attr, char *buf)
{
	struct sensor_device_attribute *sattr = to_sensor_dev_attr(attr);
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	const char *key;
	long val;
	u16 value16;
	s16 signed16;
	int ret;

	switch (sattr->index) {
	case T2SMC_POWER_EVENT_COUNT:
		return sysfs_emit(buf, "%lld\n",
				  atomic64_read(&t2->power_event_count));
	case T2SMC_POWER_LAST_EVENT_NS:
		return sysfs_emit(buf, "%llu\n",
				  READ_ONCE(t2->power_last_event_ns));
	case T2SMC_POWER_STATUS:
		if (!READ_ONCE(t2->power_status_valid))
			return -ENODATA;
		return sysfs_emit(buf, "%u\n", READ_ONCE(t2->power_status));
	case T2SMC_POWER_CAPACITY:
		ret = t2smc_read_be16(t2, T2SMC_BATTERY_CAPACITY, &value16);
		val = value16;
		break;
	case T2SMC_POWER_BATTERY_VOLTAGE:
		ret = t2smc_read_be16(t2, T2SMC_BATTERY_VOLTAGE, &value16);
		val = (long)value16 * 1000;
		break;
	case T2SMC_POWER_BATTERY_CURRENT:
		ret = t2smc_read_be16_signed(t2, T2SMC_BATTERY_CURRENT,
					     &signed16);
		val = (long)signed16 * 1000;
		break;
	case T2SMC_POWER_BATTERY_POWER:
		ret = t2smc_read_scaled(t2, T2SMC_BATTERY_POWER, 1000000, &val);
		/* B0AP is positive while discharging, B0AC while charging */
		val = -val;
		break;
	case T2SMC_POWER_CHARGE_FULL:
		ret = t2smc_read_be16(t2, T2SMC_BATTERY_FULL, &value16);
		val = (long)value16 * 1000;
		break;
	case T2SMC_POWER_CHARGE_NOW:
		ret = t2smc_read_be16(t2, T2SMC_BATTERY_REMAINING, &value16);
		val = (long)value16 * 1000;
		break;
	case T2SMC_POWER_CYCLE_COUNT:
		ret = t2smc_read_be16(t2, T2SMC_BATTERY_CYCLES, &value16);
		val = value16;
		break;
	case T2SMC_POWER_ADAPTER_VOLTAGE:
		ret = t2smc_read_scaled(t2, T2SMC_ADAPTER_VOLTAGE,
					1000000, &val);
		break;
	case T2SMC_POWER_ADAPTER_CURRENT:
		ret = t2smc_read_scaled(t2, T2SMC_ADAPTER_CURRENT,
					1000000, &val);
		break;
	case T2SMC_POWER_ADAPTER_POWER:
		key = !IS_ERR(t2smc_get_entry_by_key(t2,
						     T2SMC_ADAPTER_POWER_OLD)) ?
			T2SMC_ADAPTER_POWER_OLD : T2SMC_ADAPTER_POWER;
		ret = t2smc_read_scaled(t2, key, 1000000, &val);
		break;
	case T2SMC_POWER_TIME_TO_EMPTY:
	case T2SMC_POWER_TIME_TO_FULL:
		key = sattr->index == T2SMC_POWER_TIME_TO_EMPTY ?
			T2SMC_BATTERY_TIME_TO_EMPTY : T2SMC_BATTERY_TIME_TO_FULL;
		ret = t2smc_read_be16(t2, key, &value16);
		/* Minutes, 0xffff while not (dis)charging */
		if (!ret && value16 == 0xffff)
			return -ENODATA;
		val = (long)value16 * 60;
		break;
	case T2SMC_POWER_CHARGE_CURRENT:
		ret = t2smc_read_scaled(t2, T2SMC_CHARGE_CURRENT, 1000, &val);
		break;
	case T2SMC_POWER_CHARGE_VOLTAGE:
		ret = t2smc_read_scaled(t2, T2SMC_CHARGE_VOLTAGE, 1000, &val);
		break;
	case T2SMC_POWER_CELL_VOLTAGE_MAX:
		ret = t2smc_read_scaled(t2, T2SMC_CELL_VOLTAGE_MAX, 1000, &val);
		break;
	default:
		return -EINVAL;
	}

	if (ret)
		return ret;
	return sysfs_emit(buf, "%ld\n", val);
}

static SENSOR_DEVICE_ATTR_RO(power_event_count, t2smc_power,
			     T2SMC_POWER_EVENT_COUNT);
static SENSOR_DEVICE_ATTR_RO(power_last_event_ns, t2smc_power,
			     T2SMC_POWER_LAST_EVENT_NS);
static SENSOR_DEVICE_ATTR_RO(smc_battery_status, t2smc_power,
			     T2SMC_POWER_STATUS);
static SENSOR_DEVICE_ATTR_RO(smc_battery_capacity_percent, t2smc_power,
			     T2SMC_POWER_CAPACITY);
static SENSOR_DEVICE_ATTR_RO(smc_battery_voltage_uv, t2smc_power,
			     T2SMC_POWER_BATTERY_VOLTAGE);
static SENSOR_DEVICE_ATTR_RO(smc_battery_current_ua, t2smc_power,
			     T2SMC_POWER_BATTERY_CURRENT);
static SENSOR_DEVICE_ATTR_RO(smc_battery_power_uw, t2smc_power,
			     T2SMC_POWER_BATTERY_POWER);
static SENSOR_DEVICE_ATTR_RO(smc_battery_charge_full_uah, t2smc_power,
			     T2SMC_POWER_CHARGE_FULL);
static SENSOR_DEVICE_ATTR_RO(smc_battery_charge_now_uah, t2smc_power,
			     T2SMC_POWER_CHARGE_NOW);
static SENSOR_DEVICE_ATTR_RO(smc_battery_cycle_count, t2smc_power,
			     T2SMC_POWER_CYCLE_COUNT);
static SENSOR_DEVICE_ATTR_RO(smc_adapter_voltage_uv, t2smc_power,
			     T2SMC_POWER_ADAPTER_VOLTAGE);
static SENSOR_DEVICE_ATTR_RO(smc_adapter_current_ua, t2smc_power,
			     T2SMC_POWER_ADAPTER_CURRENT);
static SENSOR_DEVICE_ATTR_RO(smc_adapter_power_uw, t2smc_power,
			     T2SMC_POWER_ADAPTER_POWER);
static SENSOR_DEVICE_ATTR_RO(smc_battery_time_to_empty_s, t2smc_power,
			     T2SMC_POWER_TIME_TO_EMPTY);
static SENSOR_DEVICE_ATTR_RO(smc_battery_time_to_full_s, t2smc_power,
			     T2SMC_POWER_TIME_TO_FULL);
static SENSOR_DEVICE_ATTR_RO(smc_battery_charge_current_ua, t2smc_power,
			     T2SMC_POWER_CHARGE_CURRENT);
static SENSOR_DEVICE_ATTR_RO(smc_battery_charge_voltage_uv, t2smc_power,
			     T2SMC_POWER_CHARGE_VOLTAGE);
static SENSOR_DEVICE_ATTR_RO(smc_battery_cell_voltage_max_uv, t2smc_power,
			     T2SMC_POWER_CELL_VOLTAGE_MAX);

static struct attribute *t2smc_power_attrs[] = {
	&sensor_dev_attr_power_event_count.dev_attr.attr,
	&sensor_dev_attr_power_last_event_ns.dev_attr.attr,
	&sensor_dev_attr_smc_battery_status.dev_attr.attr,
	&sensor_dev_attr_smc_battery_capacity_percent.dev_attr.attr,
	&sensor_dev_attr_smc_battery_voltage_uv.dev_attr.attr,
	&sensor_dev_attr_smc_battery_current_ua.dev_attr.attr,
	&sensor_dev_attr_smc_battery_power_uw.dev_attr.attr,
	&sensor_dev_attr_smc_battery_charge_full_uah.dev_attr.attr,
	&sensor_dev_attr_smc_battery_charge_now_uah.dev_attr.attr,
	&sensor_dev_attr_smc_battery_cycle_count.dev_attr.attr,
	&sensor_dev_attr_smc_adapter_voltage_uv.dev_attr.attr,
	&sensor_dev_attr_smc_adapter_current_ua.dev_attr.attr,
	&sensor_dev_attr_smc_adapter_power_uw.dev_attr.attr,
	&sensor_dev_attr_smc_battery_time_to_empty_s.dev_attr.attr,
	&sensor_dev_attr_smc_battery_time_to_full_s.dev_attr.attr,
	&sensor_dev_attr_smc_battery_charge_current_ua.dev_attr.attr,
	&sensor_dev_attr_smc_battery_charge_voltage_uv.dev_attr.attr,
	&sensor_dev_attr_smc_battery_cell_voltage_max_uv.dev_attr.attr,
	NULL,
};

static const struct attribute_group t2smc_power_group = {
	.attrs = t2smc_power_attrs,
};

/* Thermal levels the SMC publishes in its I/O window, see event 0x54 */
static ssize_t t2smc_thermal_level_show(struct device *dev,
					struct device_attribute *attr,
					char *buf)
{
	struct sensor_device_attribute *sattr = to_sensor_dev_attr(attr);
	struct t2smc_device *t2 = dev_get_drvdata(dev);

	return sysfs_emit(buf, "%u\n", inb(t2->port_base + sattr->index));
}

static SENSOR_DEVICE_ATTR(smc_thermal_level_cpu, 0444,
			  t2smc_thermal_level_show, NULL,
			  T2SMC_PORT_THERMAL_CPU);
static SENSOR_DEVICE_ATTR(smc_thermal_level_io, 0444,
			  t2smc_thermal_level_show, NULL,
			  T2SMC_PORT_THERMAL_IO);
static SENSOR_DEVICE_ATTR(smc_thermal_level_gpu, 0444,
			  t2smc_thermal_level_show, NULL,
			  T2SMC_PORT_THERMAL_GPU);

/* Raw PLimit words the SMC publishes in its I/O window, see event 0x80 */
static ssize_t smc_plimit_raw_show(struct device *dev,
				   struct device_attribute *attr, char *buf)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);

	return sysfs_emit(buf, "%08x %02x\n",
			  inl(t2->port_base + T2SMC_PORT_PLIMIT),
			  inb(t2->port_base + T2SMC_PORT_PLIMIT_GPU));
}
static DEVICE_ATTR_RO(smc_plimit_raw);

static const char *t2smc_gpu_steps_key(struct t2smc_device *t2, int gpu)
{
	bool he2n;

	if (t2smc_has_key(t2, T2SMC_PLIMIT_EGPU3_STEPS, &he2n))
		he2n = false;
	if (gpu == T2SMC_DGPU)
		return he2n ? T2SMC_PLIMIT_EGPU3_STEPS : T2SMC_PLIMIT_EGPU_STEPS;
	return he2n ? T2SMC_PLIMIT_EGPU_STEPS : T2SMC_PLIMIT_IGPU_STEPS;
}

/*
 * Step counts of the GPUs. macOS reports them only for GPUs it drives, and
 * only user space knows which GPUs are there and their ranges.
 */
static ssize_t t2smc_gpu_steps_show(struct device *dev,
				    struct device_attribute *attr, char *buf)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	int gpu = to_sensor_dev_attr(attr)->index;
	u8 count;
	int ret;

	ret = t2smc_read_key(t2, t2smc_gpu_steps_key(t2, gpu), &count, 1);
	if (ret)
		return ret;
	return sysfs_emit(buf, "%u\n", count);
}

static ssize_t t2smc_gpu_steps_store(struct device *dev,
				     struct device_attribute *attr,
				     const char *buf, size_t len)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	int gpu = to_sensor_dev_attr(attr)->index;
	u8 count;
	int ret;

	ret = kstrtou8(buf, 0, &count);
	if (ret)
		return ret;
	if (!count || count > 0x7f)
		return -EINVAL;

	ret = t2smc_write_key(t2, t2smc_gpu_steps_key(t2, gpu), &count, 1);
	if (ret)
		return ret;
	WRITE_ONCE(t2->gpu_steps[gpu], count);
	return len;
}

static SENSOR_DEVICE_ATTR(smc_dgpu_steps, 0644, t2smc_gpu_steps_show,
			  t2smc_gpu_steps_store, T2SMC_DGPU);
static SENSOR_DEVICE_ATTR(smc_igpu_steps, 0644, t2smc_gpu_steps_show,
			  t2smc_gpu_steps_store, T2SMC_IGPU);

static int t2smc_write_cpu_floor(struct t2smc_device *t2, const u8 *floor)
{
	int ret;

	ret = t2smc_write_key(t2, T2SMC_CPU_TURBO_STATES, &floor[0], 1);
	if (ret)
		return ret;
	return t2smc_write_key(t2, T2SMC_CPU_FLOOR_STATES, &floor[1], 1);
}

/*
 * CPU P-state range as macOS reports it. Only user
 * space knows CPUFloor of the board and the P-state table.
 */
static ssize_t smc_cpu_floor_show(struct device *dev,
				  struct device_attribute *attr, char *buf)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	u8 turbo, floor;
	int ret;

	ret = t2smc_read_key(t2, T2SMC_CPU_TURBO_STATES, &turbo, 1);
	if (!ret)
		ret = t2smc_read_key(t2, T2SMC_CPU_FLOOR_STATES, &floor, 1);
	if (ret)
		return ret;
	return sysfs_emit(buf, "%u %u\n", turbo, floor);
}

static ssize_t smc_cpu_floor_store(struct device *dev,
				   struct device_attribute *attr,
				   const char *buf, size_t len)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	u8 floor[2];
	int ret;

	if (sscanf(buf, "%hhu %hhu", &floor[0], &floor[1]) != 2)
		return -EINVAL;

	ret = t2smc_write_cpu_floor(t2, floor);
	if (ret)
		return ret;
	memcpy(t2->cpu_floor, floor, sizeof(floor));
	WRITE_ONCE(t2->cpu_floor_set, true);
	return len;
}
static DEVICE_ATTR_RW(smc_cpu_floor);

static ssize_t smc_plimit_version_show(struct device *dev,
				       struct device_attribute *attr, char *buf)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);

	return sysfs_emit(buf, "%u\n", t2->plimit_version);
}
static DEVICE_ATTR_RO(smc_plimit_version);

static struct attribute *t2smc_thermal_attrs[] = {
	&sensor_dev_attr_smc_thermal_level_cpu.dev_attr.attr,
	&sensor_dev_attr_smc_thermal_level_io.dev_attr.attr,
	&sensor_dev_attr_smc_thermal_level_gpu.dev_attr.attr,
	&dev_attr_smc_plimit_raw.attr,
	&sensor_dev_attr_smc_dgpu_steps.dev_attr.attr,
	&sensor_dev_attr_smc_igpu_steps.dev_attr.attr,
	&dev_attr_smc_plimit_version.attr,
	&dev_attr_smc_cpu_floor.attr,
	NULL,
};

/* Hide the step counts whose SMC keys this Mac does not have */
static umode_t t2smc_thermal_is_visible(struct kobject *kobj,
					struct attribute *attr, int index)
{
	struct t2smc_device *t2 = dev_get_drvdata(kobj_to_dev(kobj));
	bool present;
	int gpu;

	if (attr == &dev_attr_smc_cpu_floor.attr) {
		if (t2smc_has_key(t2, T2SMC_CPU_TURBO_STATES, &present) || !present ||
		    t2smc_has_key(t2, T2SMC_CPU_FLOOR_STATES, &present) || !present)
			return 0;
		return attr->mode;
	}

	if (attr == &sensor_dev_attr_smc_dgpu_steps.dev_attr.attr)
		gpu = T2SMC_DGPU;
	else if (attr == &sensor_dev_attr_smc_igpu_steps.dev_attr.attr)
		gpu = T2SMC_IGPU;
	else
		return attr->mode;

	if (t2smc_has_key(t2, t2smc_gpu_steps_key(t2, gpu), &present) || !present)
		return 0;
	return attr->mode;
}

static const struct attribute_group t2smc_thermal_group = {
	.attrs = t2smc_thermal_attrs,
	.is_visible = t2smc_thermal_is_visible,
};

static const struct attribute_group *t2smc_hwmon_groups[] = {
	&t2smc_bclm_group,
	&t2smc_power_group,
	&t2smc_thermal_group,
	NULL,
};

static void t2smc_power_event_work(struct work_struct *work)
{
	struct t2smc_device *t2 = container_of(work, struct t2smc_device,
					       power_event_work);
	u8 status;

	if (t2smc_read_key(t2, T2SMC_BATTERY_STATUS, &status, sizeof(status)))
		return;

	WRITE_ONCE(t2->power_status, status);
	WRITE_ONCE(t2->power_status_valid, true);

	t2smc_attach_battery(t2);

	WRITE_ONCE(t2->power_last_event_ns, ktime_get_boottime_ns());
	atomic64_inc(&t2->power_event_count);
	if (t2->hwmon_dev)
		sysfs_notify(&t2->hwmon_dev->kobj, NULL, "power_event_count");
}

static int t2smc_power_supply_event(struct notifier_block *nb,
				    unsigned long event, void *data)
{
	struct t2smc_device *t2 = container_of(nb, struct t2smc_device,
					       power_supply_nb);
	struct power_supply *psy = data;

	if (event != PSY_EVENT_PROP_CHANGED || !psy || !psy->desc)
		return NOTIFY_DONE;
	if (psy->desc->type != POWER_SUPPLY_TYPE_BATTERY &&
	    psy->desc->type != POWER_SUPPLY_TYPE_MAINS)
		return NOTIFY_DONE;
	if (strncmp(psy->desc->name, "BAT", 3) &&
	    strncmp(psy->desc->name, "ADP", 3))
		return NOTIFY_DONE;

	schedule_work(&t2->power_event_work);
	return NOTIFY_OK;
}

/* -- RTC (48-bit 32768 Hz counter + offset) -- */
static int t2smc_read_rtc_key(struct t2smc_device *t2, const char *key, u64 *val)
{
	u8 buf[T2SMC_RTC_BYTES];
	int ret;

	ret = t2smc_read_key(t2, key, buf, T2SMC_RTC_BYTES);
	if (ret)
		return ret;

	*val = 0;
	memcpy(val, buf, T2SMC_RTC_BYTES);
	return 0;
}

static int t2smc_write_rtc_key(struct t2smc_device *t2, const char *key, u64 val)
{
	u8 buf[T2SMC_RTC_BYTES];

	memcpy(buf, &val, T2SMC_RTC_BYTES);
	return t2smc_write_key(t2, key, buf, T2SMC_RTC_BYTES);
}

/*
 * CLKL is Apple's asynchronous snapshot path for the PMU up-counter.  A
 * six-byte all-ones write requests a latch, after which the result is polled.
 * The complete exchange must be atomic with respect to other SMC commands.
 */
static int t2smc_read_latched_counter(struct t2smc_device *t2, u64 *val)
{
	u8 request[T2SMC_RTC_BYTES];
	u8 response[T2SMC_RTC_BYTES];
	u64 ticks;
	int outer, retry, ret = -EIO;

	if (!t2->has_rtc_latch)
		return -EOPNOTSUPP;

	memset(request, 0xff, sizeof(request));

	mutex_lock(&t2->mutex);
	for (outer = 0; outer < T2SMC_RTC_LATCH_OUTER_RETRIES; outer++) {
		for (retry = 0; retry < T2SMC_RTC_LATCH_WRITE_RETRIES; retry++) {
			ret = write_smc(t2, T2SMC_RTC_LATCH, request,
					T2SMC_RTC_BYTES);
			if (!ret)
				break;
			usleep_range(1000, 2000);
		}
		if (ret)
			continue;

		msleep(20);

		for (retry = 0; retry < T2SMC_RTC_LATCH_READ_RETRIES; retry++) {
			ret = read_smc(t2, T2SMC_RTC_LATCH, response,
				       T2SMC_RTC_BYTES);
			if (!ret) {
				ticks = 0;
				memcpy(&ticks, response, T2SMC_RTC_BYTES);
				if ((ticks & 0x7fff) == 5 ||
				    (ticks & 0x7fff) == 6) {
					ret = -EAGAIN;
					break;
				}
				if (ticks != T2SMC_RTC_MASK) {
					*val = ticks;
					goto out;
				}
				ret = -EAGAIN;
			}
			usleep_range(1000, 2000);
		}
	}

out:
	mutex_unlock(&t2->mutex);
	return ret;
}

static int t2smc_read_rtc_counter(struct t2smc_device *t2, u64 *val)
{
	if (t2->has_rtc_latch)
		return t2smc_read_latched_counter(t2, val);
	return t2smc_read_rtc_key(t2, T2SMC_RTC_COUNTER, val);
}

static int t2smc_rtc_read_time(struct device *dev, struct rtc_time *tm)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	s64 ticks;
	u64 ctr;
	time64_t now;
	int ret;

	ret = t2smc_read_rtc_counter(t2, &ctr);
	if (ret)
		return ret;

	ticks = (s64)ctr + READ_ONCE(t2->rtc_offset);
	now = div_s64(ticks, t2->rtc_rate);
	rtc_time64_to_tm(now, tm);
	return 0;
}

/* Like macOS, only write CLKO when the offset changed. */
static int t2smc_rtc_write_offset(struct t2smc_device *t2, s64 off,
				  s64 tolerance)
{
	int ret;

	off = sign_extend64((u64)off & T2SMC_RTC_MASK, T2SMC_RTC_BITS - 1);
	if (abs(off - READ_ONCE(t2->rtc_offset)) < tolerance)
		return 0;

	ret = t2smc_write_rtc_key(t2, T2SMC_RTC_OFFSET, (u64)off);
	if (!ret)
		WRITE_ONCE(t2->rtc_offset, off);
	return ret;
}

static int t2smc_rtc_set_time(struct device *dev, struct rtc_time *tm)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	u64 ctr;
	int ret;

	ret = t2smc_read_rtc_counter(t2, &ctr);
	if (ret)
		return ret;

	/*
	 * The time comes in whole seconds and the counter keeps running
	 * until it is latched, so tolerate a quarter second.
	 */
	return t2smc_rtc_write_offset(t2,
			rtc_tm_to_time64(tm) * (s64)t2->rtc_rate - (s64)ctr,
			t2->rtc_rate / 4);
}

/*
 * Synchronize CLKO with the system clock. macOS does this automatically
 * before power transitions, and the T2 derives its own clock from the SMC
 * RTC while the host sleeps or is off. The kernel's NTP sync never reaches
 * this RTC on x86 because it stops at the legacy CMOS clock.
 */
static void t2smc_rtc_sync_from_system(struct t2smc_device *t2)
{
	struct timespec64 now;
	s64 ticks;
	u64 ctr;
	int ret;

	if (!t2->rtc_dev)
		return;

	ktime_get_real_ts64(&now);
	ret = t2smc_read_rtc_counter(t2, &ctr);
	if (!ret) {
		ticks = now.tv_sec * (s64)t2->rtc_rate +
			div_u64((u64)now.tv_nsec * t2->rtc_rate, NSEC_PER_SEC);
		ret = t2smc_rtc_write_offset(t2, ticks - (s64)ctr,
				div_u64((u64)t2->rtc_rate *
					T2SMC_RTC_SYNC_TOLERANCE_MS,
					MSEC_PER_SEC));
	}
	if (ret)
		dev_warn(t2->dev, "failed to synchronize the RTC: %d\n", ret);
}

static const struct rtc_class_ops t2smc_rtc_ops = {
	.read_time = t2smc_rtc_read_time,
	.set_time = t2smc_rtc_set_time,
};

static int t2smc_register_rtc(struct t2smc_device *t2)
{
	struct device *dev = t2->dev;
	struct t2smc_entry *entry;
	bool has_counter, has_offset;
	u64 raw;
	u32 rate;
	int ret;

	ret = t2smc_has_key(t2, T2SMC_RTC_COUNTER, &has_counter);
	if (ret)
		return ret;
	ret = t2smc_has_key(t2, T2SMC_RTC_OFFSET, &has_offset);
	if (ret)
		return ret;

	if (!has_counter || !has_offset) {
		dev_info(t2->dev, "RTC keys not present, skipping RTC\n");
		return 0;
	}

	ret = t2smc_read_rtc_key(t2, T2SMC_RTC_OFFSET, &raw);
	if (ret)
		return dev_err_probe(dev, ret, "failed to read CLKO\n");
	t2->rtc_offset = sign_extend64(raw, T2SMC_RTC_BITS - 1);

	t2->rtc_rate = T2SMC_RTC_DEFAULT_RATE;
	entry = t2smc_get_entry_by_key(t2, T2SMC_RTC_RATE);
	if (!IS_ERR(entry) && entry->len == sizeof(rate)) {
		ret = t2smc_read_key(t2, T2SMC_RTC_RATE, (u8 *)&rate,
				     sizeof(rate));
		if (!ret && rate != U32_MAX && rate != 0)
			t2->rtc_rate = rate;
	} else if (IS_ERR(entry) && PTR_ERR(entry) != -ENOENT) {
		return PTR_ERR(entry);
	}

	/* Apple enables the asynchronous path only if the initial read works. */
	entry = t2smc_get_entry_by_key(t2, T2SMC_RTC_LATCH);
	if (!IS_ERR(entry) && entry->len == T2SMC_RTC_BYTES &&
	    !t2smc_read_rtc_key(t2, T2SMC_RTC_LATCH, &raw))
		t2->has_rtc_latch = true;
	else if (IS_ERR(entry) && PTR_ERR(entry) != -ENOENT)
		return PTR_ERR(entry);

	t2->rtc_dev = devm_rtc_allocate_device(dev);
	if (IS_ERR(t2->rtc_dev))
		return PTR_ERR(t2->rtc_dev);

	t2->rtc_dev->ops = &t2smc_rtc_ops;
	t2->rtc_dev->range_min =
		div_s64(-(s64)BIT_ULL(T2SMC_RTC_BITS - 1), t2->rtc_rate);
	t2->rtc_dev->range_max =
		div_s64(BIT_ULL(T2SMC_RTC_BITS - 1) - 1, t2->rtc_rate);

	ret = devm_rtc_register_device(t2->rtc_dev);
	if (ret)
		return ret;

	dev_info(t2->dev, "RTC registered (rate=%u Hz, CLKL=%s)\n",
		 t2->rtc_rate, t2->has_rtc_latch ? "enabled" : "unavailable");
	return 0;
}

/* FnID holds the fan name after a four byte header */
static void t2smc_read_fan_labels(struct t2smc_device *t2)
{
	struct t2smc_entry *entry;
	char key[5];
	u8 buf[4 + T2SMC_FAN_LABEL_LEN];
	size_t len;
	int i;

	for (i = 0; i < t2->fan_count; i++) {
		scnprintf(key, sizeof(key), "F%dID", i);
		entry = t2smc_get_entry_by_key(t2, key);
		if (IS_ERR(entry) || entry->len != sizeof(buf) ||
		    t2smc_read_key(t2, key, buf, sizeof(buf)))
			continue;
		memcpy(t2->fan_labels[i], buf + 4, T2SMC_FAN_LABEL_LEN);
		t2->fan_labels[i][T2SMC_FAN_LABEL_LEN] = '\0';
		len = strlen(t2->fan_labels[i]);
		while (len && isspace(t2->fan_labels[i][len - 1]))
			t2->fan_labels[i][--len] = '\0';
	}
}

static struct hwmon_channel_info *
t2smc_channel_info(struct device *dev, enum hwmon_sensor_types type,
		   unsigned int count, u32 config)
{
	struct hwmon_channel_info *info;
	u32 *channel_config;
	int i;

	info = devm_kzalloc(dev, sizeof(*info), GFP_KERNEL);
	channel_config = devm_kcalloc(dev, count + 1, sizeof(u32), GFP_KERNEL);
	if (!info || !channel_config)
		return NULL;

	for (i = 0; i < count; i++)
		channel_config[i] = config;
	info->type = type;
	info->config = channel_config;
	return info;
}

/* Register hwmon device with fan and sensor channels and the extra groups */
static int t2smc_register_hwmon(struct t2smc_device *t2)
{
	const struct {
		enum hwmon_sensor_types type;
		const struct t2smc_sensors *set;
		u32 config;
	} sensors[] = {
		{ hwmon_temp, &t2->temp, HWMON_T_INPUT | HWMON_T_LABEL },
		{ hwmon_curr, &t2->curr, HWMON_C_INPUT | HWMON_C_LABEL },
		{ hwmon_in, &t2->in, HWMON_I_INPUT | HWMON_I_LABEL },
		{ hwmon_power, &t2->power, HWMON_P_INPUT | HWMON_P_LABEL },
	};
	struct device *dev = t2->dev;
	struct device *hwmon_dev;
	const struct hwmon_channel_info **info;
	struct hwmon_channel_info *fan_info;
	struct hwmon_chip_info *chip_info;
	u32 *fan_config;
	int i, idx = 0;

	t2smc_read_fan_labels(t2);

	fan_config = devm_kcalloc(dev, t2->fan_count + 1, sizeof(u32), GFP_KERNEL);
	fan_info  = devm_kzalloc(dev, sizeof(*fan_info), GFP_KERNEL);
	chip_info = devm_kzalloc(dev, sizeof(*chip_info), GFP_KERNEL);
	/* fan + sensor types + sentinel */
	info = devm_kcalloc(dev, ARRAY_SIZE(sensors) + 2, sizeof(*info),
			    GFP_KERNEL);
	if (!fan_config || !fan_info || !chip_info || !info)
		return -ENOMEM;

	for (i = 0; i < t2->fan_count; i++) {
		fan_config[i] = HWMON_F_INPUT | HWMON_F_MIN |
				HWMON_F_MAX | HWMON_F_TARGET;
		if (t2->fan_labels[i][0])
			fan_config[i] |= HWMON_F_LABEL;
	}
	fan_info->type   = hwmon_fan;
	fan_info->config = fan_config;
	info[idx++] = fan_info;

	for (i = 0; i < ARRAY_SIZE(sensors); i++) {
		if (!sensors[i].set->count)
			continue;
		info[idx] = t2smc_channel_info(dev, sensors[i].type,
					       sensors[i].set->count,
					       sensors[i].config);
		if (!info[idx++])
			return -ENOMEM;
	}
	info[idx] = NULL;

	chip_info->info = info;
	chip_info->ops = &t2smc_hwmon_ops;

	hwmon_dev = devm_hwmon_device_register_with_info(dev, "t2smc", t2,
							  chip_info,
							  t2smc_hwmon_groups);
	if (IS_ERR(hwmon_dev))
		return PTR_ERR(hwmon_dev);
	t2->hwmon_dev = hwmon_dev;

	return 0;
}

/* Re-enter manual mode for fans that had a target set before suspend */
static void t2smc_restore_fans(struct t2smc_device *t2)
{
	int i, ret;

	for_each_set_bit(i, &t2->fan_manual, t2->fan_count) {
		ret = t2smc_write_fan_manual(t2, i, 1);
		if (!ret)
			ret = t2smc_write_fan(t2, i, T2SMC_FAN_OPT_TARGET,
					      READ_ONCE(t2->fan_target[i]));
		if (ret) {
			dev_warn(t2->dev, "fan %d: failed to restore manual mode: %d\n",
				 i, ret);
			clear_bit(i, &t2->fan_manual);
		}
	}
}

/* -- Watchdog (OSWD, or NATi/NATJ on older firmware) -- */
/*
 * Longer timeouts are emulated by the watchdog core, which pings the SMC
 * before the hardware limit runs out, so the SMC never gets more than that.
 */
static int t2smc_wdt_ping(struct watchdog_device *wdd)
{
	struct t2smc_device *t2 = watchdog_get_drvdata(wdd);

	return t2smc_write_uint(t2, t2->has_oswd ? T2SMC_WDT_TIMER :
				T2SMC_WDT_LEGACY_TIMER,
				min(wdd->timeout, T2SMC_WDT_HW_MAX_TIMEOUT));
}

static int t2smc_wdt_arm(struct t2smc_device *t2, struct watchdog_device *wdd)
{
	int ret;

	ret = t2smc_wdt_ping(wdd);
	if (ret || t2->has_oswd)
		return ret;
	/* NATJ selects what happens when NATi expires, 2 forces a restart */
	return t2smc_write_uint(t2, T2SMC_WDT_LEGACY_MODE,
				T2SMC_WDT_LEGACY_RESTART);
}

static int t2smc_wdt_disarm(struct t2smc_device *t2)
{
	int ret;

	if (t2->has_oswd)
		return t2smc_write_uint(t2, T2SMC_WDT_TIMER, 0);

	ret = t2smc_write_uint(t2, T2SMC_WDT_LEGACY_MODE, 0);
	if (ret)
		return ret;
	return t2smc_write_uint(t2, T2SMC_WDT_LEGACY_TIMER, 0);
}

/* Start, stop and timeout changes are rare, so each one is logged */
static int t2smc_wdt_start(struct watchdog_device *wdd)
{
	struct t2smc_device *t2 = watchdog_get_drvdata(wdd);
	int ret;

	ret = t2smc_wdt_arm(t2, wdd);
	if (ret)
		dev_warn(t2->dev, "failed to start watchdog: %d\n", ret);
	else
		dev_info(t2->dev, "watchdog started (timeout=%us)\n",
			 wdd->timeout);
	return ret;
}

static int t2smc_wdt_stop(struct watchdog_device *wdd)
{
	struct t2smc_device *t2 = watchdog_get_drvdata(wdd);
	int ret;

	ret = t2smc_wdt_disarm(t2);
	if (ret)
		dev_warn(t2->dev, "failed to stop watchdog: %d\n", ret);
	else
		dev_info(t2->dev, "watchdog stopped\n");
	return ret;
}

static int t2smc_wdt_set_timeout(struct watchdog_device *wdd,
				 unsigned int timeout)
{
	struct t2smc_device *t2 = watchdog_get_drvdata(wdd);

	dev_info(t2->dev, "watchdog timeout set to %us\n", timeout);
	wdd->timeout = timeout;
	if (watchdog_active(wdd))
		return t2smc_wdt_ping(wdd);
	return 0;
}

static const struct watchdog_ops t2smc_wdt_ops = {
	.owner       = THIS_MODULE,
	.start       = t2smc_wdt_start,
	.stop        = t2smc_wdt_stop,
	.ping        = t2smc_wdt_ping,
	.set_timeout = t2smc_wdt_set_timeout,
};

static const struct watchdog_info t2smc_wdt_info = {
	.options  = WDIOF_SETTIMEOUT | WDIOF_KEEPALIVEPING | WDIOF_MAGICCLOSE,
	.identity = "t2smc watchdog",
};

static int t2smc_register_watchdog(struct t2smc_device *t2)
{
	struct watchdog_device *wdd = &t2->wdd;
	bool has_timer, has_mode;
	int ret;

	ret = t2smc_has_key(t2, T2SMC_WDT_TIMER, &t2->has_oswd);
	if (ret)
		return ret;
	if (!t2->has_oswd) {
		ret = t2smc_has_key(t2, T2SMC_WDT_LEGACY_TIMER, &has_timer);
		if (ret)
			return ret;
		ret = t2smc_has_key(t2, T2SMC_WDT_LEGACY_MODE, &has_mode);
		if (ret)
			return ret;
		if (!has_timer || !has_mode) {
			dev_info(t2->dev, "watchdog keys not present, skipping watchdog\n");
			return 0;
		}
	}

	wdd->info = &t2smc_wdt_info;
	wdd->ops = &t2smc_wdt_ops;
	wdd->parent = t2->dev;
	wdd->min_timeout = 1;
	/* systemd asks for 10 minutes during shutdown (RebootWatchdogSec) */
	wdd->max_hw_heartbeat_ms = T2SMC_WDT_HW_MAX_TIMEOUT * MSEC_PER_SEC;
	wdd->timeout = T2SMC_WDT_DEFAULT_TIMEOUT;
	watchdog_init_timeout(wdd, wdt_timeout, t2->dev);
	watchdog_set_nowayout(wdd, nowayout);
	watchdog_stop_on_reboot(wdd);
	watchdog_stop_on_unregister(wdd);
	watchdog_set_drvdata(wdd, t2);

	ret = devm_watchdog_register_device(t2->dev, wdd);
	if (ret)
		return ret;
	t2->has_wdt = true;

	dev_info(t2->dev, "watchdog registered (%s, timeout=%us)\n",
		 t2->has_oswd ? "OSWD" : "NATi/NATJ", wdd->timeout);
	return 0;
}

/* -- SMC event interrupt -- */
/*
 * macOS answers ShutdownImminent with EmergencyHeadPark on every AHCI and
 * NVMe disk, which makes NVMe flush and prepare for abrupt power loss. The
 * closest Linux equivalent is a full sync including block device flushes.
 */
static void t2smc_sync_work(struct work_struct *work)
{
	ksys_sync_helper();
}

/* Wake up pollers of the thermal level files whose value changed */
static void t2smc_thermal_work(struct work_struct *work)
{
	struct t2smc_device *t2 = container_of(work, struct t2smc_device,
					       thermal_work);
	static const struct {
		u8 port;
		const char *attr;
	} levels[] = {
		{ T2SMC_PORT_THERMAL_CPU, "smc_thermal_level_cpu" },
		{ T2SMC_PORT_THERMAL_IO, "smc_thermal_level_io" },
		{ T2SMC_PORT_THERMAL_GPU, "smc_thermal_level_gpu" },
	};
	struct device *hwmon_dev = READ_ONCE(t2->hwmon_dev);
	u8 level;
	int i;

	if (!hwmon_dev)
		return;

	for (i = 0; i < ARRAY_SIZE(levels); i++) {
		level = inb(t2->port_base + levels[i].port);
		if (level == t2->thermal_level[i])
			continue;
		t2->thermal_level[i] = level;
		sysfs_notify(&hwmon_dev->kobj, NULL, levels[i].attr);
	}
}

/* Wake up pollers of smc_plimit_raw */
static void t2smc_plimit_work(struct work_struct *work)
{
	struct t2smc_device *t2 = container_of(work, struct t2smc_device,
					       plimit_work);
	struct device *hwmon_dev = READ_ONCE(t2->hwmon_dev);

	if (hwmon_dev)
		sysfs_notify(&hwmon_dev->kobj, NULL, "smc_plimit_raw");
}

/* AppleSMC copies the text and prints it as "Log: %s" */
static void t2smc_log_message(struct t2smc_device *t2)
{
	char msg[T2SMC_LOG_LEN + 1];
	int i;

	memcpy_fromio(msg, t2->iomem + T2SMC_IOMEM_LOG, T2SMC_LOG_LEN);
	msg[T2SMC_LOG_LEN] = '\0';
	for (i = 0; msg[i]; i++)
		if (!isprint(msg[i]))
			msg[i] = ' ';
	dev_info_ratelimited(t2->dev, "SMC log: %s\n", strim(msg));
}

static irqreturn_t t2smc_irq(int irq, void *data)
{
	struct t2smc_device *t2 = data;
	u8 event;

	if (!(ioread8(t2->iomem + T2SMC_IOMEM_INT_STATUS) & 0x20))
		return IRQ_HANDLED;

	event = inb(t2->port_base + T2SMC_PORT_EVENT);
	switch (event) {
	case T2SMC_EVENT_KEY_DONE:
		complete(&t2->cmd_done);
		break;
	case T2SMC_EVENT_SHUTDOWN:
		dev_crit(t2->dev, "SMC reports imminent power loss, syncing storage\n");
		queue_work(system_highpri_wq, &t2->sync_work);
		break;
	case T2SMC_EVENT_BRIDGEOS_PANIC:
		dev_warn(t2->dev, "SMC reports a BridgeOS panic\n");
		break;
	/* Under load the SMC reports level changes about once per second */
	case T2SMC_EVENT_THERMAL_LEVEL:
		dev_dbg(t2->dev, "SMC thermal level changed\n");
		schedule_work(&t2->thermal_work);
		break;
	/* Sent once right after NTOK is set */
	case T2SMC_EVENT_THERMAL_CONFIG:
		dev_dbg(t2->dev, "SMC thermal configuration changed\n");
		break;
	case T2SMC_EVENT_LOG:
		t2smc_log_message(t2);
		break;
	case T2SMC_EVENT_PLIMIT:
		dev_dbg(t2->dev, "SMC power limit changed\n");
		schedule_work(&t2->plimit_work);
		break;
	default:
		dev_dbg(t2->dev, "SMC event 0x%02x\n", event);
		break;
	}

	return IRQ_HANDLED;
}

/*
 * Handles an NTOK reply to a key write. Events that are still
 * pending are drained first, because the interrupt is edge triggered and
 * would not fire again for the KeyDone of the NTOK write. Interrupt mode is
 * switched on before the write so that its KeyDone already arrives by
 * interrupt, and off again if the write fails.
 */
static int t2smc_enable_notifications(struct t2smc_device *t2)
{
	u8 val = 1;
	int i, ret;

	mutex_lock(&t2->mutex);
	for (i = 0; i <= T2SMC_NTOK_DRAIN_TRIES &&
	     (ioread8(t2->iomem + T2SMC_IOMEM_INT_STATUS) & 0x20); i++) {
		inb(t2->port_base + T2SMC_PORT_EVENT);
		msleep(T2SMC_NTOK_DRAIN_MS);
	}

	WRITE_ONCE(t2->cmd_irq, true);
	ret = write_smc(t2, T2SMC_NOTIFY, &val, 1);
	if (ret)
		WRITE_ONCE(t2->cmd_irq, false);
	mutex_unlock(&t2->mutex);
	return ret;
}

static void t2smc_disable_events(void *data)
{
	struct t2smc_device *t2 = data;
	u8 val = 0;

	mutex_lock(&t2->mutex);
	WRITE_ONCE(t2->cmd_irq, false);
	write_smc(t2, T2SMC_NOTIFY, &val, 1);
	mutex_unlock(&t2->mutex);
}

static void t2smc_cancel_sync_work(void *data)
{
	struct t2smc_device *t2 = data;

	cancel_work_sync(&t2->sync_work);
}

/* Runs before the hwmon device goes away, later thermal work is a no-op */
static void t2smc_stop_thermal_notify(void *data)
{
	struct t2smc_device *t2 = data;

	WRITE_ONCE(t2->hwmon_dev, NULL);
	cancel_work_sync(&t2->thermal_work);
	cancel_work_sync(&t2->plimit_work);
}

/*
 * SMC events are optional like in AppleSMC. Without an interrupt, the I/O
 * port window or a working NTOK the driver keeps polling for KeyDone.
 */
static int t2smc_setup_events(struct platform_device *pdev,
			      struct t2smc_device *t2)
{
	int irq, ret;

	if (!t2->has_port) {
		dev_info(t2->dev, "no SMC I/O port window, polling for KeyDone\n");
		return 0;
	}

	irq = platform_get_irq_optional(pdev, 0);
	if (irq < 0) {
		dev_info(t2->dev, "no SMC interrupt, polling for KeyDone\n");
		return 0;
	}

	if (!devm_request_region(t2->dev, t2->port_base, T2SMC_PORT_MIN_SIZE,
				 "t2smc")) {
		dev_warn(t2->dev, "I/O ports 0x%x busy, polling for KeyDone\n",
			 t2->port_base);
		return 0;
	}

	ret = devm_add_action_or_reset(t2->dev, t2smc_cancel_sync_work, t2);
	if (ret)
		return ret;

	ret = devm_request_irq(t2->dev, irq, t2smc_irq, 0, "t2smc", t2);
	if (ret) {
		dev_warn(t2->dev, "failed to request IRQ %d: %d, polling for KeyDone\n",
			 irq, ret);
		return 0;
	}
	t2->has_events = true;

	ret = devm_add_action_or_reset(t2->dev, t2smc_disable_events, t2);
	if (ret)
		return ret;

	ret = t2smc_enable_notifications(t2);
	if (ret)
		dev_warn(t2->dev, "failed to enable SMC events: %d, polling for KeyDone\n",
			 ret);
	else if (!READ_ONCE(t2->cmd_irq))
		dev_warn(t2->dev, "SMC events enabled (IRQ %d), but KeyDone is polled\n",
			 irq);
	else
		dev_info(t2->dev, "SMC events enabled (IRQ %d)\n", irq);
	return 0;
}

/*
 * -- Raw key access through /dev/smc --
 * Same ioctl ABI as the macsmc patch 3014-mfd-macsmc-add-ioctl-to-access-smc-keys
 * from t2linux/linux-t2-patches, so tools work with both drivers. Keys the
 * driver owns or that control charging, clock, watchdog, events and fans
 * cannot be written.
 */
typedef u32 smc_key;

#define T2SMC_KEY(a, b, c, d) \
	(((u32)(a) << 24) | ((u32)(b) << 16) | ((u32)(c) << 8) | (u32)(d))

struct apple_smc_key_info {
	u32 type_code;
	u8 size;
	u8 flags;
};

struct ioctl_smc_rw_payload {
	smc_key key;
	size_t size;
	u8 buf[32];
};

struct ioctl_smc_index_payload {
	int index;
	smc_key key;
};

struct ioctl_smc_info_payload {
	smc_key key;
	struct apple_smc_key_info info;
};

#define APPLE_SMC_MAGIC 'S'
#define IOCTL_SMC_READ           _IOWR(APPLE_SMC_MAGIC, 1, struct ioctl_smc_rw_payload)
#define IOCTL_SMC_WRITE          _IOW(APPLE_SMC_MAGIC, 2, struct ioctl_smc_rw_payload)
#define IOCTL_SMC_GET_KEY_INDEX  _IOWR(APPLE_SMC_MAGIC, 3, struct ioctl_smc_index_payload)
#define IOCTL_SMC_GET_KEY_INFO   _IOWR(APPLE_SMC_MAGIC, 4, struct ioctl_smc_info_payload)

static void t2smc_key_to_str(smc_key key, char *str)
{
	str[0] = key >> 24;
	str[1] = key >> 16;
	str[2] = key >> 8;
	str[3] = key;
	str[4] = '\0';
}

static smc_key t2smc_str_to_key(const char *str)
{
	return T2SMC_KEY(str[0], str[1], str[2], str[3]);
}

static bool t2smc_key_write_denied(smc_key key)
{
	switch (key) {
	case T2SMC_KEY('C', 'H', '0', 'I'):
	case T2SMC_KEY('C', 'H', '0', 'C'):
	case T2SMC_KEY('C', 'H', '0', 'K'):
	case T2SMC_KEY('C', 'H', '0', 'B'):
	case T2SMC_KEY('C', 'H', 'T', 'E'):
	case T2SMC_KEY('C', 'H', 'L', 'S'):
	case T2SMC_KEY('B', 'C', 'L', 'M'):
	case T2SMC_KEY('M', 'B', 'S', 'E'):
	case T2SMC_KEY('C', 'L', 'K', 'L'):
	case T2SMC_KEY('C', 'L', 'K', 'O'):
	case T2SMC_KEY('O', 'S', 'W', 'D'):
	case T2SMC_KEY('N', 'A', 'T', 'J'):
	case T2SMC_KEY('N', 'A', 'T', 'i'):
	case T2SMC_KEY('N', 'E', 'S', 'N'):
	case T2SMC_KEY('M', 'O', 'C', 'N'):
	case T2SMC_KEY('L', 'K', 'S', 'B'):
	case T2SMC_KEY('N', 'T', 'O', 'K'):
	case T2SMC_KEY('N', 'T', 'A', 'P'):
	case T2SMC_KEY('F', '0', 'T', 'g'):
	case T2SMC_KEY('F', '1', 'T', 'g'):
	case T2SMC_KEY('F', '0', 'M', 'd'):
	case T2SMC_KEY('F', '1', 'M', 'd'):
	case T2SMC_KEY('F', 'S', '!', ' '):
		return true;
	}
	return false;
}

static long t2smc_smcdev_rw(struct t2smc_device *t2, void __user *argp,
			    bool write)
{
	struct ioctl_smc_rw_payload payload;
	struct t2smc_entry *entry;
	char key[5];
	int ret;

	if (copy_from_user(&payload, argp, sizeof(payload)))
		return -EFAULT;

	t2smc_key_to_str(payload.key, key);
	entry = t2smc_get_entry_by_key(t2, key);
	if (IS_ERR(entry))
		return PTR_ERR(entry);
	if (payload.size != entry->len || payload.size > sizeof(payload.buf))
		return -EINVAL;

	if (write) {
		if (t2smc_key_write_denied(payload.key))
			return -EPERM;
		return t2smc_write_key(t2, key, payload.buf, payload.size);
	}

	ret = t2smc_read_key(t2, key, payload.buf, payload.size);
	if (ret)
		return ret;
	if (copy_to_user(argp, &payload, sizeof(payload)))
		return -EFAULT;
	return 0;
}

static long t2smc_smcdev_ioctl(struct file *file, unsigned int cmd,
			       unsigned long arg)
{
	struct t2smc_device *t2 = container_of(file->private_data,
					       struct t2smc_device, miscdev);
	void __user *argp = (void __user *)arg;
	struct t2smc_entry *entry;
	char key[5];

	switch (cmd) {
	case IOCTL_SMC_READ:
		return t2smc_smcdev_rw(t2, argp, false);
	case IOCTL_SMC_WRITE:
		return t2smc_smcdev_rw(t2, argp, true);
	case IOCTL_SMC_GET_KEY_INDEX: {
		struct ioctl_smc_index_payload payload;

		if (copy_from_user(&payload, argp, sizeof(payload)))
			return -EFAULT;
		if (payload.index < 0 || payload.index >= t2->key_count)
			return -EINVAL;

		entry = t2smc_get_entry_by_index(t2, payload.index);
		if (IS_ERR(entry))
			return PTR_ERR(entry);
		payload.key = t2smc_str_to_key(entry->key);

		if (copy_to_user(argp, &payload, sizeof(payload)))
			return -EFAULT;
		return 0;
	}
	case IOCTL_SMC_GET_KEY_INFO: {
		struct ioctl_smc_info_payload payload;

		if (copy_from_user(&payload, argp, sizeof(payload)))
			return -EFAULT;

		t2smc_key_to_str(payload.key, key);
		entry = t2smc_get_entry_by_key(t2, key);
		if (IS_ERR(entry))
			return PTR_ERR(entry);
		payload.info.type_code = t2smc_str_to_key(entry->type);
		payload.info.size = entry->len;
		payload.info.flags = entry->flags;

		if (copy_to_user(argp, &payload, sizeof(payload)))
			return -EFAULT;
		return 0;
	}
	default:
		return -EINVAL;
	}
}

static const struct file_operations t2smc_smcdev_fops = {
	.owner = THIS_MODULE,
	.unlocked_ioctl = t2smc_smcdev_ioctl,
	.compat_ioctl = compat_ptr_ioctl,
	.llseek = noop_llseek,
};

static void t2smc_unregister_smcdev(void *data)
{
	misc_deregister(data);
}

static int t2smc_register_smcdev(struct t2smc_device *t2)
{
	int ret;

	t2->miscdev.minor = MISC_DYNAMIC_MINOR;
	t2->miscdev.name = "smc";
	t2->miscdev.fops = &t2smc_smcdev_fops;
	t2->miscdev.parent = t2->dev;
	t2->miscdev.mode = 0600;

	ret = misc_register(&t2->miscdev);
	if (ret)
		return ret;
	return devm_add_action_or_reset(t2->dev, t2smc_unregister_smcdev,
					&t2->miscdev);
}

/* devm action: cleanup non-devm resources after hwmon devres */
static void t2smc_devm_cleanup(void *data)
{
	struct t2smc_device *t2 = data;

	if (t2->iomem)
		iounmap(t2->iomem);
	mutex_destroy(&t2->mutex);
	mutex_destroy(&t2->battery_lock);
	mutex_destroy(&t2->charge_lock);
	kfree(t2->cache);
	t2smc_free_sensors(t2);
}

static void t2smc_unregister_power_notifier(void *data)
{
	struct t2smc_device *t2 = data;

	if (t2->power_notifier_registered) {
		power_supply_unreg_notifier(&t2->power_supply_nb);
		t2->power_notifier_registered = false;
	}
	cancel_work_sync(&t2->power_event_work);
}

/*
 * Apple names causes in its PowerManagement sources (common/CommonLib.c,
 * PowerManagement-1846), which pmconfigd logs as "SMC shutdown cause".
 * The community entries come from the Eclectic Light Company and George
 * Garside lists. Only causes on which those lists agree, or that only one
 * of them names, are included, and Apple wins every conflict. They are
 * marked so that each one can be confirmed or refuted on real hardware.
 */
static const struct {
	s8 cause;
	bool apple;
	const char *desc;
} t2smc_shutdown_causes[] = {
	{    0, true,  "Battery disconnected" },
	{    1, true,  "Normal warm reset" },
	{    2, true,  "Power supply disconnected" },
	{    3, true,  "Power button pressed for > 4 sec" },
	{    5, true,  "Software initiated shutdown" },
	{    7, true,  "Normal shutdown by SOC" },
	{   -3, false, "Multiple temperature sensors too high" },
	{  -14, false, "Electricity spike or surge" },
	{  -20, false, "BridgeOS (T2) initiated shutdown" },
	{  -60, true,  "Battery fully drained" },
	{  -61, false, "Watchdog detected unresponsive app, shutting down" },
	{  -62, false, "Watchdog detected unresponsive app, restarting" },
	{  -64, false, "Kernel panic" },
	{  -71, false, "Memory temperature too high" },
	{  -74, false, "Battery temperature too high" },
	{  -75, false, "Power adapter communication problem" },
	{  -78, false, "Incorrect input current from power adapter" },
	{  -79, false, "Incorrect current from battery" },
	{  -81, true,  "Thermal shutdown for overtemp" },
	{  -86, false, "Proximity temperature too high" },
	{ -100, false, "Power supply temperature too high" },
	{ -101, false, "Display temperature too high" },
	{ -102, false, "Overvoltage" },
	{ -103, false, "Battery voltage too low" },
	{ -104, false, "Unknown battery fault" },
	{ -127, false, "PMU/SMC forced shutdown for another cause" },
};

/*
 * Log the cause of the previous shutdown like macOS does at boot ("Previous
 * shutdown cause: %d"). The writes match macOS so that a one-shot cause is not reported again on the next boot.
 */
static void t2smc_log_shutdown_cause(struct t2smc_device *t2)
{
	u8 cause, flag = 0, val = 0;
	bool has_flag, has_cause;
	int i;

	if (t2smc_has_key(t2, T2SMC_SHUTDOWN_CAUSE, &has_cause) || !has_cause)
		return;
	if (t2smc_has_key(t2, T2SMC_SHUTDOWN_FLAG, &has_flag))
		return;
	if (has_flag && t2smc_read_key(t2, T2SMC_SHUTDOWN_FLAG, &flag, 1))
		flag = 0;

	if (t2smc_read_key(t2, T2SMC_SHUTDOWN_CAUSE, &cause, 1)) {
		dev_warn(t2->dev, "failed to read the previous shutdown cause\n");
		return;
	}

	if (has_flag && cause == T2SMC_CAUSE_FLAGGED) {
		if (flag != 1)
			cause = T2SMC_CAUSE_RESET;
		else if (t2smc_write_key(t2, T2SMC_SHUTDOWN_FLAG, &val, 1))
			dev_warn(t2->dev, "failed to clear MSSW\n");
	} else if (!has_flag && cause == T2SMC_CAUSE_ONE_SHOT) {
		val = T2SMC_CAUSE_RESET;
		if (t2smc_write_key(t2, T2SMC_SHUTDOWN_CAUSE, &val, 1))
			dev_warn(t2->dev, "failed to reset MSSD\n");
	}

	for (i = 0; i < ARRAY_SIZE(t2smc_shutdown_causes); i++)
		if (t2smc_shutdown_causes[i].cause == (s8)cause)
			break;
	if (i == ARRAY_SIZE(t2smc_shutdown_causes))
		dev_info(t2->dev, "previous shutdown cause: %d (unknown)\n",
			 (s8)cause);
	else
		dev_info(t2->dev, "previous shutdown cause: %d (%s, %s)\n",
			 (s8)cause, t2smc_shutdown_causes[i].desc,
			 t2smc_shutdown_causes[i].apple ? "Apple" : "community");
}

/*
 * MSSD stays unchanged when the T2 panics and takes the x86 side down.
 * bridgeOS records these keys in its own panic log, so they are logged raw
 * in the byte order of that log, to compare them across boots.
 */
static void t2smc_log_x86_state(struct t2smc_device *t2)
{
	static const char *const keys[] = {
		T2SMC_X86_POWER_STATE, T2SMC_X86_SYSTEM_STATE,
		T2SMC_X86_EFI_STATE, T2SMC_X86_TRANSITIONS,
	};
	struct t2smc_entry *entry;
	char line[128];
	size_t pos = 0;
	u8 buf[8];
	int i;

	for (i = 0; i < ARRAY_SIZE(keys); i++) {
		entry = t2smc_get_entry_by_key(t2, keys[i]);
		if (IS_ERR(entry) || !entry->len || entry->len > sizeof(buf) ||
		    t2smc_read_key(t2, keys[i], buf, entry->len))
			continue;
		pos += scnprintf(line + pos, sizeof(line) - pos, " %s=%*ph",
				 keys[i], entry->len, buf);
	}
	if (pos)
		dev_info(t2->dev, "previous x86 state:%s\n", line);
}

static void t2smc_log_plimit_keys(struct t2smc_device *t2)
{
	static const char *const keys[] = {
		T2SMC_PLIMIT_GEN, T2SMC_PLIMIT_VERSION,
		T2SMC_PLIMIT_EGPU_STEPS, T2SMC_PLIMIT_IGPU_STEPS,
		T2SMC_PLIMIT_EGPU3_STEPS, T2SMC_CPU_TURBO_STATES,
		T2SMC_CPU_FLOOR_STATES,
	};
	struct t2smc_entry *entry;
	char line[128];
	size_t pos = 0;
	u8 buf[8];
	int i;

	for (i = 0; i < ARRAY_SIZE(keys); i++) {
		entry = t2smc_get_entry_by_key(t2, keys[i]);
		if (IS_ERR(entry) || !entry->len || entry->len > sizeof(buf) ||
		    t2smc_read_key(t2, keys[i], buf, entry->len))
			continue;
		pos += scnprintf(line + pos, sizeof(line) - pos, " %s=%*ph",
				 keys[i], entry->len, buf);
	}
	if (pos)
		dev_info(t2->dev, "PLimit keys:%s\n", line);
}

/*
 * The SMC forgets the step counts over sleep, user space set them once.
 * macOS rewrites the CPU floor on wake as well.
 */
static void t2smc_set_plimit_ranges(struct t2smc_device *t2)
{
	const char *key;
	u8 count;
	int gpu, ret;

	if (READ_ONCE(t2->cpu_floor_set)) {
		ret = t2smc_write_cpu_floor(t2, t2->cpu_floor);
		if (ret)
			dev_warn(t2->dev, "failed to set the CPU floor: %d\n", ret);
	}

	for (gpu = 0; gpu < T2SMC_GPUS; gpu++) {
		count = READ_ONCE(t2->gpu_steps[gpu]);
		if (!count)
			continue;
		key = t2smc_gpu_steps_key(t2, gpu);
		ret = t2smc_write_key(t2, key, &count, 1);
		if (ret)
			dev_warn(t2->dev, "failed to set %s: %d\n", key, ret);
	}
}

/*
 * RGEN 3 means data version 3. Otherwise the
 * data size of DPLM selects it, 5 for version 3, 4 for 2 and 3 for 1, and a
 * missing DPLM means version 1. macOS only acts on version 2 and
 * later.
 */
static void t2smc_read_plimit_version(struct t2smc_device *t2)
{
	struct t2smc_entry *entry;
	u8 gen;

	entry = t2smc_get_entry_by_key(t2, T2SMC_PLIMIT_GEN);
	if (!IS_ERR(entry) && entry->len == 1 &&
	    !t2smc_read_key(t2, T2SMC_PLIMIT_GEN, &gen, 1) && gen == 3) {
		t2->plimit_version = 3;
		return;
	}

	entry = t2smc_get_entry_by_key(t2, T2SMC_PLIMIT_VERSION);
	if (IS_ERR(entry)) {
		t2->plimit_version = 1;
		return;
	}
	switch (entry->len) {
	case 5: t2->plimit_version = 3; break;
	case 4: t2->plimit_version = 2; break;
	case 3: t2->plimit_version = 1; break;
	default: t2->plimit_version = 0; break;
	}
}

/* -- Platform driver callbacks -- */
static int t2smc_probe(struct platform_device *pdev)
{
	struct acpi_device *adev = ACPI_COMPANION(&pdev->dev);
	struct t2smc_device *t2;
	int ret;

	if (!adev)
		return -ENODEV;

	t2 = devm_kzalloc(&pdev->dev, sizeof(*t2), GFP_KERNEL);
	if (!t2)
		return -ENOMEM;

	t2->adev = adev;
	t2->dev = &pdev->dev;
	mutex_init(&t2->mutex);
	mutex_init(&t2->battery_lock);
	mutex_init(&t2->charge_lock);
	INIT_WORK(&t2->power_event_work, t2smc_power_event_work);
	INIT_WORK(&t2->sync_work, t2smc_sync_work);
	INIT_WORK(&t2->thermal_work, t2smc_thermal_work);
	INIT_WORK(&t2->plimit_work, t2smc_plimit_work);
	init_completion(&t2->cmd_done);
	atomic64_set(&t2->power_event_count, 0);
	t2->power_supply_nb.notifier_call = t2smc_power_supply_event;
	platform_set_drvdata(pdev, t2);

	/*
	 * Register cleanup action before anything that can fail.
	 * devres runs in reverse order, so this runs AFTER hwmon devres,
	 * ensuring hwmon callbacks never see freed t2.
	 */
	ret = devm_add_action_or_reset(&pdev->dev, t2smc_devm_cleanup, t2);
	if (ret)
		return ret;

	/* Walk ACPI _CRS to find MMIO region */
	ret = acpi_walk_resources(adev->handle, METHOD_NAME__CRS,
				  t2smc_walk_resources, t2);
	if (ACPI_FAILURE(ret) || !t2->iomem_addr) {
		dev_err(t2->dev, "No suitable MMIO resource found\n");
		ret = -ENXIO;
		return ret;
	}

	ret = t2smc_try_enable_iomem(pdev, t2);
	if (ret)
		return ret;

	/* Retry key cache init with timeout */
	{
		int ms;
		for (ms = 0; ms < INIT_TIMEOUT_MSECS; ms += INIT_WAIT_MSECS) {
			/* Free old cache from previous failed attempt */
			kfree(t2->cache);
			t2->cache = NULL;
			t2smc_free_sensors(t2);
			t2->key_count = 0;

			ret = t2smc_init_keycache(t2);
			if (!ret) {
				if (ms)
					dev_info(t2->dev,
						 "keycache init took %d ms\n", ms);
				break;
			}
			if (ret == -EUCLEAN)
				break;
			msleep(INIT_WAIT_MSECS);
		}
		if (ret) {
			dev_err(t2->dev, "Failed to init key cache: %d\n", ret);
			return ret;
		}
	}

	t2smc_log_shutdown_cause(t2);
	t2smc_log_x86_state(t2);
	t2smc_log_plimit_keys(t2);
	t2smc_read_plimit_version(t2);
	dev_info(t2->dev, "PLimit data version %u\n", t2->plimit_version);

	ret = t2smc_register_hwmon(t2);
	if (ret)
		return ret;
	ret = devm_add_action_or_reset(&pdev->dev, t2smc_stop_thermal_notify, t2);
	if (ret)
		return ret;

	ret = devm_add_action_or_reset(&pdev->dev, t2smc_detach_battery, t2);
	if (ret)
		return ret;
	t2smc_attach_battery(t2);

	ret = t2smc_register_rtc(t2);
	if (ret)
		return ret;

	ret = t2smc_register_watchdog(t2);
	if (ret)
		return ret;

	ret = t2smc_register_smcdev(t2);
	if (ret)
		return ret;

	if (!t2smc_read_key(t2, T2SMC_BATTERY_STATUS, &t2->power_status,
			    sizeof(t2->power_status)))
		t2->power_status_valid = true;

	ret = power_supply_reg_notifier(&t2->power_supply_nb);
	if (ret)
		return ret;
	t2->power_notifier_registered = true;
	ret = devm_add_action_or_reset(&pdev->dev,
				       t2smc_unregister_power_notifier, t2);
	if (ret)
		return ret;

	dev_info(t2->dev, "t2smc %s ready (fans=%u)\n",
		 T2SMC_VERSION, t2->fan_count);
	return 0;
}

static const struct acpi_device_id t2smc_ids[] = {
	{ "APP0001", 0 },
	{ "smc-huronriver", 0 },
	{ "", 0 },
};
MODULE_DEVICE_TABLE(acpi, t2smc_ids);

static int t2smc_suspend(struct device *dev)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	int ret;

	t2smc_rtc_sync_from_system(t2);

	/* The SMC keeps counting while the host sleeps */
	if (t2->has_wdt && watchdog_active(&t2->wdd)) {
		ret = t2smc_wdt_stop(&t2->wdd);
		if (ret)
			return ret;
		t2->wdt_suspended = true;
	}
	return 0;
}

static int t2smc_resume(struct device *dev)
{
	struct t2smc_device *t2 = dev_get_drvdata(dev);
	int ret;

	/* Like macOS, retry NTOK after a revert to polling */
	if (t2->has_events && !READ_ONCE(t2->cmd_irq)) {
		ret = t2smc_enable_notifications(t2);
		if (ret)
			dev_warn(dev, "failed to re-enable SMC events: %d\n", ret);
	}

	t2smc_restore_fans(t2);
	t2smc_set_plimit_ranges(t2);

	if (t2->wdt_suspended) {
		t2->wdt_suspended = false;
		return t2smc_wdt_start(&t2->wdd);
	}
	return 0;
}

static DEFINE_SIMPLE_DEV_PM_OPS(t2smc_pm_ops, t2smc_suspend, t2smc_resume);

static void t2smc_shutdown(struct platform_device *pdev)
{
	t2smc_rtc_sync_from_system(platform_get_drvdata(pdev));
}

static struct platform_driver t2smc_driver = {
	.probe = t2smc_probe,
	.shutdown = t2smc_shutdown,
	.driver = {
		.name = "t2smc",
		.acpi_match_table = t2smc_ids,
		.pm = pm_sleep_ptr(&t2smc_pm_ops),
	},
};

module_platform_driver(t2smc_driver);

MODULE_AUTHOR("André Eikmeyer <andre.eikmeyer@kait2en.org");
MODULE_DESCRIPTION("T2 Mac SMC driver");
MODULE_LICENSE("GPL");
MODULE_VERSION(T2SMC_VERSION);
MODULE_ALIAS("applesmc");

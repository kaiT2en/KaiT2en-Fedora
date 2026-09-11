// SPDX-License-Identifier: GPL-2.0-only
/*
 * Apple T2 Secure Enclave Processor PCI transport.
 *
 * Reaches the SEP mailbox on device 106b:1802 and, with register_ool=1,
 * exposes /dev/t2sep for opaque request/response exchanges with the
 * AppleKeyStore endpoint. Nothing in KAIT2EN consumes it yet, so it is not
 * shipped and does not autoload; see README.md.
 */

#include <linux/device.h>
#include <linux/ctype.h>
#include <linux/fs.h>
#include <linux/miscdevice.h>
#include <linux/dma-mapping.h>
#include <linux/jiffies.h>
#include <linux/ktime.h>
#include <linux/module.h>
#include <linux/mutex.h>
#include <linux/pci.h>
#include <linux/sizes.h>
#include <linux/uaccess.h>
#include <linux/unaligned.h>
#include <linux/workqueue.h>

#include <crypto/hash.h>
#include <crypto/sha2.h>

#include "t2sep_mailbox.h"
#include "t2sep_uapi.h"

#define T2SEP_VENDOR_ID   0x106b
#define T2SEP_DEVICE_ID   0x1802
#define T2SEP_MAILBOX_BAR 4
#define T2SEP_DMA_BITS    44
#define T2SEP_OOL_SIZE    SZ_16K

#define T2SEP_CONTROL_ENDPOINT 0
#define T2SEP_AKS_ENDPOINT     7
#define T2SEP_SET_OOL_IN       2
#define T2SEP_SET_OOL_OUT      3
#define T2SEP_CONTROL_TIMEOUT  5000
#define T2SEP_MAX_SKIPPED      32
#define T2SEP_DISCOVERY_ENDPOINT 0xfd
#define T2SEP_DISCOVERY_MAX      32
#define T2SEP_DISCOVERY_BATCH    64
#define T2SEP_DISCOVERY_POLL_MS  10
#define T2SEP_DISCOVERY_MAX_MS   60000

#define T2SEP_IOP_RESET          0x8040
#define T2SEP_IOP_RUN            0x8048
#define T2SEP_IOP_CONTROL        0x8028
#define T2SEP_IOP_CONTROL_START  5

#define T2SEP_TESTING_ENDPOINT   0x0f
#define T2SEP_TEST_QUERY         0x09
#define T2SEP_TEST_SEQUENCE      1

#define T2SEP_AKS_GET_CAPABILITIES 0x4d
#define T2SEP_AKS_HEADER_VERSION   1
#define T2SEP_AKS_HEADER_SIZE      0x48
#define T2SEP_AKS_WIRE_HEADER_SIZE (sizeof(__le32) + T2SEP_AKS_HEADER_SIZE)
#define T2SEP_AKS_CAP_REQUEST_SIZE 0x5c

struct t2sep_aks_header {
	u8 digest[16];
	__le32 version;
	__le64 usec_time;
	__le32 flags;
	__le64 clock_id;
	u8 platform_data[0x20];
} __packed;

static_assert(sizeof(struct t2sep_aks_header) == T2SEP_AKS_HEADER_SIZE);

struct t2sep_discovery_entry {
	u32 name;
	u32 aux;
	u8 endpoint;
	u8 opcode;
	bool valid;
};

struct t2sep_device {
	struct pci_dev *pdev;
	void __iomem *bar;
	struct t2sep_mailbox mailbox;
	struct mutex mailbox_lock;
	struct mutex discovery_lock;
	struct delayed_work discovery_work;
	unsigned long discovery_deadline;
	struct t2sep_discovery_entry discovery[T2SEP_DISCOVERY_MAX];
	u32 discovery_messages;
	u32 discovery_unknown_messages;
	bool discovery_complete;
	bool sep_start_issued;
	bool testing_complete;
	bool testing_error;
	u8 testing_reply_code;
	u32 testing_reply_word1;
	u32 testing_reply_word2;
	bool control_complete;
	u32 control_result;
	void *ool_in;
	dma_addr_t ool_in_dma;
	void *ool_out;
	dma_addr_t ool_out_dma;
	bool ool_in_registered;
	bool ool_out_registered;
	bool ool_dma_retained;
	bool capabilities_complete;
	u64 capabilities;
	u16 capabilities_reply_length;
	struct miscdevice miscdev;
	bool miscdev_registered;
};

static bool register_ool;
module_param(register_ool, bool, 0400);
MODULE_PARM_DESC(register_ool,
	"Register 16 KiB input/output buffers for AKS endpoint 7 (default: false)");

static bool probe_capabilities;
module_param(probe_capabilities, bool, 0400);
MODULE_PARM_DESC(probe_capabilities,
	"Issue one read-only AKS capability query after OOL registration (default: false)");

static uint discovery_window_ms;
module_param(discovery_window_ms, uint, 0400);
MODULE_PARM_DESC(discovery_window_ms,
	"Passively collect SEP discovery advertisements for up to 60000 ms (default: 0)");

static bool start_sep;
module_param(start_sep, bool, 0400);
MODULE_PARM_DESC(start_sep,
	"Start the SEP IOP using the recovered Apple Intel startup sequence (default: false)");

static bool probe_testing;
module_param(probe_testing, bool, 0400);
MODULE_PARM_DESC(probe_testing,
	"Issue Apple's read-only EP_TESTING query-tests header request (default: false)");

static bool probe_control;
module_param(probe_control, bool, 0400);
MODULE_PARM_DESC(probe_control,
	"Issue Apple's side-effect-free endpoint-0 control NOP (default: false)");

static void t2sep_record_discovery(struct t2sep_device *sep,
				   const struct t2sep_message *message);

static void t2sep_start_iop(struct t2sep_device *sep)
{
	/* Recovered from AppleSEPIntelIOP::_startCPUGated(). */
	writel(0, sep->bar + T2SEP_IOP_RESET);
	writel(1, sep->bar + T2SEP_IOP_RUN);
	wmb();
	writel(T2SEP_IOP_CONTROL_START, sep->bar + T2SEP_IOP_CONTROL);
	/* Flush posted PCI writes before returning to the discovery poller. */
	readl(sep->bar + T2SEP_IOP_CONTROL);
	sep->sep_start_issued = true;
	dev_info(&sep->pdev->dev,
		 "issued recovered Apple SEP IOP startup sequence\n");
}

static int t2sep_probe_testing(struct t2sep_device *sep)
{
	struct t2sep_message request = { };
	struct t2sep_message reply;
	unsigned int skipped = 0;
	u8 endpoint;
	u8 sequence;
	int ret;

	/* AppleSEPTesting::query_tests(): endpoint, command, sequence, zero payload. */
	request.word[0] = T2SEP_TESTING_ENDPOINT |
		(T2SEP_TEST_QUERY << 8) | (T2SEP_TEST_SEQUENCE << 16);

	mutex_lock(&sep->mailbox_lock);
	ret = t2sep_mailbox_send(&sep->mailbox, &request,
				 T2SEP_CONTROL_TIMEOUT);
	if (ret)
		goto out_unlock;

	for (;;) {
		ret = t2sep_mailbox_receive(&sep->mailbox, &reply,
					      T2SEP_CONTROL_TIMEOUT);
		if (ret)
			goto out_unlock;
		endpoint = reply.word[0] & 0xff;
		sequence = (reply.word[0] >> 8) & 0x7f;
		if (endpoint == T2SEP_TESTING_ENDPOINT &&
		    sequence == T2SEP_TEST_SEQUENCE)
			break;
		t2sep_record_discovery(sep, &reply);
		if (++skipped == T2SEP_MAX_SKIPPED) {
			ret = -EOVERFLOW;
			goto out_unlock;
		}
	}

	sep->testing_error = !!(reply.word[0] & BIT(15));
	sep->testing_reply_code = (reply.word[0] >> 16) & 0xff;
	sep->testing_reply_word1 = reply.word[1];
	sep->testing_reply_word2 = reply.word[2];
	sep->testing_complete = true;
	dev_info(&sep->pdev->dev,
		 "SEP testing query replied: error=%u code=%u word1=%#x word2=%#x\n",
		 sep->testing_error, sep->testing_reply_code,
		 sep->testing_reply_word1, sep->testing_reply_word2);

out_unlock:
	mutex_unlock(&sep->mailbox_lock);
	return ret;
}

static int t2sep_probe_control(struct t2sep_device *sep)
{
	struct t2sep_message request = { };
	struct t2sep_message reply;
	unsigned int skipped = 0;
	u8 endpoint;
	u8 tag = 1;
	int ret;

	/* AppleSEPControl::cmsgNOP(): zero message; _cmsgSend() supplies the tag. */
	request.word[0] = tag << 8;

	mutex_lock(&sep->mailbox_lock);
	ret = t2sep_mailbox_send(&sep->mailbox, &request,
				 T2SEP_CONTROL_TIMEOUT);
	if (ret)
		goto out_unlock;

	for (;;) {
		ret = t2sep_mailbox_receive(&sep->mailbox, &reply,
					      T2SEP_CONTROL_TIMEOUT);
		if (ret)
			goto out_unlock;
		endpoint = reply.word[0] & 0xff;
		if (endpoint == T2SEP_CONTROL_ENDPOINT &&
		    ((reply.word[0] >> 8) & 0xff) == tag)
			break;
		t2sep_record_discovery(sep, &reply);
		if (++skipped == T2SEP_MAX_SKIPPED) {
			ret = -EOVERFLOW;
			goto out_unlock;
		}
	}

	sep->control_result = reply.word[1];
	sep->control_complete = true;
	dev_info(&sep->pdev->dev,
		 "SEP control NOP replied: result=%#x word2=%#x word3=%#x\n",
		 sep->control_result, reply.word[2], reply.word[3]);
	if (sep->control_result)
		ret = -EREMOTEIO;

out_unlock:
	mutex_unlock(&sep->mailbox_lock);
	return ret;
}

static bool t2sep_fourcc_printable(u32 value)
{
	unsigned int shift;

	for (shift = 0; shift < 32; shift += 8) {
		u8 byte = value >> shift;

		if (!isascii(byte) || byte < 0x20)
			return false;
	}
	return true;
}

static void t2sep_format_fourcc(u32 value, char text[5])
{
	text[0] = value;
	text[1] = value >> 8;
	text[2] = value >> 16;
	text[3] = value >> 24;
	text[4] = '\0';
}

static void t2sep_record_discovery(struct t2sep_device *sep,
				   const struct t2sep_message *message)
{
	u8 wire_endpoint = message->word[0] & 0xff;
	u8 opcode = (message->word[0] >> 16) & 0xff;
	u8 runtime_endpoint = message->word[0] >> 24;
	u32 name = message->word[1];
	char fourcc[5];
	int free_slot = -1;
	int i;

	mutex_lock(&sep->discovery_lock);
	if (wire_endpoint != T2SEP_DISCOVERY_ENDPOINT) {
		sep->discovery_unknown_messages++;
		goto out_unlock;
	}
	sep->discovery_messages++;
	if ((opcode != 0 && opcode != 1) || !t2sep_fourcc_printable(name)) {
		dev_dbg(&sep->pdev->dev,
			"discovery message opcode=%u endpoint=%u name=%#x aux=%#x\n",
			opcode, runtime_endpoint, name, message->word[2]);
		goto out_unlock;
	}

	for (i = 0; i < T2SEP_DISCOVERY_MAX; i++) {
		if (!sep->discovery[i].valid) {
			if (free_slot < 0)
				free_slot = i;
			continue;
		}
		if (sep->discovery[i].name == name ||
		    sep->discovery[i].endpoint == runtime_endpoint) {
			free_slot = i;
			break;
		}
	}
	if (free_slot >= 0) {
		sep->discovery[free_slot].name = name;
		sep->discovery[free_slot].aux = message->word[2];
		sep->discovery[free_slot].endpoint = runtime_endpoint;
		sep->discovery[free_slot].opcode = opcode;
		sep->discovery[free_slot].valid = true;
	}

	t2sep_format_fourcc(name, fourcc);
	dev_info(&sep->pdev->dev,
		 "discovered SEP service '%s' at runtime endpoint %u (opcode=%u aux=%#x)\n",
		 fourcc, runtime_endpoint, opcode, message->word[2]);
out_unlock:
	mutex_unlock(&sep->discovery_lock);
}

static void t2sep_discovery_work(struct work_struct *work)
{
	struct t2sep_device *sep = container_of(to_delayed_work(work),
						struct t2sep_device,
						discovery_work);
	struct t2sep_message message;
	unsigned int count;

	mutex_lock(&sep->mailbox_lock);
	for (count = 0; count < T2SEP_DISCOVERY_BATCH; count++) {
		if (t2sep_mailbox_try_receive(&sep->mailbox, &message))
			break;
		t2sep_record_discovery(sep, &message);
	}
	mutex_unlock(&sep->mailbox_lock);

	if (time_before(jiffies, sep->discovery_deadline)) {
		schedule_delayed_work(&sep->discovery_work,
				      msecs_to_jiffies(T2SEP_DISCOVERY_POLL_MS));
		return;
	}
	mutex_lock(&sep->discovery_lock);
	sep->discovery_complete = true;
	dev_info(&sep->pdev->dev,
		 "passive discovery complete: advertisements=%u other_messages=%u\n",
		 sep->discovery_messages, sep->discovery_unknown_messages);
	mutex_unlock(&sep->discovery_lock);
}

static int t2sep_control(struct t2sep_device *sep, u8 opcode, u8 tag,
			 dma_addr_t dma, size_t size, bool *posted)
{
	struct t2sep_message request = { };
	struct t2sep_message reply;
	unsigned int skipped = 0;
	u8 endpoint;
	u8 reply_tag;
	int ret;

	*posted = false;
	if (!IS_ALIGNED(dma, PAGE_SIZE) || dma >> T2SEP_DMA_BITS || size > U32_MAX)
		return -ERANGE;

	request.word[0] = T2SEP_CONTROL_ENDPOINT | (tag << 8) |
		(opcode << 16) | (T2SEP_AKS_ENDPOINT << 24);
	request.word[1] = lower_32_bits(dma >> PAGE_SHIFT);
	request.word[2] = size;

	mutex_lock(&sep->mailbox_lock);
	ret = t2sep_mailbox_send(&sep->mailbox, &request,
				 T2SEP_CONTROL_TIMEOUT);
	if (ret)
		goto out_unlock;
	*posted = true;

	for (;;) {
		ret = t2sep_mailbox_receive(&sep->mailbox, &reply,
					      T2SEP_CONTROL_TIMEOUT);
		if (ret)
			goto out_unlock;

		endpoint = reply.word[0] & GENMASK(4, 0);
		reply_tag = (reply.word[0] >> 8) & 0xff;
		if (endpoint == T2SEP_CONTROL_ENDPOINT && reply_tag == tag)
			break;

		dev_dbg(&sep->pdev->dev,
			"ignored mailbox message from endpoint %u while waiting for tag %u\n",
			endpoint, tag);
		if (++skipped == T2SEP_MAX_SKIPPED) {
			ret = -EOVERFLOW;
			goto out_unlock;
		}
	}

	if (reply.word[1]) {
		dev_err(&sep->pdev->dev,
			"OOL control opcode %u returned SEP result %#x\n",
			opcode, reply.word[1]);
		ret = -EREMOTEIO;
	}

out_unlock:
	mutex_unlock(&sep->mailbox_lock);
	return ret;
}

static void t2sep_free_ool(struct t2sep_device *sep)
{
	if (sep->ool_out)
		dma_free_coherent(&sep->pdev->dev, T2SEP_OOL_SIZE,
				  sep->ool_out, sep->ool_out_dma);
	if (sep->ool_in)
		dma_free_coherent(&sep->pdev->dev, T2SEP_OOL_SIZE,
				  sep->ool_in, sep->ool_in_dma);
	sep->ool_out = NULL;
	sep->ool_in = NULL;
}

static int t2sep_register_ool(struct t2sep_device *sep)
{
	bool posted;
	int ret;

	/* Establish the endpoint-0 dialogue before handing DMA memory to SEP. */
	ret = t2sep_probe_control(sep);
	if (ret)
		return dev_err_probe(&sep->pdev->dev, ret,
				     "control NOP handshake failed; refusing OOL registration\n");

	ret = dma_set_mask_and_coherent(&sep->pdev->dev,
					 DMA_BIT_MASK(T2SEP_DMA_BITS));
	if (ret)
		return dev_err_probe(&sep->pdev->dev, ret,
				     "44-bit DMA is unavailable\n");

	sep->ool_in = dma_alloc_coherent(&sep->pdev->dev, T2SEP_OOL_SIZE,
					 &sep->ool_in_dma, GFP_KERNEL);
	if (!sep->ool_in)
		return -ENOMEM;

	sep->ool_out = dma_alloc_coherent(&sep->pdev->dev, T2SEP_OOL_SIZE,
					  &sep->ool_out_dma, GFP_KERNEL);
	if (!sep->ool_out) {
		ret = -ENOMEM;
		goto err_free;
	}

	if (!IS_ALIGNED(sep->ool_in_dma, PAGE_SIZE) ||
	    !IS_ALIGNED(sep->ool_out_dma, PAGE_SIZE)) {
		ret = -ERANGE;
		dev_err(&sep->pdev->dev, "SEP OOL DMA buffers are not page aligned\n");
		goto err_free;
	}

	pci_set_master(sep->pdev);
	ret = t2sep_control(sep, T2SEP_SET_OOL_IN, 1,
			    sep->ool_in_dma, T2SEP_OOL_SIZE, &posted);
	if (ret) {
		if (posted && ret != -EREMOTEIO) {
			dev_err(&sep->pdev->dev,
				"AKS input registration result is unknown: %d; retaining DMA memory until reboot\n",
				ret);
			sep->ool_dma_retained = true;
			__module_get(THIS_MODULE);
			return 0;
		}
		dev_err(&sep->pdev->dev,
			"failed to register AKS input buffer: %d\n", ret);
		goto err_clear_master;
	}
	sep->ool_in_registered = true;

	ret = t2sep_control(sep, T2SEP_SET_OOL_OUT, 2,
			    sep->ool_out_dma, T2SEP_OOL_SIZE, &posted);
	if (ret) {
		dev_err(&sep->pdev->dev,
			"AKS input registered but output registration failed: %d; reboot required\n",
			ret);
		sep->ool_dma_retained = true;
		__module_get(THIS_MODULE);
		return 0;
	}
	sep->ool_out_registered = true;

	/* SEP retains the DMA addresses and provides no known unregister command. */
	sep->ool_dma_retained = true;
	__module_get(THIS_MODULE);
	dev_info(&sep->pdev->dev,
		 "registered 16 KiB AKS endpoint-7 input/output buffers\n");
	return 0;

err_clear_master:
	pci_clear_master(sep->pdev);
err_free:
	t2sep_free_ool(sep);
	return ret;
}

static int t2sep_aks_digest(void *message, size_t length)
{
	struct t2sep_aks_header *header = message + sizeof(__le32);
	struct crypto_shash *tfm;
	struct shash_desc *desc;
	u8 digest[SHA256_DIGEST_SIZE];
	int ret;

	if (length < T2SEP_AKS_WIRE_HEADER_SIZE)
		return -EINVAL;

	tfm = crypto_alloc_shash("sha256", 0, 0);
	if (IS_ERR(tfm))
		return PTR_ERR(tfm);
	desc = kmalloc(sizeof(*desc) + crypto_shash_descsize(tfm), GFP_KERNEL);
	if (!desc) {
		crypto_free_shash(tfm);
		return -ENOMEM;
	}
	desc->tfm = tfm;

	ret = crypto_shash_init(desc);
	if (!ret)
		ret = crypto_shash_update(desc,
					 (u8 *)header + sizeof(header->digest),
					 T2SEP_AKS_HEADER_SIZE - sizeof(header->digest));
	if (!ret)
		ret = crypto_shash_update(desc,
					 message + T2SEP_AKS_WIRE_HEADER_SIZE,
					 length - T2SEP_AKS_WIRE_HEADER_SIZE);
	if (!ret)
		ret = crypto_shash_final(desc, digest);
	if (!ret)
		memcpy(header->digest, digest, sizeof(header->digest));

	memzero_explicit(digest, sizeof(digest));
	kfree(desc);
	crypto_free_shash(tfm);
	return ret;
}

/*
 * Post a request that is already staged in the OOL input buffer and wait for
 * the matching reply descriptor. The caller holds mailbox_lock and owns the
 * OOL buffers; on success the response is in the OOL output buffer and its
 * length is returned in reply_length.
 */
static int t2sep_ool_exchange(struct t2sep_device *sep, u8 endpoint,
			      u8 operation, u32 request_length, u16 *reply_length)
{
	struct t2sep_message request = { };
	struct t2sep_message reply;
	unsigned int skipped = 0;
	int ret;

	request.word[0] = endpoint | (operation << 8) | (1 << 16);
	request.word[1] = request_length << 16;

	/* Publish the coherent request before notifying SEP through MMIO. */
	dma_wmb();
	ret = t2sep_mailbox_send(&sep->mailbox, &request, T2SEP_CONTROL_TIMEOUT);
	if (ret)
		return ret;

	for (;;) {
		ret = t2sep_mailbox_receive(&sep->mailbox, &reply,
					    T2SEP_CONTROL_TIMEOUT);
		if (ret)
			return ret;
		if ((reply.word[0] & 0xff) == endpoint &&
		    ((reply.word[0] >> 8) & 0x7f) == operation &&
		    ((reply.word[0] >> 16) & 0xff) == 1)
			break;
		if (++skipped == T2SEP_MAX_SKIPPED)
			return -EOVERFLOW;
	}
	/* SEP completed the descriptor before publishing the OOL response. */
	dma_rmb();

	*reply_length = reply.word[1] >> 16;
	return 0;
}

static int t2sep_probe_capabilities(struct t2sep_device *sep)
{
	struct t2sep_aks_header *header;
	u8 expected_digest[16];
	u8 *payload;
	u16 reply_length;
	int ret;

	if (!sep->ool_in_registered || !sep->ool_out_registered)
		return -ENXIO;

	memset(sep->ool_in, 0, T2SEP_OOL_SIZE);
	memset(sep->ool_out, 0, T2SEP_OOL_SIZE);
	put_unaligned_le32(T2SEP_AKS_HEADER_SIZE, sep->ool_in);
	header = sep->ool_in + sizeof(__le32);
	header->version = cpu_to_le32(T2SEP_AKS_HEADER_VERSION);
	header->usec_time = cpu_to_le64(ktime_get_boottime_ns() / NSEC_PER_USEC);

	/* Read-only request body: result placeholder, selector 1, empty blob. */
	payload = sep->ool_in + T2SEP_AKS_WIRE_HEADER_SIZE;
	put_unaligned_le32(0, payload);
	put_unaligned_le64(1, payload + sizeof(__le32));
	put_unaligned_le32(0, payload + sizeof(__le32) + sizeof(__le64));
	ret = t2sep_aks_digest(sep->ool_in, T2SEP_AKS_CAP_REQUEST_SIZE);
	if (ret)
		return ret;

	mutex_lock(&sep->mailbox_lock);
	ret = t2sep_ool_exchange(sep, T2SEP_AKS_ENDPOINT,
				 T2SEP_AKS_GET_CAPABILITIES,
				 T2SEP_AKS_CAP_REQUEST_SIZE, &reply_length);
	if (ret)
		goto out_unlock;
	if (reply_length < T2SEP_AKS_WIRE_HEADER_SIZE ||
	    reply_length > T2SEP_OOL_SIZE ||
	    get_unaligned_le32(sep->ool_out) != T2SEP_AKS_HEADER_SIZE ||
	    get_unaligned_le32(sep->ool_out + sizeof(__le32) + 0x10) !=
	    T2SEP_AKS_HEADER_VERSION) {
		ret = -EPROTO;
		goto out_unlock;
	}

	memcpy(expected_digest, sep->ool_out + sizeof(__le32),
	       sizeof(expected_digest));
	memset(sep->ool_out + sizeof(__le32), 0, sizeof(expected_digest));
	ret = t2sep_aks_digest(sep->ool_out, reply_length);
	if (!ret && memcmp(expected_digest, sep->ool_out + sizeof(__le32),
			   sizeof(expected_digest)))
		ret = -EBADMSG;
	memzero_explicit(expected_digest, sizeof(expected_digest));
	if (ret)
		goto out_unlock;

	if (reply_length < T2SEP_AKS_CAP_REQUEST_SIZE) {
		ret = -EPROTO;
		goto out_unlock;
	}
	payload = sep->ool_out + T2SEP_AKS_WIRE_HEADER_SIZE;
	if (get_unaligned_le32(payload)) {
		dev_warn(&sep->pdev->dev,
			 "AKS capability query returned status %#x\n",
			 get_unaligned_le32(payload));
		ret = -EREMOTEIO;
		goto out_unlock;
	}

	sep->capabilities = get_unaligned_le64(payload + sizeof(__le32));
	sep->capabilities_reply_length = reply_length;
	sep->capabilities_complete = true;
	dev_info(&sep->pdev->dev,
		 "AKS capabilities=%#llx reply_length=%u (integrity verified)\n",
		 (unsigned long long)sep->capabilities, reply_length);

out_unlock:
	memzero_explicit(sep->ool_in, T2SEP_OOL_SIZE);
	memzero_explicit(sep->ool_out, T2SEP_OOL_SIZE);
	mutex_unlock(&sep->mailbox_lock);
	return ret;
}

static ssize_t mailbox_status_show(struct device *dev,
				   struct device_attribute *attr, char *buf)
{
	struct t2sep_device *sep = pci_get_drvdata(to_pci_dev(dev));
	u32 inbox = t2sep_mailbox_inbox_status(&sep->mailbox);
	u32 outbox = t2sep_mailbox_outbox_status(&sep->mailbox);

	return sysfs_emit(buf, "inbox=0x%08x empty=%u outbox=0x%08x full=%u\n",
			  inbox, t2sep_mailbox_inbox_empty(&sep->mailbox),
			  outbox, t2sep_mailbox_outbox_full(&sep->mailbox));
}
static DEVICE_ATTR_RO(mailbox_status);

static ssize_t ool_status_show(struct device *dev,
			       struct device_attribute *attr, char *buf)
{
	struct t2sep_device *sep = pci_get_drvdata(to_pci_dev(dev));

	return sysfs_emit(buf,
			  "requested=%u input=%u output=%u retained=%u size=%u\n",
			  register_ool, sep->ool_in_registered,
			  sep->ool_out_registered, sep->ool_dma_retained,
			  T2SEP_OOL_SIZE);
}
static DEVICE_ATTR_RO(ool_status);

static ssize_t capabilities_show(struct device *dev,
				 struct device_attribute *attr, char *buf)
{
	struct t2sep_device *sep = pci_get_drvdata(to_pci_dev(dev));

	if (!sep->capabilities_complete)
		return sysfs_emit(buf, "requested=%u complete=0\n",
				  probe_capabilities);
	return sysfs_emit(buf,
			  "requested=1 complete=1 value=0x%016llx reply_length=%u\n",
			  (unsigned long long)sep->capabilities,
			  sep->capabilities_reply_length);
}
static DEVICE_ATTR_RO(capabilities);

static ssize_t discovery_show(struct device *dev,
			      struct device_attribute *attr, char *buf)
{
	struct t2sep_device *sep = pci_get_drvdata(to_pci_dev(dev));
	ssize_t length = 0;
	char fourcc[5];
	int i;

	mutex_lock(&sep->discovery_lock);
	length += sysfs_emit_at(buf, length,
		"requested_ms=%u start_issued=%u complete=%u advertisements=%u other_messages=%u\n",
		discovery_window_ms, sep->sep_start_issued,
		sep->discovery_complete,
		sep->discovery_messages, sep->discovery_unknown_messages);
	for (i = 0; i < T2SEP_DISCOVERY_MAX && length < PAGE_SIZE; i++) {
		if (!sep->discovery[i].valid)
			continue;
		t2sep_format_fourcc(sep->discovery[i].name, fourcc);
		length += sysfs_emit_at(buf, length,
			"name=%s endpoint=%u opcode=%u aux=0x%08x\n",
			fourcc, sep->discovery[i].endpoint,
			sep->discovery[i].opcode, sep->discovery[i].aux);
	}
	mutex_unlock(&sep->discovery_lock);
	return length;
}
static DEVICE_ATTR(discovery, 0400, discovery_show, NULL);

static ssize_t testing_show(struct device *dev,
			    struct device_attribute *attr, char *buf)
{
	struct t2sep_device *sep = pci_get_drvdata(to_pci_dev(dev));

	if (!sep->testing_complete)
		return sysfs_emit(buf, "requested=%u complete=0\n", probe_testing);
	return sysfs_emit(buf,
			  "requested=1 complete=1 error=%u code=%u word1=0x%08x word2=0x%08x\n",
			  sep->testing_error, sep->testing_reply_code,
			  sep->testing_reply_word1, sep->testing_reply_word2);
}
static DEVICE_ATTR(testing, 0400, testing_show, NULL);

static ssize_t control_show(struct device *dev,
			    struct device_attribute *attr, char *buf)
{
	struct t2sep_device *sep = pci_get_drvdata(to_pci_dev(dev));

	if (!sep->control_complete)
		return sysfs_emit(buf, "requested=%u complete=0\n",
				  probe_control || register_ool);
	return sysfs_emit(buf, "requested=1 complete=1 result=0x%08x\n",
			  sep->control_result);
}
static DEVICE_ATTR(control, 0400, control_show, NULL);

static struct attribute *t2sep_attrs[] = {
	&dev_attr_mailbox_status.attr,
	&dev_attr_ool_status.attr,
	&dev_attr_capabilities.attr,
	&dev_attr_discovery.attr,
	&dev_attr_testing.attr,
	&dev_attr_control.attr,
	NULL,
};

static const struct attribute_group t2sep_attr_group = {
	.attrs = t2sep_attrs,
};

static long t2sep_ioctl(struct file *file, unsigned int cmd, unsigned long arg)
{
	struct t2sep_device *sep =
		container_of(file->private_data, struct t2sep_device, miscdev);
	struct t2sep_exchange ex;
	u16 reply_length;
	int ret;

	if (cmd != T2SEP_IOC_EXCHANGE)
		return -ENOTTY;
	if (!capable(CAP_SYS_ADMIN))
		return -EPERM;
	if (copy_from_user(&ex, (void __user *)arg, sizeof(ex)))
		return -EFAULT;
	if (ex.endpoint == 0 || ex.request_length == 0 ||
	    ex.request_length > T2SEP_OOL_SIZE ||
	    ex.response_capacity > T2SEP_OOL_SIZE)
		return -EINVAL;
	if (!sep->ool_in_registered || !sep->ool_out_registered)
		return -ENXIO;

	mutex_lock(&sep->mailbox_lock);
	memset(sep->ool_in, 0, T2SEP_OOL_SIZE);
	if (copy_from_user(sep->ool_in, (void __user *)(uintptr_t)ex.request,
			   ex.request_length)) {
		ret = -EFAULT;
		goto out_unlock;
	}
	memset(sep->ool_out, 0, T2SEP_OOL_SIZE);
	ret = t2sep_ool_exchange(sep, ex.endpoint, ex.operation,
				 ex.request_length, &reply_length);
	if (ret)
		goto out_unlock;
	if (reply_length > T2SEP_OOL_SIZE) {
		ret = -EPROTO;
		goto out_unlock;
	}
	ex.response_length = reply_length;
	if (reply_length > ex.response_capacity) {
		ret = -ENOSPC;
		goto out_unlock;
	}
	if (ex.response &&
	    copy_to_user((void __user *)(uintptr_t)ex.response, sep->ool_out,
			 reply_length)) {
		ret = -EFAULT;
		goto out_unlock;
	}
	ret = 0;
out_unlock:
	mutex_unlock(&sep->mailbox_lock);
	if (ret)
		return ret;
	if (copy_to_user((void __user *)arg, &ex, sizeof(ex)))
		return -EFAULT;
	return 0;
}

static const struct file_operations t2sep_fops = {
	.owner = THIS_MODULE,
	.unlocked_ioctl = t2sep_ioctl,
};

static int t2sep_probe(struct pci_dev *pdev, const struct pci_device_id *id)
{
	struct t2sep_device *sep;
	int ret;

	if (discovery_window_ms > T2SEP_DISCOVERY_MAX_MS)
		return dev_err_probe(&pdev->dev, -EINVAL,
				     "discovery_window_ms exceeds 60000\n");
	if (discovery_window_ms && (register_ool || probe_capabilities))
		return dev_err_probe(&pdev->dev, -EINVAL,
				     "passive discovery cannot be combined with active OOL or AKS probing\n");
	if (probe_testing &&
	    (discovery_window_ms || register_ool || probe_capabilities))
		return dev_err_probe(&pdev->dev, -EINVAL,
				     "testing query cannot be combined with discovery, OOL, or AKS probing\n");
	if (probe_control &&
	    (discovery_window_ms || register_ool || probe_capabilities ||
	     probe_testing))
		return dev_err_probe(&pdev->dev, -EINVAL,
				     "control NOP cannot be combined with another probe\n");
	if (start_sep && !discovery_window_ms && !probe_testing && !probe_control)
		return dev_err_probe(&pdev->dev, -EINVAL,
				     "start_sep requires discovery or the testing query\n");

	if (pci_resource_len(pdev, T2SEP_MAILBOX_BAR) < T2SEP_MAILBOX_MIN_SIZE)
		return dev_err_probe(&pdev->dev, -ENODEV,
				     "BAR4 is too small for the SEP mailbox\n");

	ret = pci_enable_device_mem(pdev);
	if (ret)
		return dev_err_probe(&pdev->dev, ret,
				     "cannot enable SEP PCI memory resources\n");

	ret = pci_request_region(pdev, T2SEP_MAILBOX_BAR, "t2sep");
	if (ret) {
		dev_err_probe(&pdev->dev, ret, "cannot claim SEP BAR4\n");
		goto err_disable_device;
	}

	sep = devm_kzalloc(&pdev->dev, sizeof(*sep), GFP_KERNEL);
	if (!sep) {
		ret = -ENOMEM;
		goto err_release_region;
	}

	sep->bar = pci_iomap(pdev, T2SEP_MAILBOX_BAR, 0);
	if (!sep->bar) {
		ret = -ENOMEM;
		dev_err(&pdev->dev, "cannot map SEP BAR4\n");
		goto err_release_region;
	}

	sep->pdev = pdev;
	t2sep_mailbox_init(&sep->mailbox, sep->bar);
	mutex_init(&sep->mailbox_lock);
	mutex_init(&sep->discovery_lock);
	INIT_DELAYED_WORK(&sep->discovery_work, t2sep_discovery_work);
	pci_set_drvdata(pdev, sep);

	ret = sysfs_create_group(&pdev->dev.kobj, &t2sep_attr_group);
	if (ret) {
		dev_err_probe(&pdev->dev, ret,
			      "cannot create SEP status attributes\n");
		goto err_unmap;
	}

	dev_info(&pdev->dev,
		 "SEP mailbox ready: inbox %#x (%s), outbox %#x (%s)\n",
		 t2sep_mailbox_inbox_status(&sep->mailbox),
		 t2sep_mailbox_inbox_empty(&sep->mailbox) ? "empty" : "data pending",
		 t2sep_mailbox_outbox_status(&sep->mailbox),
		 t2sep_mailbox_outbox_full(&sep->mailbox) ? "full" : "ready");

	if (discovery_window_ms) {
		sep->discovery_deadline = jiffies +
			msecs_to_jiffies(discovery_window_ms);
		schedule_delayed_work(&sep->discovery_work, 0);
		dev_info(&pdev->dev,
			 "%s SEP discovery for %u ms\n",
			 start_sep ? "starting IOP and observing" :
			 "passively observing", discovery_window_ms);
		if (start_sep)
			t2sep_start_iop(sep);
	} else {
		if (start_sep)
			t2sep_start_iop(sep);
		if (probe_testing) {
			ret = t2sep_probe_testing(sep);
			if (ret)
				dev_warn(&pdev->dev,
					 "read-only SEP testing query failed: %d\n",
					 ret);
		}
		if (probe_control) {
			ret = t2sep_probe_control(sep);
			if (ret)
				dev_warn(&pdev->dev,
					 "SEP control NOP failed: %d\n", ret);
		}
	}

	if (register_ool) {
		ret = t2sep_register_ool(sep);
		if (ret)
			goto err_remove_group;
		sep->miscdev.minor = MISC_DYNAMIC_MINOR;
		sep->miscdev.name = "t2sep";
		sep->miscdev.fops = &t2sep_fops;
		sep->miscdev.mode = 0600;
		ret = misc_register(&sep->miscdev);
		if (ret)
			goto err_remove_group;
		sep->miscdev_registered = true;
		if (probe_capabilities && sep->ool_in_registered &&
		    sep->ool_out_registered) {
			ret = t2sep_probe_capabilities(sep);
			if (ret)
				dev_warn(&pdev->dev,
					 "read-only AKS capability query failed: %d\n",
					 ret);
		}
	} else if (probe_capabilities) {
		dev_warn(&pdev->dev,
			 "probe_capabilities requires register_ool=1\n");
	}
	return 0;

err_remove_group:
	sysfs_remove_group(&pdev->dev.kobj, &t2sep_attr_group);
err_unmap:
	pci_iounmap(pdev, sep->bar);
err_release_region:
	pci_release_region(pdev, T2SEP_MAILBOX_BAR);
err_disable_device:
	pci_disable_device(pdev);
	return ret;
}

static void t2sep_remove(struct pci_dev *pdev)
{
	struct t2sep_device *sep = pci_get_drvdata(pdev);

	cancel_delayed_work_sync(&sep->discovery_work);
	if (sep->miscdev_registered)
		misc_deregister(&sep->miscdev);
	sysfs_remove_group(&pdev->dev.kobj, &t2sep_attr_group);
	if (sep->ool_dma_retained) {
		dev_warn(&pdev->dev,
			 "SEP still owns OOL DMA buffers; refusing unsafe cleanup\n");
		return;
	}
	t2sep_free_ool(sep);
	pci_clear_master(pdev);
	pci_iounmap(pdev, sep->bar);
	pci_release_region(pdev, T2SEP_MAILBOX_BAR);
	pci_disable_device(pdev);
}

static const struct pci_device_id t2sep_ids[] = {
	{ PCI_DEVICE(T2SEP_VENDOR_ID, T2SEP_DEVICE_ID) },
	{ }
};
/* No MODULE_DEVICE_TABLE: the driver must not autoload onto the SEP while it
 * has no consumer. Load it by hand for SEP work. */

static struct pci_driver t2sep_driver = {
	.name = "t2sep",
	.id_table = t2sep_ids,
	.probe = t2sep_probe,
	.remove = t2sep_remove,
	.driver = {
		.suppress_bind_attrs = true,
	},
};
module_pci_driver(t2sep_driver);

MODULE_AUTHOR("Andre Eikmeyer <andre.eikmeyer@kait2en.org>");
MODULE_DESCRIPTION("Apple T2 SEP PCI mailbox and OOL transport");
MODULE_LICENSE("GPL");
MODULE_VERSION("0.8");
MODULE_INFO(endpoint, "control 0, AppleKeyStore 7, testing 15");
MODULE_INFO(transport, "128-bit SEP mailbox with 16 KiB coherent OOL DMA");
MODULE_INFO(safety, "active MMIO only through explicit module parameters");
MODULE_INFO(scope, "AppleKeyStore transport; Touch ID uses BCE/BridgeXPC");

#!/usr/bin/env python3
"""Detect, validate, and stage T2 Mac ACPI fixes for KaiT2en on Fedora.

The script reads the running machine's ACPI tables and current-boot kernel
log. It only builds overrides for the documented CpuSSDT sub-table loading
error and DSDT _OSC buffer overflow when their corresponding log signatures
are present. CpuSSDT is disassembled, structurally patched, and required to
recompile without errors or warnings. DSDT is patched directly in AML bytes
and independently re-disassembled with iasl before it can be deployed. A full
DSDT recompilation is deliberately avoided because unrelated constructs in
real Apple firmware tables do not reliably round-trip through iasl.

Validated tables are installed below /usr/local/lib/firmware/acpi and enabled
through a dracut configuration file. KaiT2en rebuilds the initramfs in the
following installer step; this script never rebuilds it itself. Every file
changed here is backed up first and restored if deployment fails.
"""

from __future__ import annotations

__author__ = "Alexander Fischer <alexander@fischermail.me>"

from collections import Counter
import datetime as dt
import os
import platform
import re
import shutil
import struct
import subprocess
import sys
import uuid as uuidlib
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Sequence

ACPI_TABLE_DIR = Path("/sys/firmware/acpi/tables")
DEPLOY_DIR = Path("/usr/local/lib/firmware/acpi")
DRACUT_CONF = Path("/etc/dracut.conf.d/t2-acpi-fix.conf")
BACKUP_ROOT = Path("/var/backups/t2-acpi-fix")

CPU_JOURNAL_GREP = "Marking method"
DSDT_JOURNAL_GREP = "AE_AML_BUFFER_LIMIT"
CPU_EXPECTED_RE = re.compile(
    r"Marking\s+method\s+.*?_PDC\s+as\s+Serialized.*AE_ALREADY_EXISTS", re.I
)

UUID_SB_OSC = "0811b06e-4a27-44f9-8d60-3cbbc22e7b48"
UUID_PCI0_OSC = "33db4d5b-1ff7-401c-9657-7441c03dd766"

DRACUT_REQUIRED_LINES = (
    'acpi_override="yes"',
    'acpi_table_dir="/usr/local/lib/firmware/acpi"',
)


class FixError(RuntimeError):
    """Expected, user-facing failure."""


@dataclass(frozen=True)
class Detection:
    cpussdt_problem: bool
    dsdt_problem: bool
    cpussdt_log: str
    dsdt_log: str


@dataclass(frozen=True)
class BuiltTable:
    kind: str
    source_table: Path
    details: Path
    aml: Path
    deploy_name: str


@dataclass(frozen=True)
class CompileResult:
    aml: Path
    errors: int
    warnings: int
    warning_codes: Counter[str]


@dataclass(frozen=True)
class AmlMethod:
    start: int
    pkg_length: int
    pkg_length_size: int
    flags: int
    body_start: int
    end: int

    @property
    def body_length(self) -> int:
        return self.end - self.body_start


def info(message: str) -> None:
    print(f"[kait2en] {message}", flush=True)


def output_summary(output: str) -> str:
    lines = [line.strip() for line in output.splitlines() if line.strip()]
    return lines[-1] if lines else ""


def run(
    argv: Sequence[str],
    *,
    cwd: Path | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run(
        list(argv),
        cwd=str(cwd) if cwd else None,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    if check and proc.returncode != 0:
        summary = output_summary(proc.stdout)
        detail = f": {summary}" if summary else ""
        raise FixError(
            f"command failed with exit status {proc.returncode}: "
            f"{' '.join(argv)}{detail}"
        )
    return proc


def require_root() -> None:
    if os.geteuid() != 0:
        raise FixError("run the KaiT2en installer with sudo")


def require_fedora() -> None:
    path = Path("/etc/os-release")
    if not path.is_file():
        raise FixError("/etc/os-release is missing")
    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key] = value.strip().strip('"').strip("'")
    distro_id = values.get("ID", "").lower()
    id_like = values.get("ID_LIKE", "").lower().split()
    if distro_id != "fedora" and "fedora" not in id_like:
        raise FixError("ACPI autofix is Fedora-only")


def require_apple_intel_supported() -> str:
    if platform.machine().lower() not in {"x86_64", "amd64"}:
        raise FixError(
            f"the ACPI fixes require an Intel T2 Mac; detected architecture "
            f"{platform.machine()!r}"
        )

    vendor_path = Path("/sys/class/dmi/id/sys_vendor")
    product_path = Path("/sys/class/dmi/id/product_name")
    vendor = vendor_path.read_text(errors="replace").strip() if vendor_path.exists() else ""
    product = product_path.read_text(errors="replace").strip() if product_path.exists() else ""

    if "apple" not in vendor.lower():
        raise FixError(f"this does not appear to be Apple hardware; DMI sys_vendor={vendor!r}")
    if not ACPI_TABLE_DIR.is_dir():
        raise FixError(f"ACPI table directory is unavailable: {ACPI_TABLE_DIR}")

    return product or "AppleMac"


def require_commands(commands: Iterable[str]) -> None:
    missing = [command for command in commands if shutil.which(command) is None]
    if missing:
        suffix = " (install acpica-tools)" if "iasl" in missing else ""
        raise FixError(f"missing required command(s): {', '.join(missing)}{suffix}")


def has_systemd_journal() -> bool:
    return Path("/run/systemd/system").is_dir() and shutil.which("journalctl") is not None


def kernel_log_grep(pattern: str) -> str:
    if has_systemd_journal():
        proc = run(
            ["journalctl", "-b", "0", "-k", "--no-pager", f"--grep={pattern}"],
            check=False,
        )
        if proc.returncode not in (0, 1):
            raise FixError(
                f"journalctl failed while searching for {pattern!r}: "
                f"{output_summary(proc.stdout)}"
            )
        return proc.stdout

    require_commands(("dmesg",))
    proc = run(["dmesg", "--kernel"], check=False)
    if proc.returncode != 0:
        raise FixError(
            f"dmesg failed while searching for {pattern!r}: "
            f"{output_summary(proc.stdout)}"
        )
    return "\n".join(line for line in proc.stdout.splitlines() if pattern in line)


def detect_problems() -> Detection:
    cpu_log = kernel_log_grep(CPU_JOURNAL_GREP)
    dsdt_log = kernel_log_grep(DSDT_JOURNAL_GREP)

    cpussdt_problem = bool(CPU_EXPECTED_RE.search(cpu_log))
    normalized = dsdt_log.replace("\\", "")
    has_buffer_error = "AE_AML_BUFFER_LIMIT" in normalized
    has_documented_osc = any(
        marker in normalized
        for marker in ("_SB._OSC", "_SB.PCI0._OSC", "Index (0x00000008)")
    )
    dsdt_problem = has_buffer_error and has_documented_osc
    return Detection(cpussdt_problem, dsdt_problem, cpu_log, dsdt_log)


def safe_timestamp() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def make_workdir(timestamp: str) -> Path:
    path = Path(f"/var/tmp/acpi-t2-fix-{timestamp}")
    try:
        path.mkdir(parents=True, exist_ok=False)
        path.chmod(0o700)
    except OSError as exc:
        raise FixError(f"cannot create new working directory {path}: {exc}") from exc
    return path


def find_cpussdt_table() -> Path:
    matches: list[Path] = []
    for candidate in sorted(ACPI_TABLE_DIR.glob("SSDT*")):
        if not candidate.is_file():
            continue
        try:
            data = candidate.read_bytes()
        except OSError as exc:
            raise FixError(f"Cannot read ACPI table {candidate}: {exc}") from exc
        if len(data) < 36 or data[:4] != b"SSDT":
            continue
        table_id = data[16:24].decode("ascii", errors="replace")
        if normalize_table_id(table_id) == "CPUSSDT":
            matches.append(candidate)

    if not matches:
        raise FixError(
            f"No SSDT with OEM Table ID 'CpuSsdt' was found under {ACPI_TABLE_DIR}."
        )
    if len(matches) > 1:
        joined = ", ".join(str(path) for path in matches)
        raise FixError(f"Multiple CpuSsdt candidates were found; refusing ambiguity: {joined}")
    return matches[0]


def copy_and_disassemble(
    source: Path,
    workdir: Path,
    *,
    external_tables: Sequence[Path] = (),
) -> tuple[Path, Path]:
    local_source = workdir / source.name
    shutil.copy2(source, local_source)
    argv = ["iasl"]
    if external_tables:
        argv.extend(["-e", *(table.name for table in external_tables)])
    argv.extend(["-d", local_source.name])
    run(argv, cwd=workdir)
    dsl = local_source.with_suffix(".dsl")
    if not dsl.is_file():
        raise FixError(f"iasl did not create expected DSL file: {dsl}")
    return local_source, dsl


def increment_definition_revision(text: str, signature: str, table_id: str | None) -> str:
    table_id_pattern = re.escape(table_id) if table_id is not None else r'[^"\r\n]*'
    pattern = re.compile(
        rf'^(?P<prefix>\s*DefinitionBlock\s*\(\s*""\s*,\s*"{re.escape(signature)}"'
        rf'\s*,\s*\d+\s*,\s*"[^"\r\n]*"\s*,\s*"{table_id_pattern}"\s*,\s*)'
        rf'(?P<revision>0x[0-9A-Fa-f]+|\d+)(?P<suffix>\s*\).*)$',
        re.MULTILINE,
    )
    matches = list(pattern.finditer(text))
    if len(matches) != 1:
        wanted = f"{signature}/{table_id or '*'}"
        raise FixError(
            f"Expected exactly one DefinitionBlock for {wanted}, found {len(matches)}."
        )

    match = matches[0]
    old_token = match.group("revision")
    old_value = int(old_token, 0)
    if old_value >= 0xFFFFFFFF:
        raise FixError("OEM revision cannot be incremented beyond 0xFFFFFFFF.")
    new_token = f"0x{old_value + 1:08X}"
    return text[: match.start()] + match.group("prefix") + new_token + match.group("suffix") + text[match.end() :]


def normalize_table_id(value: str) -> str:
    return re.sub(r"[\x00\s]+", "", value).upper()


def loaded_ssdt_table_ids() -> dict[str, list[str]]:
    """Return normalized OEM Table IDs for SSDTs already loaded by the kernel."""
    result: dict[str, list[str]] = {}
    for candidate in sorted(ACPI_TABLE_DIR.glob("SSDT*")):
        if not candidate.is_file():
            continue
        try:
            data = candidate.read_bytes()
        except OSError as exc:
            raise FixError(f"Cannot read ACPI table {candidate}: {exc}") from exc
        if len(data) < 36 or data[:4] != b"SSDT":
            continue
        declared_length = struct.unpack_from("<I", data, 4)[0]
        if declared_length != len(data):
            raise FixError(
                f"Loaded SSDT {candidate} has header length {declared_length}, "
                f"but file size {len(data)}."
            )
        raw_id = data[16:24].decode("ascii", errors="replace").rstrip("\x00 ")
        normalized = normalize_table_id(raw_id)
        if normalized:
            result.setdefault(normalized, []).append(candidate.name)
    return result


def asl_integer(token: str) -> int:
    value = token.strip()
    named = {"ZERO": 0, "ONE": 1, "ONES": 0xFFFFFFFFFFFFFFFF}
    if value.upper() in named:
        return named[value.upper()]
    try:
        return int(value, 0)
    except ValueError as exc:
        raise FixError(f"Unsupported ASL integer token: {token!r}") from exc


def strip_asl_comments(text: str) -> str:
    # Shared with the DSDT independent-validation code further below: both
    # CpuSSDT's text patching and the DSDT's post-patch disassembly checks
    # need to search decompiled ASL without matching inside comments.
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.DOTALL)
    return re.sub(r"//[^\r\n]*", " ", text)


def find_matching_brace(text: str, opening: int) -> int:
    depth = 0
    in_string = False
    escaped = False
    for index in range(opening, len(text)):
        char = text[index]
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return index
    raise FixError("Could not find the end of the CpuSSDT package block.")


def cpussdt_package_elements(text: str) -> list[str | int]:
    clean = strip_asl_comments(text)
    match = re.search(
        r"\bName\s*\(\s*(?:\\)?SSDT\s*,\s*Package\s*\([^)]*\)\s*\{",
        clean,
        re.IGNORECASE,
    )
    if not match:
        raise FixError("CpuSsdt does not contain the expected SSDT Package object.")
    opening = clean.find("{", match.start())
    closing = find_matching_brace(clean, opening)
    body = clean[opening + 1 : closing]
    token_re = re.compile(
        r'"(?P<string>[^"\\]*(?:\\.[^"\\]*)*)"|'
        r'(?<![A-Za-z0-9_])(?P<number>Zero|One|Ones|0x[0-9A-Fa-f]+|\d+)(?![A-Za-z0-9_])',
        re.IGNORECASE,
    )
    elements: list[str | int] = []
    for token in token_re.finditer(body):
        if token.group("string") is not None:
            elements.append(token.group("string"))
        else:
            elements.append(asl_integer(token.group("number")))
    if not elements:
        raise FixError("The CpuSSDT SSDT package could not be parsed.")
    return elements


def derive_cpussdt_sdtl_mask(
    text: str,
    loaded_ids: dict[str, list[str]],
) -> tuple[int, list[tuple[str, int, str, str]]]:
    """Derive bits for sub-tables that Linux has already loaded from the XSDT.

    Each dynamic Load() in CpuSsdt is associated with a package entry and an
    SDTL bit. We set a bit only when a currently loaded SSDT has the same OEM
    Table ID. This avoids hard-coding 0x3A and also avoids suppressing loads for
    HWP/PSD tables that are not static on a particular model.
    """
    elements = cpussdt_package_elements(text)
    clean = strip_asl_comments(text)
    integer_token = r"(?:Zero|One|Ones|0x[0-9A-Fa-f]+|\d+)"
    op_region_re = re.compile(
        rf"\bOperationRegion\s*\(\s*(?P<region>[A-Za-z_][A-Za-z0-9_]{{0,3}})\s*,"
        rf"\s*SystemMemory\s*,\s*DerefOf\s*\(\s*(?:\\)?SSDT\s*\[\s*"
        rf"(?P<index>{integer_token})\s*\]",
        re.IGNORECASE,
    )
    mask_assign_re = re.compile(
        rf"(?:\\)?SDTL\s*\|=\s*(?P<mask>{integer_token})",
        re.IGNORECASE,
    )
    mask_test_re = re.compile(
        rf"(?:\\)?SDTL\s*&\s*(?P<mask>{integer_token})",
        re.IGNORECASE,
    )

    mappings: list[tuple[str, int, str, str]] = []
    seen_labels: set[str] = set()
    for operation in op_region_re.finditer(clean):
        region = operation.group("region")
        after = clean[operation.end() : operation.end() + 600]
        if not re.search(rf"\bLoad\s*\(\s*{re.escape(region)}\s*,", after, re.IGNORECASE):
            continue

        element_index = asl_integer(operation.group("index"))
        if element_index <= 0 or element_index >= len(elements):
            raise FixError(
                f"CpuSsdt OperationRegion {region} references package index "
                f"{element_index}, outside the parsed SSDT package."
            )
        label_value = elements[element_index - 1]
        address_value = elements[element_index]
        if not isinstance(label_value, str) or not isinstance(address_value, int):
            raise FixError(
                f"CpuSsdt package index {element_index} for region {region} does not "
                "follow the expected label/address layout."
            )
        label = normalize_table_id(label_value)
        if not label or label not in loaded_ids:
            continue

        before = clean[max(0, operation.start() - 1200) : operation.start()]
        assignments = list(mask_assign_re.finditer(before))
        if not assignments:
            raise FixError(
                f"Could not find an SDTL bit assignment for loaded sub-table {label} "
                f"near OperationRegion {region}."
            )
        assignment = assignments[-1]
        mask = asl_integer(assignment.group("mask"))
        if mask <= 0 or mask & (mask - 1):
            raise FixError(
                f"SDTL value for {label} is not a single bit: 0x{mask:X}."
            )
        nearby_condition = before[max(0, assignment.start() - 700) : assignment.start()]
        tested_masks = [asl_integer(m.group("mask")) for m in mask_test_re.finditer(nearby_condition)]
        if mask not in tested_masks:
            raise FixError(
                f"The SDTL assignment 0x{mask:X} for {label} is not guarded by a "
                "matching SDTL bit test; refusing an unfamiliar CpuSsdt layout."
            )
        if label in seen_labels:
            raise FixError(f"Loaded CpuSsdt sub-table {label} was mapped more than once.")
        seen_labels.add(label)
        mappings.append((label, mask, region, ",".join(loaded_ids[label])))

    if not mappings:
        available = ", ".join(sorted(loaded_ids)) or "none"
        raise FixError(
            "No CpuSsdt dynamic Load() could be matched to an SSDT already loaded by "
            f"the kernel. Loaded OEM Table IDs: {available}."
        )

    mask = 0
    for _, bit, _, _ in mappings:
        if mask & bit:
            raise FixError(f"The same SDTL bit 0x{bit:X} maps to multiple loaded tables.")
        mask |= bit
    return mask, sorted(mappings)


def patch_cpussdt_text(
    text: str,
    loaded_ids: dict[str, list[str]],
) -> tuple[str, int, list[tuple[str, int, str, str]]]:
    if not re.search(r"\bGCAP\b", text):
        raise FixError("CpuSsdt does not contain GCAP; refusing an unfamiliar table.")

    mask, mappings = derive_cpussdt_sdtl_mask(text, loaded_ids)
    text = increment_definition_revision(text, "SSDT", "CpuSsdt")
    pattern = re.compile(
        r'^(?P<indent>\s*)Name\s*\(\s*(?P<root>\\?)SDTL\s*,\s*Zero\s*\)\s*$',
        re.MULTILINE,
    )
    matches = list(pattern.finditer(text))
    if len(matches) != 1:
        raise FixError(
            "Expected exactly one 'Name (\\SDTL, Zero)' in CpuSsdt, "
            f"found {len(matches)}."
        )
    match = matches[0]
    replacement = f"{match.group('indent')}Name ({match.group('root')}SDTL, 0x{mask:08X})"
    patched = text[: match.start()] + replacement + text[match.end() :]

    if not re.search(
        rf"Name\s*\(\s*\\?SDTL\s*,\s*0x{mask:08X}\s*\)",
        patched,
        re.IGNORECASE,
    ):
        raise FixError("Internal validation failed: derived SDTL value was not found.")
    return patched, mask, mappings


# ---------------------------------------------------------------------------
# DSDT fix (the _OSC buffer-overflow bug): patched directly in AML bytes
# instead of via disassemble/edit-ASL/recompile -- see the module docstring
# for why. Layout, top to bottom:
#   1. generic AML primitives (PkgLength, integers, NameSegs, UUID buffers)
#   2. the two _OSC shapes real firmware uses ("Family A" / "Family B") and
#      the fixed replacement bytes for each
#   3. find_osc_replacements() / patch_dsdt_aml(): tie 1+2 together into the
#      actual byte-level patch
#   4. a second, independent check of the same patch via iasl's decompiler
# ---------------------------------------------------------------------------

# Used only in the two regexes below, which match an ASL integer literal
# however iasl chose to render it (bare "4"/"8" or "0x04"/"0x08").
_OFFSET_4_RE = r"(?:0x0?4|4)"
_OFFSET_8_RE = r"(?:0x0?8|8)"


def validate_acpi_binary(data: bytes, expected_signature: bytes) -> None:
    """Validate the standard 36-byte ACPI header and whole-table checksum."""
    if len(data) < 36:
        raise FixError(f"ACPI table is too short: {len(data)} bytes.")
    if data[:4] != expected_signature:
        raise FixError(
            f"Expected ACPI signature {expected_signature!r}, found {data[:4]!r}."
        )
    declared_length = struct.unpack_from("<I", data, 4)[0]
    if declared_length != len(data):
        raise FixError(
            f"ACPI header length is {declared_length}, but file size is {len(data)}."
        )
    if sum(data) & 0xFF:
        raise FixError("ACPI table checksum is invalid before patching.")


def aml_uuid_bytes(uuid_text: str) -> bytes:
    """Return the byte order used by AML Buffer(ToUUID(...))."""
    return uuidlib.UUID(uuid_text).bytes_le


def decode_pkg_length(data: bytes, offset: int) -> tuple[int, int]:
    """Decode an AML PkgLength and return (length, encoded-byte-count).

    Per the ACPI spec, the lead byte's top 2 bits say how many extra bytes
    follow (0-3). With none, the whole lead byte (6 bits) is the length. With
    extra bytes, only the *low nibble* of the lead byte is used and the extra
    bytes each contribute 8 more bits above it -- the middle 2 bits of the
    lead byte are unused padding in that case.
    """
    if offset >= len(data):
        raise ValueError("PkgLength starts beyond end of data")
    lead = data[offset]
    follow = lead >> 6
    size = follow + 1
    if offset + size > len(data):
        raise ValueError("Truncated AML PkgLength")
    if follow == 0:
        return lead & 0x3F, 1
    length = lead & 0x0F
    for index in range(follow):
        length |= data[offset + 1 + index] << (4 + 8 * index)
    return length, size


def iter_simple_methods(data: bytes, name: bytes = b"_OSC") -> list[AmlMethod]:
    """Find MethodOp objects whose NameString is a simple four-byte NameSeg.

    This is a brute-force scan for the MethodOp byte at every offset, not a
    real namespace-aware AML walk -- deliberately so, since we don't care
    which scope a method lives in, only whether its body matches one of the
    documented broken _OSC shapes. A MethodOp byte that isn't really a
    MethodOp (e.g. it's the middle of some other object's data) will almost
    certainly fail the PkgLength/name/flags checks below and simply be
    skipped, and every method this returns is re-validated structurally by
    its caller before anything gets patched.
    """
    result: list[AmlMethod] = []
    for start in range(36, max(36, len(data) - 8)):
        if data[start] != 0x14:  # MethodOp
            continue
        try:
            pkg_length, pkg_size = decode_pkg_length(data, start + 1)
        except ValueError:
            continue
        end = start + 1 + pkg_length
        name_offset = start + 1 + pkg_size
        flags_offset = name_offset + 4
        if end > len(data) or flags_offset >= end:
            continue
        if data[name_offset : name_offset + 4] != name:
            continue
        result.append(
            AmlMethod(
                start=start,
                pkg_length=pkg_length,
                pkg_length_size=pkg_size,
                flags=data[flags_offset],
                body_start=flags_offset + 1,
                end=end,
            )
        )
    return result


class AmlCursor:
    def __init__(self, data: bytes, start: int = 0, end: int | None = None) -> None:
        self.data = data
        self.pos = start
        self.end = len(data) if end is None else end

    def need(self, count: int) -> None:
        if self.pos + count > self.end:
            raise ValueError("Unexpected end of AML object")

    def byte(self, expected: int | None = None) -> int:
        self.need(1)
        value = self.data[self.pos]
        if expected is not None and value != expected:
            raise ValueError(
                f"Expected AML opcode 0x{expected:02X} at +0x{self.pos:X}, "
                f"found 0x{value:02X}"
            )
        self.pos += 1
        return value

    def nameseg(self, expected: bytes | None = None) -> bytes:
        self.need(4)
        value = self.data[self.pos : self.pos + 4]
        if expected is not None and value != expected:
            raise ValueError(
                f"Expected NameSeg {expected!r} at +0x{self.pos:X}, found {value!r}"
            )
        self.pos += 4
        return value

    def integer(self) -> int:
        opcode = self.byte()
        if opcode == 0x00:
            return 0
        if opcode == 0x01:
            return 1
        if opcode == 0xFF:
            return 0xFFFFFFFFFFFFFFFF
        widths = {0x0A: 1, 0x0B: 2, 0x0C: 4, 0x0E: 8}
        if opcode not in widths:
            raise ValueError(f"Unsupported AML integer opcode 0x{opcode:02X}")
        width = widths[opcode]
        self.need(width)
        value = int.from_bytes(self.data[self.pos : self.pos + width], "little")
        self.pos += width
        return value

    def package_end(self) -> int:
        length, size = decode_pkg_length(self.data, self.pos)
        package_start = self.pos
        self.pos += size
        end = package_start + length
        if end > self.end or end < self.pos:
            raise ValueError("Invalid nested AML package length")
        return end


def parse_uuid_buffer(cursor: AmlCursor, expected_uuid: str) -> None:
    cursor.byte(0x11)  # BufferOp
    buffer_end = cursor.package_end()
    if cursor.integer() != 16:
        raise ValueError("UUID buffer does not declare a 16-byte size")
    expected = aml_uuid_bytes(expected_uuid)
    cursor.need(16)
    value = cursor.data[cursor.pos : cursor.pos + 16]
    cursor.pos += 16
    if value != expected:
        raise ValueError("UUID buffer does not contain the expected UUID")
    if cursor.pos != buffer_end:
        raise ValueError("Unexpected data in UUID buffer")


def read_uuid_buffer_bytes(cursor: AmlCursor) -> bytes:
    """Like parse_uuid_buffer(), but returns whatever 16-byte UUID is present
    instead of checking it against an expected value."""
    cursor.byte(0x11)  # BufferOp
    buffer_end = cursor.package_end()
    if cursor.integer() != 16:
        raise ValueError("Buffer does not declare a 16-byte size")
    cursor.need(16)
    value = cursor.data[cursor.pos : cursor.pos + 16]
    cursor.pos += 16
    if cursor.pos != buffer_end:
        raise ValueError("Unexpected data in UUID buffer")
    return value


def is_valid_nameseg(value: bytes) -> bool:
    """AML NameSegs are exactly 4 bytes: [A-Z_] followed by three [A-Z0-9_]."""
    if len(value) != 4:
        return False

    def ok(b: int, allow_digit: bool) -> bool:
        return 0x41 <= b <= 0x5A or b == 0x5F or (allow_digit and 0x30 <= b <= 0x39)

    return ok(value[0], False) and all(ok(b, True) for b in value[1:])


def find_named_uuid(data: bytes, nameseg: bytes) -> str:
    """Find Name(<nameseg>, Buffer(ToUUID(...))) anywhere in `data` and return
    which of the two documented _OSC UUIDs it holds.

    Used for the "Family B" DSDT shape, where the _OSC method compares Arg0
    against a pre-declared named UUID object instead of an inline literal
    (see parse_family_b_osc_prologue()). This only reads the Name(...)
    declaration; it never touches it.
    """
    marker = b"\x08" + nameseg  # NameOp + NameSeg
    matches: list[bytes] = []
    start = 0
    while True:
        idx = data.find(marker, start)
        if idx == -1:
            break
        try:
            value = read_uuid_buffer_bytes(AmlCursor(data, idx + len(marker)))
        except ValueError:
            start = idx + 1
            continue
        matches.append(value)
        start = idx + 1

    if not matches:
        raise ValueError(
            f"No Name({nameseg.decode('ascii', 'replace')}, Buffer(ToUUID(...))) "
            "declaration was found"
        )
    unique = set(matches)
    if len(unique) != 1:
        raise ValueError(
            f"Name {nameseg.decode('ascii', 'replace')!r} is declared with "
            f"{len(unique)} different UUID values"
        )
    value = next(iter(unique))
    for uuid_text in (UUID_SB_OSC, UUID_PCI0_OSC):
        if value == aml_uuid_bytes(uuid_text):
            return uuid_text
    raise ValueError(
        f"Name {nameseg.decode('ascii', 'replace')!r} holds UUID "
        f"{uuidlib.UUID(bytes_le=value)}, which is neither documented _OSC UUID"
    )


def parse_create_dword_field(
    cursor: AmlCursor,
    expected_index: int,
    expected_name: bytes,
) -> None:
    cursor.byte(0x8A)  # CreateDWordFieldOp
    cursor.byte(0x60)  # Local0
    if cursor.integer() != expected_index:
        raise ValueError(f"CreateDWordField index is not {expected_index}")
    cursor.nameseg(expected_name)


def parse_documented_broken_osc_body(body: bytes, uuid_text: str) -> None:
    """Accept the documented broken _OSC semantics, independent of Method PkgLength."""
    cursor = AmlCursor(body)
    cursor.byte(0xA0)  # IfOp
    if_end = cursor.package_end()
    if_cursor = AmlCursor(body, cursor.pos, if_end)
    if_cursor.byte(0x93)  # LEqualOp
    if_cursor.byte(0x68)  # Arg0
    parse_uuid_buffer(if_cursor, uuid_text)
    if_cursor.byte(0x70)  # StoreOp
    if_cursor.byte(0x6B)  # Arg3
    if_cursor.byte(0x60)  # Local0
    parse_create_dword_field(if_cursor, 0, b"CDW1")
    parse_create_dword_field(if_cursor, 4, b"CDW2")
    parse_create_dword_field(if_cursor, 8, b"CDW3")
    if if_cursor.pos != if_end:
        raise ValueError("Unexpected AML terms inside the _OSC If block")
    cursor.pos = if_end

    cursor.byte(0xA1)  # ElseOp
    else_end = cursor.package_end()
    else_cursor = AmlCursor(body, cursor.pos, else_end)
    else_cursor.byte(0x7D)  # OrOp
    else_cursor.nameseg(b"CDW1")
    if else_cursor.integer() != 4:
        raise ValueError("_OSC Else branch does not OR status bit 0x04")
    else_cursor.nameseg(b"CDW1")
    if else_cursor.pos != else_end:
        raise ValueError("Unexpected AML terms inside the _OSC Else block")
    cursor.pos = else_end

    cursor.byte(0xA4)  # ReturnOp
    cursor.byte(0x60)  # Local0
    while cursor.pos < cursor.end and body[cursor.pos] == 0xA3:  # tolerate firmware NoopOp padding
        cursor.pos += 1
    if cursor.pos != cursor.end:
        raise ValueError("Unexpected AML terms after Return(Local0)")


def fixed_osc_body(uuid_text: str, target_length: int) -> bytes:
    """Build the documented fixed method body and pad it without changing size."""
    logical = (
        b"\x70\x6B\x60"                 # Store(Arg3, Local0)
        + b"\x8A\x60\x00CDW1"           # CreateDWordField(Local0, 0, CDW1)
        + b"\xA0\x1F\x93\x68\x11\x13\x0A\x10"
        + aml_uuid_bytes(uuid_text)
        + b"\x8A\x60\x0A\x04CDW2"      # in-bounds CDW2 only
        + b"\xA1\x0C\x7DCDW1\x0A\x04CDW1"
        + b"\xA4\x60"
    )
    if len(logical) > target_length:
        raise FixError(
            f"The fixed _OSC body needs {len(logical)} bytes, but firmware provides "
            f"only {target_length}."
        )
    padding = b"\xA3" * (target_length - len(logical))
    # Keep Return(Local0) last and place harmless NoopOp padding immediately before it.
    return logical[:-2] + padding + logical[-2:]


def parse_family_b_osc_prologue(body: bytes) -> bytes:
    """Match Apple's PCI-hotplug-aware _OSC shape ("Family B"): Local0=Arg3
    and all three CreateDWordField calls -- including the out-of-bounds
    CDW3 -- happen unconditionally before any UUID check, and the UUID
    comparison further down references a named Buffer(ToUUID(...)) object
    (resolved separately via find_named_uuid()) instead of an inline
    literal. Real firmware from MacBookPro15,1, MacBookPro16,1 and
    Macmini8,1 uses this exact 193-byte method body, implementing NHPG/NPME
    hotplug notifications and OSDW/OSCC/CTRL/SUPP negotiation around the
    same CDW3 overflow bug the simple "Family A" shape has.

    Returns the 4-byte NameSeg compared against Arg0.
    """
    cursor = AmlCursor(body)
    cursor.byte(0x70)  # StoreOp
    cursor.byte(0x6B)  # Arg3
    cursor.byte(0x60)  # Local0
    parse_create_dword_field(cursor, 0, b"CDW1")
    parse_create_dword_field(cursor, 4, b"CDW2")
    parse_create_dword_field(cursor, 8, b"CDW3")
    prologue_end = cursor.pos

    rest = body[prologue_end:]
    names: set[bytes] = set()
    search_from = 0
    marker = b"\x93\x68"  # LEqualOp, Arg0
    while True:
        idx = rest.find(marker, search_from)
        if idx == -1:
            break
        nameseg = rest[idx + 2 : idx + 6]
        if is_valid_nameseg(nameseg):
            names.add(nameseg)
        search_from = idx + 1

    if not names:
        raise ValueError(
            "No Arg0-compared name reference found after the unconditional prologue"
        )
    if len(names) > 1:
        sorted_names = sorted(n.decode("ascii", "replace") for n in names)
        raise ValueError(f"Multiple different names compared against Arg0: {sorted_names}")
    return next(iter(names))


def _pkg_length_byte(payload_len: int) -> bytes:
    # +1 because PkgLength counts its own encoded byte(s) too (see
    # decode_pkg_length's docstring). 0x3F is the largest length a single
    # PkgLength byte can hold (top 2 bits must stay 0 to mean "no extra
    # bytes follow") -- comfortably more than our fixed skeleton ever needs.
    total = payload_len + 1
    if total > 0x3F:
        raise FixError("Internal error: fixed _OSC skeleton package exceeds 1-byte PkgLength.")
    return bytes([total])


def fixed_osc_body_named(nameseg: bytes, target_length: int) -> bytes:
    """Build the fixed method body for the "Family B" shape: the same
    minimal documented-fix skeleton as fixed_osc_body(), but comparing Arg0
    against the pre-existing named UUID object instead of an inline literal
    -- the Name(...) declaration itself lives outside this method body and
    is left untouched."""
    if_payload = b"\x93\x68" + nameseg + b"\x8A\x60\x0A\x04CDW2"
    else_payload = b"\x7DCDW1\x0A\x04CDW1"
    logical = (
        b"\x70\x6B\x60"  # Store(Arg3, Local0)
        + b"\x8A\x60\x00CDW1"  # CreateDWordField(Local0, 0, CDW1)
        + b"\xA0" + _pkg_length_byte(len(if_payload)) + if_payload
        + b"\xA1" + _pkg_length_byte(len(else_payload)) + else_payload
        + b"\xA4\x60"  # Return(Local0)
    )
    if len(logical) > target_length:
        raise FixError(
            f"The fixed _OSC body needs {len(logical)} bytes, but firmware provides "
            f"only {target_length}."
        )
    padding = b"\xA3" * (target_length - len(logical))
    return logical[:-2] + padding + logical[-2:]


def find_osc_replacements(
    data: bytes,
) -> list[tuple[str, AmlMethod, str, bytes | None]]:
    """Find every documented-broken _OSC method for either documented UUID,
    in either of the two AML shapes real T2 firmware uses ("Family A":
    parse_documented_broken_osc_body(); "Family B":
    parse_family_b_osc_prologue()).

    Real hardware only ever has a method for one or both of the two
    documented UUIDs, in one shape or the other -- not necessarily both --
    so this returns whatever it can resolve rather than demanding both up
    front; patch_dsdt_aml() decides whether the result is enough to
    proceed. A method that doesn't match either shape at all is silently
    skipped (with a diagnostic kept for the final error message if nothing
    at all resolves); unrecognized DSDTs are rejected rather than patched speculatively.
    """
    resolved: dict[str, tuple[AmlMethod, str, bytes | None]] = {}
    diagnostics: list[str] = []

    for method in iter_simple_methods(data, b"_OSC"):
        if method.flags != 0x0C:
            diagnostics.append(
                f"_OSC at 0x{method.start:X}: unexpected Method flags 0x{method.flags:02X}"
            )
            continue
        body = data[method.body_start : method.end]

        match: tuple[str, str, bytes | None] | None = None
        for uuid_text in (UUID_SB_OSC, UUID_PCI0_OSC):
            if aml_uuid_bytes(uuid_text) not in body:
                continue
            try:
                parse_documented_broken_osc_body(body, uuid_text)
            except ValueError as exc:
                diagnostics.append(f"_OSC at 0x{method.start:X} ({uuid_text}, family A): {exc}")
                continue
            match = (uuid_text, "A", None)
            break

        if match is None:
            try:
                nameseg = parse_family_b_osc_prologue(body)
            except ValueError as exc:
                diagnostics.append(f"_OSC at 0x{method.start:X} (family B): {exc}")
                continue
            try:
                uuid_text = find_named_uuid(data, nameseg)
            except ValueError as exc:
                diagnostics.append(
                    f"_OSC at 0x{method.start:X} (family B, name "
                    f"{nameseg.decode('ascii', 'replace')!r}): {exc}"
                )
                continue
            match = (uuid_text, "B", nameseg)

        uuid_text, kind, extra = match
        if uuid_text in resolved:
            raise FixError(
                f"Multiple documented broken _OSC methods resolved to UUID {uuid_text}: "
                f"0x{resolved[uuid_text][0].start:X} and 0x{method.start:X}."
            )
        resolved[uuid_text] = (method, kind, extra)

    if not resolved:
        details = "; ".join(diagnostics) if diagnostics else "no _OSC methods found at all"
        raise FixError(
            "No _OSC method matched either documented broken AML shape for either "
            f"documented UUID ({details}). This model's DSDT is not patched automatically."
        )

    return [(uuid_text, method, kind, extra) for uuid_text, (method, kind, extra) in resolved.items()]


def patch_dsdt_aml(
    original: bytes,
) -> tuple[bytes, list[tuple[str, int, int, str, str]], int, int]:
    """Apply all semantically validated _OSC replacements and refresh the header.

    Returns (patched_bytes, replacements, old_revision, new_revision), where
    each replacement is (uuid_text, method_start, method_length, kind, anchor)
    -- anchor is the UUID text for "Family A" methods or the compared NameSeg
    (e.g. "GUID") for "Family B" ones, and is what an independent disassembly
    check can search for to relocate the same method in decompiled text.
    """
    validate_acpi_binary(original, b"DSDT")
    resolved = find_osc_replacements(original)

    def fixed_body_for(kind: str, uuid_text: str, extra: bytes | None, length: int) -> bytes:
        if kind == "A":
            return fixed_osc_body(uuid_text, length)
        assert extra is not None
        return fixed_osc_body_named(extra, length)

    patched = bytearray(original)
    replacements: list[tuple[str, int, int, str, str]] = []
    for uuid_text, method, kind, extra in resolved:
        replacement = fixed_body_for(kind, uuid_text, extra, method.body_length)
        patched[method.body_start : method.end] = replacement
        anchor = uuid_text if kind == "A" else extra.decode("ascii")
        replacements.append((uuid_text, method.start, method.end - method.start, kind, anchor))

    # Bumping the OEM revision (header offset 24, a 4-byte LE field) is what
    # makes the kernel prefer this override over the firmware's own DSDT.
    old_revision = struct.unpack_from("<I", patched, 24)[0]
    if old_revision >= 0xFFFFFFFF:
        raise FixError("DSDT OEM revision cannot be incremented beyond 0xFFFFFFFF.")
    new_revision = old_revision + 1
    struct.pack_into("<I", patched, 24, new_revision)

    # ACPI tables are valid iff the unsigned byte-sum of the whole table is
    # 0 mod 256. Zero the checksum byte (header offset 9) first so it doesn't
    # contribute its old value to its own recomputation.
    patched[9] = 0
    patched[9] = (-sum(patched)) & 0xFF
    result = bytes(patched)

    if len(result) != len(original):
        raise FixError("Internal validation failed: DSDT size changed.")
    if struct.unpack_from("<I", result, 4)[0] != len(result):
        raise FixError("Internal validation failed: DSDT header length changed or is invalid.")
    if sum(result) & 0xFF:
        raise FixError("Internal validation failed: patched DSDT checksum is invalid.")

    for uuid_text, method, kind, extra in resolved:
        expected = fixed_body_for(kind, uuid_text, extra, method.body_length)
        if result[method.body_start : method.end] != expected:
            raise FixError(f"Internal validation failed for patched _OSC {uuid_text}.")

    # Belt-and-suspenders: confirm byte-for-byte that nothing changed outside
    # the patched method bodies and the header fields we intentionally
    # touched above. A bug in the slice arithmetic above would corrupt
    # unrelated parts of the table; this catches that before deployment
    # rather than trusting the slicing was correct.
    changed = [index for index, (before, after) in enumerate(zip(original, result)) if before != after]
    allowed: set[int] = {9} | set(range(24, 28))
    for _, method, _, _ in resolved:
        allowed.update(range(method.body_start, method.end))
    unexpected = [index for index in changed if index not in allowed]
    if unexpected:
        preview = ", ".join(hex(index) for index in unexpected[:8])
        raise FixError(f"Internal validation failed: unexpected DSDT byte changes at {preview}.")

    return result, replacements, old_revision, new_revision


# ---------------------------------------------------------------------------
# Independent validation of the DSDT patch: re-decompile the patched table
# with iasl and check the *text* shows the fix, using regexes that are
# deliberately not shared with the AmlCursor byte parser above. A bug that
# exists in both the byte-level patcher and this checker would slip through
# either way, but a bug in just one of them will not.
# ---------------------------------------------------------------------------


def locate_osc_method_body(text: str, anchor: str, *, source_label: str) -> str:
    """Return the brace-delimited body of the _OSC method whose body contains
    `anchor` -- either a UUID string (the "Family A" inline-literal shape) or
    a bare NameSeg like "GUID" (the "Family B" named-reference shape).

    Scans every `Method (_OSC ...)` in the text and returns the first whose
    body contains the anchor, rather than searching backward from a single
    occurrence of the anchor in the whole text: for Family B, the anchor
    NameSeg's own Name(...) declaration textually precedes the method, so a
    backward search from that declaration would never find the method at all.
    """
    clean = strip_asl_comments(text)
    anchor_re = re.compile(re.escape(anchor), re.IGNORECASE)
    method_pattern = re.compile(r"Method\s*\(\s*_OSC\b")
    for match in method_pattern.finditer(clean):
        brace_open = clean.find("{", match.end())
        if brace_open == -1:
            continue
        brace_close = find_matching_brace(clean, brace_open)
        body = clean[match.start() : brace_close + 1]
        if anchor_re.search(body):
            return body
    raise FixError(
        f"Could not locate an _OSC Method in {source_label} whose body references "
        f"{anchor!r}."
    )


def assert_disassembled_osc_is_fixed(text: str, anchor: str, *, source_label: str) -> None:
    """Confirm, via iasl's own decompiler, that the patched method matches the
    documented fix: CDW1 created before the If, CDW2 only inside the If,
    CDW1 |= 0x04 in the Else, and no CDW3 anywhere."""
    body = locate_osc_method_body(text, anchor, source_label=source_label)

    if re.search(r"\bCDW3\b", body):
        raise FixError(
            f"{source_label} still shows CDW3 for _OSC ({anchor}) after patching; "
            "the text rewrite did not take effect as intended."
        )

    create_cdw1 = re.search(
        r"CreateDWordField\s*\(\s*Local0\s*,\s*Zero\s*,\s*CDW1\s*\)", body, re.IGNORECASE
    )
    if not create_cdw1:
        raise FixError(
            f"{source_label} does not show CDW1 being created for _OSC ({anchor}) "
            "after patching."
        )

    if_match = re.search(r"\bIf\s*\(", body, re.IGNORECASE)
    if not if_match or if_match.start() < create_cdw1.start():
        raise FixError(
            f"{source_label} does not show CDW1 being created before the If block "
            f"for _OSC ({anchor}) after patching; both branches must see CDW1."
        )

    if not re.search(
        rf"CreateDWordField\s*\(\s*Local0\s*,\s*{_OFFSET_4_RE}\s*,\s*CDW2\s*\)",
        body,
        re.IGNORECASE,
    ):
        raise FixError(
            f"{source_label} does not show CDW2 being created inside the If branch "
            f"for _OSC ({anchor}) after patching."
        )

    if not re.search(
        rf"CDW1\s*\|=\s*{_OFFSET_4_RE}|Or\s*\(\s*CDW1\s*,\s*{_OFFSET_4_RE}\s*,\s*CDW1\s*\)",
        body,
        re.IGNORECASE,
    ):
        raise FixError(
            f"{source_label} does not show the Else branch OR'ing CDW1 with 0x04 "
            f"for _OSC ({anchor}) after patching."
        )


def assert_disassembled_osc_is_the_documented_bug(
    text: str, anchor: str, *, source_label: str
) -> None:
    """Sanity-check that the *unmodified* firmware DSDT still shows the
    documented CDW3 overflow, so we know the baseline we are comparing
    against is actually the bug the patch targets."""
    body = locate_osc_method_body(text, anchor, source_label=source_label)
    if not re.search(r"\bCDW3\b", body):
        raise FixError(
            f"Independent iasl disassembly of {source_label} does not show the "
            f"documented CDW3 buffer overflow for _OSC ({anchor}); the AML-level "
            "patch and the disassembly view of this table disagree about the "
            "original shape, refusing to deploy."
        )


def disassemble_only(aml: Path) -> tuple[str, int, int]:
    """Run `iasl -d` as an independent structural check; never recompiles.

    This is deliberately not a full `-tc` round-trip: recompiling an Apple
    DSDT from scratch is unreliable across models (unrelated constructs like
    legacy VarPackage brightness tables can fail to round-trip even though
    they have nothing to do with the _OSC fix). Disassembly alone is a much
    weaker requirement -- iasl's decompiler tolerates forward references and
    duplicate namespace objects that its compiler rejects -- but it is still
    an independent second parser of the AML patch_dsdt_aml() hand-built,
    distinct from the AmlCursor code above.
    """
    proc = run(["iasl", "-d", aml.name], cwd=aml.parent, check=False)
    dsl = aml.with_suffix(".dsl")
    if not dsl.is_file():
        raise FixError(
            f"iasl could not disassemble {aml.name} for independent validation; "
            f"refusing to trust the binary patch: {output_summary(proc.stdout)}"
        )
    summaries = re.findall(r"(\d+)\s+Errors?,\s+(\d+)\s+Warnings?", proc.stdout, re.I)
    errors, warnings = (int(value) for value in summaries[-1]) if summaries else (0, 0)
    text = dsl.read_text(encoding="utf-8", errors="replace")
    return text, errors, warnings


def validate_patched_dsdt_with_iasl(
    baseline_aml: Path,
    patched_aml: Path,
    replacements: Sequence[tuple[str, int, int, str, str]],
) -> None:
    """Independently re-check the hand-built AML patch using iasl's decompiler.

    This does not replace the byte-level checks in patch_dsdt_aml(); it is a
    second, differently-implemented opinion. It compares the patched table
    against the machine's own unmodified DSDT rather than a fixed expectation,
    so pre-existing firmware quirks (unresolved externals, vendor remarks)
    don't cause false failures as long as the patch doesn't add new ones.

    `replacements` is patch_dsdt_aml()'s return value: each entry's `anchor`
    (last element) is what locates the same method in decompiled text --
    the UUID string for "Family A" methods, or the compared NameSeg (e.g.
    "GUID") for "Family B" ones.
    """
    baseline_text, baseline_errors, baseline_warnings = disassemble_only(baseline_aml)
    patched_text, patched_errors, patched_warnings = disassemble_only(patched_aml)

    if patched_errors > baseline_errors:
        raise FixError(
            "Independent iasl disassembly reports more errors for the patched DSDT "
            f"({patched_errors}) than for the unmodified firmware DSDT ({baseline_errors}); "
            "refusing to deploy."
        )
    if patched_warnings > baseline_warnings:
        raise FixError(
            "Independent iasl disassembly reports more warnings for the patched DSDT "
            f"({patched_warnings}) than for the unmodified firmware DSDT "
            f"({baseline_warnings}); refusing to deploy."
        )

    for _, _, _, _, anchor in replacements:
        assert_disassembled_osc_is_the_documented_bug(
            baseline_text, anchor, source_label="the unmodified DSDT"
        )
        assert_disassembled_osc_is_fixed(
            patched_text, anchor, source_label="the patched DSDT"
        )

# ---------------------------------------------------------------------------
# Building each table: compile the patched CpuSSDT ASL, and drive the DSDT
# byte-patch + independent validation above into a deployable BuiltTable each
# ---------------------------------------------------------------------------


def write_patched_dsl(path: Path, patched: str) -> None:
    path.write_text(patched, encoding="utf-8", newline="\n")
    path.chmod(0o600)


def compile_dsl(dsl: Path) -> CompileResult:
    proc = run(
        ["iasl", "-tc", dsl.name],
        cwd=dsl.parent,
        check=False,
    )
    summaries = re.findall(
        r"(\d+)\s+Errors?,\s+(\d+)\s+Warnings?",
        proc.stdout,
        re.I,
    )
    if not summaries:
        raise FixError("Could not verify the iasl error/warning summary; refusing deployment.")
    errors, warnings = (int(value) for value in summaries[-1])
    warning_codes: Counter[str] = Counter(
        re.findall(r"^Warning\s+(\d+)\s+-", proc.stdout, re.MULTILINE)
    )
    aml = dsl.with_suffix(".aml")
    if errors == 0 and (not aml.is_file() or aml.stat().st_size == 0):
        raise FixError(f"Compiled AML file is missing or empty: {aml}")
    return CompileResult(aml, errors, warnings, warning_codes)


def require_clean_compile(result: CompileResult, table_name: str) -> Path:
    if result.errors != 0 or result.warnings != 0:
        raise FixError(
            f"iasl reported {result.errors} error(s) and {result.warnings} warning(s) "
            f"while compiling {table_name}; expected 0 Errors and 0 Warnings."
        )
    return result.aml

def cpussdt_deploy_name(product_name: str) -> str:
    match = re.search(r"(?:MacBookPro|MacBookAir|Macmini|iMacPro|iMac|MacPro)(\d+),(\d+)", product_name, re.I)
    if match:
        name = f"{match.group(1)}{match.group(2)}CpuSSDT.aml"
    else:
        name = "CpuSSDT.aml"
    if len(name) > 17:
        raise FixError(f"Generated CpuSSDT filename exceeds 17 characters: {name}")
    return name


def build_cpussdt(workdir: Path, product_name: str) -> BuiltTable:
    workdir = workdir / "cpussdt"
    workdir.mkdir(mode=0o700)
    source = find_cpussdt_table()
    local_source, dsl = copy_and_disassemble(source, workdir)
    original_copy = Path(str(dsl) + ".orig")
    shutil.copy2(dsl, original_copy)
    original = dsl.read_text(encoding="utf-8", errors="strict")
    loaded_ids = loaded_ssdt_table_ids()
    patched, mask, mappings = patch_cpussdt_text(original, loaded_ids)
    write_patched_dsl(dsl, patched)
    aml = require_clean_compile(compile_dsl(dsl), "CpuSSDT")
    report = workdir / "CpuSSDT.patch-report.txt"
    report_lines = [
        "T2 ACPI CpuSSDT patch report",
        "",
        f"Source: {source}",
        f"Derived SDTL mask: 0x{mask:08X}",
        "",
        "Bits set only for SSDT package entries whose OEM Table ID is already",
        "present as a kernel-loaded SSDT:",
    ]
    for label, bit, region, files in mappings:
        report_lines.append(f"  {label}: bit 0x{bit:X}, region {region}, files {files}")
    report.write_text("\n".join(report_lines) + "\n", encoding="utf-8")
    report.chmod(0o600)
    return BuiltTable("CpuSSDT", local_source, report, aml, cpussdt_deploy_name(product_name))


def build_dsdt(workdir: Path) -> BuiltTable:
    dsdt_dir = workdir / "dsdt"
    dsdt_dir.mkdir(mode=0o700)
    source = ACPI_TABLE_DIR / "DSDT"
    if not source.is_file():
        raise FixError(f"DSDT table is missing: {source}")

    local_source = dsdt_dir / "DSDT.original.aml"
    shutil.copy2(source, local_source)
    local_source.chmod(0o600)
    original = local_source.read_bytes()

    patched, replacements, old_revision, new_revision = patch_dsdt_aml(original)
    aml = dsdt_dir / "DSDT.patched.aml"
    aml.write_bytes(patched)
    aml.chmod(0o600)

    validate_patched_dsdt_with_iasl(local_source, aml, replacements)

    family_names = {"A": "inline UUID literal", "B": "PCI-hotplug, named UUID reference"}
    report = dsdt_dir / "DSDT.patch-report.txt"
    lines = [
        "T2 ACPI DSDT binary patch report",
        "",
        f"Source: {source}",
        "Patched directly in AML bytes (not via disassemble/edit-ASL/recompile,",
        "which is unreliable across models for the reasons explained in this",
        "script's module docstring). Verified by an independent iasl disassembly",
        "(no recompile) rather than trusting only the hand-written AML parser.",
        "",
        f"OEM revision: 0x{old_revision:08X} -> 0x{new_revision:08X}",
        "",
        f"{len(replacements)} of the 2 documented _OSC UUIDs had a matching method on",
        "this DSDT (some models only implement one of the two -- that is normal, not",
        "an error). Each matched method body was replaced so that:",
        "  - Store(Arg3, Local0) and CreateDWordField(Local0, Zero, CDW1) run before",
        "    the UUID comparison, so both branches can see CDW1",
        "  - CreateDWordField(Local0, 0x04, CDW2) is the only field left inside the If",
        "  - CreateDWordField(Local0, 0x08, CDW3) (the out-of-bounds field) is removed",
        "  - the Else branch (CDW1 |= 0x04) is unchanged",
        "  - the method's total AML byte length, and every enclosing package length,",
        "    is unchanged; the freed space is backfilled with NoopOp padding",
    ]
    for uuid_text, start, length, kind, anchor in replacements:
        lines.append(
            f"  {uuid_text}: method at file offset 0x{start:X}, {length} bytes, "
            f"shape {kind} ({family_names[kind]}, anchor {anchor!r})"
        )
    report.write_text("\n".join(lines) + "\n", encoding="utf-8")
    report.chmod(0o600)

    return BuiltTable("DSDT", local_source, report, aml, "dsdt.aml")


# ---------------------------------------------------------------------------

def backup_path(path: Path, backup_dir: Path) -> None:
    if not path.exists() and not path.is_symlink():
        return
    relative = path.relative_to("/")
    destination = backup_dir / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    if path.is_dir():
        shutil.copytree(path, destination, symlinks=True)
    else:
        shutil.copy2(path, destination, follow_symlinks=False)


def atomic_copy(source: Path, destination: Path, mode: int = 0o644) -> None:
    # Write to a temp file and rename over the target: a crash or power loss
    # mid-write can never leave a half-written ACPI override in place, since
    # rename() is atomic and the old file stays intact until it succeeds.
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp-acpi-fix")
    shutil.copyfile(source, temporary)
    temporary.chmod(mode)
    os.replace(temporary, destination)


def updated_dracut_conf(existing: str, enable: bool) -> str:
    retained: list[str] = []
    setting_re = re.compile(r"^\s*(acpi_override|acpi_table_dir)\s*=")
    for line in existing.splitlines():
        if not setting_re.match(line):
            retained.append(line)
    while retained and not retained[-1].strip():
        retained.pop()
    if enable and retained:
        retained.append("")
    if enable:
        retained.extend(DRACUT_REQUIRED_LINES)
    return "\n".join(retained) + ("\n" if retained else "")


def write_atomic_text(path: Path, text: str, mode: int = 0o644) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp-acpi-fix")
    temporary.write_text(text, encoding="utf-8", newline="\n")
    temporary.chmod(mode)
    os.replace(temporary, path)


def restore_file(path: Path, backup_dir: Path, existed_before: bool) -> None:
    backup = backup_dir / path.relative_to("/")
    if existed_before:
        if not backup.exists() and not backup.is_symlink():
            raise FixError(f"Backup expected but missing during rollback: {backup}")
        path.parent.mkdir(parents=True, exist_ok=True)
        if backup.is_dir():
            if path.exists():
                shutil.rmtree(path)
            shutil.copytree(backup, path, symlinks=True)
        else:
            shutil.copy2(backup, path, follow_symlinks=False)
    else:
        try:
            path.unlink()
        except FileNotFoundError:
            pass


def check_cpussdt_duplicates(target_name: str) -> None:
    if not DEPLOY_DIR.is_dir():
        return
    duplicates = [
        p for p in DEPLOY_DIR.iterdir()
        if p.is_file() and p.name.lower().endswith("cpussdt.aml") and p.name != target_name
    ]
    if duplicates:
        joined = ", ".join(str(p) for p in duplicates)
        raise FixError(
            "An existing differently named CpuSSDT override could cause duplicate table loading. "
            f"Move or remove it first: {joined}"
        )


# ---------------------------------------------------------------------------

def deploy_tables(
    tables: Sequence[BuiltTable], timestamp: str, product_name: str
) -> Path:
    backup_dir = BACKUP_ROOT / timestamp
    backup_dir.mkdir(parents=True, exist_ok=False)
    backup_dir.chmod(0o700)

    DEPLOY_DIR.mkdir(parents=True, exist_ok=True)
    DEPLOY_DIR.chmod(0o755)

    cpussdt = next((table for table in tables if table.kind == "CpuSSDT"), None)
    if cpussdt:
        check_cpussdt_duplicates(cpussdt.deploy_name)

    targets = [DEPLOY_DIR / table.deploy_name for table in tables]
    desired_targets = set(targets)
    managed_targets = {
        DEPLOY_DIR / "dsdt.aml",
        DEPLOY_DIR / cpussdt_deploy_name(product_name),
    }
    obsolete_targets = sorted(managed_targets - desired_targets)
    tracked = sorted(desired_targets | set(obsolete_targets)) + [DRACUT_CONF]
    existed_before = {path: path.exists() or path.is_symlink() for path in tracked}
    for path in tracked:
        backup_path(path, backup_dir)

    manifest = backup_dir / "MANIFEST.txt"
    manifest.write_text(
        "KaiT2en ACPI backup created before deployment.\n"
        + "\n".join(f"{path}: existed={existed_before[path]}" for path in tracked)
        + "\n",
        encoding="utf-8",
    )
    manifest.chmod(0o600)

    try:
        for table, target in zip(tables, targets):
            atomic_copy(table.aml, target)
        for target in obsolete_targets:
            target.unlink(missing_ok=True)

        existing = (
            DRACUT_CONF.read_text(encoding="utf-8", errors="replace")
            if DRACUT_CONF.exists()
            else ""
        )
        updated_conf = updated_dracut_conf(existing, bool(tables))
        if updated_conf:
            write_atomic_text(DRACUT_CONF, updated_conf)
        else:
            DRACUT_CONF.unlink(missing_ok=True)
    except Exception as original_error:
        rollback_errors: list[str] = []
        for path in reversed(tracked):
            try:
                restore_file(path, backup_dir, existed_before[path])
            except Exception as rollback_error:
                rollback_errors.append(f"{path}: {rollback_error}")

        detail = ""
        if rollback_errors:
            detail = "; rollback also failed for " + "; ".join(rollback_errors)
        raise FixError(
            f"deployment failed and rollback was attempted: {original_error}{detail}"
        ) from original_error

    return backup_dir


def main() -> int:
    if len(sys.argv) != 1:
        raise FixError("this installer component does not accept command-line arguments")

    require_root()
    require_fedora()
    product_name = require_apple_intel_supported()

    info("checking whether ACPI firmware fixes are needed")
    detection = detect_problems()
    timestamp = safe_timestamp()
    tables: list[BuiltTable] = []
    if detection.cpussdt_problem or detection.dsdt_problem:
        require_commands(("iasl",))
        workdir = make_workdir(timestamp)
        if detection.cpussdt_problem:
            tables.append(build_cpussdt(workdir, product_name))
        if detection.dsdt_problem:
            tables.append(build_dsdt(workdir))

    deploy_tables(tables, timestamp, product_name)
    if not tables:
        info("ACPI firmware fixes are not needed; removed obsolete KaiT2en overrides")
    else:
        kinds = ", ".join(table.kind for table in tables)
        info(f"ACPI firmware fixes prepared for initramfs rebuild: {kinds}")
    return 0


def print_failure(message: str) -> None:
    print(f"[kait2en] ACPI autofix could not be applied: {message}", file=sys.stderr)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
    except FixError as exc:
        print_failure(str(exc))
        raise SystemExit(1)
    except OSError as exc:
        print_failure(f"operating-system error: {exc}")
        raise SystemExit(1)
    except Exception as exc:
        print_failure(f"unexpected {type(exc).__name__}: {exc}")
        raise SystemExit(1)

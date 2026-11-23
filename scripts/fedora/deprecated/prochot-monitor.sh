#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
# prochot-monitor.sh
# André Eikmeyer Aug,2026
# Live display of:
#  - IA32_THERM_STATUS (MSR 0x19C) per core: PROCHOT/FORCEPR event + log, digital readout
#  - MSR_POWER_CTL (0x1FC) bit 0: whether BD_PROCHOT (external throttle signal) is enabled
#  - MSR_PKG_POWER_LIMIT (0x610): RAPL PL1/PL2 power limits and whether they are enabled
#  - MSR_PKG_ENERGY_STATUS (0x611): current average package power draw
#
# Missing Fedora runtime dependencies are installed automatically.

set -u

THERM_MSR=0x19C
TEMP_TARGET_MSR=0x1A2
POWER_CTL_MSR=0x1FC
RAPL_UNIT_MSR=0x606
PKG_POWER_LIMIT_MSR=0x610
PKG_ENERGY_MSR=0x611
PERF_LIMIT_MSR=0x64F

if [ "$(id -u)" -ne 0 ]; then
    echo "Please run as root (sudo $0)." >&2
    exit 1
fi

MISSING_PACKAGES=()
command -v rdmsr >/dev/null || MISSING_PACKAGES+=(msr-tools)
command -v modprobe >/dev/null || MISSING_PACKAGES+=(kmod)
if ! command -v tput >/dev/null || ! command -v clear >/dev/null; then
    MISSING_PACKAGES+=(ncurses)
fi
command -v awk >/dev/null || MISSING_PACKAGES+=(gawk)

if (( ${#MISSING_PACKAGES[@]} )); then
    if ! command -v dnf >/dev/null; then
        echo "Missing runtime dependencies and dnf is unavailable." >&2
        exit 1
    fi

    printf 'Installing missing dependencies: %s\n' "${MISSING_PACKAGES[*]}"
    dnf install -y "${MISSING_PACKAGES[@]}"
fi

modprobe msr

NCPU=$(nproc)

TEMP_TARGET_HEX=$(rdmsr -p 0 "$TEMP_TARGET_MSR")
TEMP_TARGET_DEC=$((16#$TEMP_TARGET_HEX))
TJMAX=$(( (TEMP_TARGET_DEC >> 16) & 0xFF ))
TCC_OFFSET=$(( (TEMP_TARGET_DEC >> 24) & 0xF ))
THERMAL_SAMPLES=3

# RAPL units are fixed at boot, we read them once. Power unit and energy unit are given as
# negative powers of two (exponents), so we keep them as divisors to stay in integer math.
RAPL_UNIT_HEX=$(rdmsr -p 0 "$RAPL_UNIT_MSR")
if [ -n "$RAPL_UNIT_HEX" ]; then
    RAPL_UNIT_DEC=$((16#$RAPL_UNIT_HEX))
    PU_EXP=$(( RAPL_UNIT_DEC & 0xF ))
    ESU_EXP=$(( (RAPL_UNIT_DEC >> 8) & 0x1F ))
    PU_DIV=$(( 1 << PU_EXP ))
    ESU_DIV=$(( 1 << ESU_EXP ))
else
    PU_DIV=8
    ESU_DIV=65536
fi

# Prepare terminal
tput civis
trap 'tput cnorm; exit 0' INT TERM
RED=$(tput setaf 1)
RESET=$(tput sgr0)

FIRST=1
PREV_ENERGY=""
PREV_TIME=""

# Print a value already scaled by 10 (i.e. tenths) as X.Y
fmt_tenths() {
    local tenths=$1
    printf "%d.%d" $((tenths / 10)) $((tenths % 10))
}

join_reasons() {
    local result="" reason

    for reason in "$@"; do
        if [[ -n $result ]]; then
            result+=", "
        fi
        result+=$reason
    done
    printf '%s' "$result"
}

while true; do
    if [ "$FIRST" -eq 1 ]; then
        clear
        FIRST=0
    else
        tput cup 0 0
    fi

    echo "Prochot/Forcepr Live Monitor    $(date '+%H:%M:%S')"
    echo "Ctrl+C to quit"
    echo

    # --- Package-level info: BD_PROCHOT enable + RAPL power limits + current draw ---
    CTL_HEX=$(rdmsr -p 0 "$POWER_CTL_MSR")
    if [ -n "$CTL_HEX" ]; then
        CTL_DEC=$((16#$CTL_HEX))
        BD_EN=$(( CTL_DEC & 1 ))
        BD_TXT="disabled"
        [ "$BD_EN" -eq 1 ] && BD_TXT="ENABLED (external throttle signal accepted)"
    else
        BD_TXT="? (read error)"
    fi
    echo "BD_PROCHOT (MSR 0x1FC bit0): $BD_TXT"
    echo "TjMax: ${TJMAX} C    TCC offset: ${TCC_OFFSET} C    thermal threshold: $((TJMAX - TCC_OFFSET)) C"

    LIM_HEX=$(rdmsr -p 0 "$PKG_POWER_LIMIT_MSR")
    if [ -n "$LIM_HEX" ]; then
        LIM_DEC=$((16#$LIM_HEX))
        PL1_RAW=$(( LIM_DEC & 0x7FFF ))
        PL1_EN=$(( (LIM_DEC >> 15) & 1 ))
        PL2_RAW=$(( (LIM_DEC >> 32) & 0x7FFF ))
        PL2_EN=$(( (LIM_DEC >> 47) & 1 ))

        PL1_W=$(fmt_tenths $(( PL1_RAW * 10 / PU_DIV )))
        PL2_W=$(fmt_tenths $(( PL2_RAW * 10 / PU_DIV )))

        PL1_TXT="disabled"
        [ "$PL1_EN" -eq 1 ] && PL1_TXT="enabled"
        PL2_TXT="disabled"
        [ "$PL2_EN" -eq 1 ] && PL2_TXT="enabled"

        echo "PL1 (sustained): ${PL1_W} W ($PL1_TXT)    PL2 (burst): ${PL2_W} W ($PL2_TXT)"
    else
        echo "PL1/PL2: ? (read error)"
    fi

    ENE_HEX=$(rdmsr -p 0 "$PKG_ENERGY_MSR")
    CUR_W_TXT="?"
    if [ -n "$ENE_HEX" ]; then
        ENE_DEC=$((16#$ENE_HEX))
        NOW=$(date +%s)
        if [ -n "$PREV_ENERGY" ] && [ -n "$PREV_TIME" ]; then
            DE=$(( ENE_DEC - PREV_ENERGY ))
            if [ "$DE" -lt 0 ]; then
                DE=$(( DE + 4294967296 ))   # 32-bit counter wraparound
            fi
            DT=$(( NOW - PREV_TIME ))
            if [ "$DT" -gt 0 ]; then
                CUR_W_TXT=$(fmt_tenths $(( DE * 10 / (ESU_DIV * DT) )))"W "
            fi
        fi
        PREV_ENERGY=$ENE_DEC
        PREV_TIME=$NOW
    fi
    echo "Current package power draw: $CUR_W_TXT"
    echo

    THERM_VALID=()
    THERM_STATUS=()
    DIGITAL_READOUT=()
    ANY_PROCHOT=0
    ANY_PROCHOT_LOG=0
    ANY_THERMAL=0
    ANY_POWER_LIMIT=0
    ANY_POWER_LIMIT_LOG=0
    MIN_DTJ=127
    PERF_ACTIVE=0
    PERF_LOG=0

    # Multiple quick passes reduce the chance of missing a brief thermal
    # assertion while the logical CPUs are read sequentially.
    for ((sample = 0; sample < THERMAL_SAMPLES; sample++)); do
        for ((i = 0; i < NCPU; i++)); do
            VAL_HEX=$(rdmsr -p "$i" "$THERM_MSR")
            if [ -z "$VAL_HEX" ]; then
                THERM_VALID[$i]=0
                continue
            fi

            VAL_DEC=$((16#$VAL_HEX))
            THERM_VALID[$i]=1
            THERM_STATUS[$i]=$(( ${THERM_STATUS[$i]:-0} | (VAL_DEC & 1) ))
            DIGITAL_READOUT[$i]=$(( (VAL_DEC >> 16) & 0x7F ))

            (( (VAL_DEC >> 2) & 1 )) && ANY_PROCHOT=1
            (( (VAL_DEC >> 3) & 1 )) && ANY_PROCHOT_LOG=1
            (( VAL_DEC & 1 )) && ANY_THERMAL=1
            (( (VAL_DEC >> 10) & 1 )) && ANY_POWER_LIMIT=1
            (( (VAL_DEC >> 11) & 1 )) && ANY_POWER_LIMIT_LOG=1
            if (( DIGITAL_READOUT[$i] < MIN_DTJ )); then
                MIN_DTJ=${DIGITAL_READOUT[$i]}
            fi

            PERF_HEX=$(rdmsr -p "$i" "$PERF_LIMIT_MSR" 2>/dev/null)
            if [[ -n $PERF_HEX ]]; then
                PERF_DEC=$((16#$PERF_HEX))
                PERF_ACTIVE=$(( PERF_ACTIVE | (PERF_DEC & 0xFFFF) ))
                PERF_LOG=$(( PERF_LOG | ((PERF_DEC >> 16) & 0xFFFF) ))
            fi
        done
    done

    (( PERF_ACTIVE & 1 )) && ANY_PROCHOT=1

    BD_PROCHOT_INFERRED=0
    if (( ANY_PROCHOT && !ANY_THERMAL && MIN_DTJ > TCC_OFFSET )); then
        BD_PROCHOT_INFERRED=1
    fi

    # --- Per-CPU thermal status ---
    printf "%-6s %-8s %-8s %-8s %-30s\n" "CPU" "MHz" "Temp" "dTj" "Status"
    echo "--------------------------------------------------------------"

    for i in $(seq 0 $((NCPU - 1))); do
        if [[ ${THERM_VALID[$i]:-0} -eq 0 ]]; then
            printf "%-6s %-8s %-8s %-8s %-30s\n" "$i" "?" "?" "?" "read error"
            continue
        fi

        MHZ=$(cat "/sys/devices/system/cpu/cpu$i/cpufreq/scaling_cur_freq")
        if [ -n "$MHZ" ]; then
            MHZ=$((MHZ / 1000))
        else
            MHZ=$(awk -v c="$i" '
                $0 ~ "^processor" { cur=$3 }
                $0 ~ "^cpu MHz" && cur==c { print int($4); exit }
            ' /proc/cpuinfo)
        fi

        STATUS="ok"
        if (( THERM_STATUS[$i] )); then
            STATUS="thermal"
        fi

        CORE_TEMP=$(( TJMAX - DIGITAL_READOUT[$i] ))
        printf "%-6s %-8s %-8s %-8s " \
            "$i" "${MHZ:-?}" "${CORE_TEMP} C" "${DIGITAL_READOUT[$i]}"
        if [[ $STATUS == thermal ]]; then
            printf "%s%-30s%s\n" "$RED" "$STATUS" "$RESET"
        else
            printf "%-30s\n" "$STATUS"
        fi
    done

    ACTIVE_REASONS=()
    LOG_REASONS=()
    (( ANY_POWER_LIMIT )) && ACTIVE_REASONS+=("power limit")
    (( PERF_ACTIVE & (1 << 10) )) && ACTIVE_REASONS+=("PL1 power limit")
    (( PERF_ACTIVE & (1 << 11) )) && ACTIVE_REASONS+=("PL2 power limit")
    if (( ANY_PROCHOT )); then
        if (( BD_PROCHOT_INFERRED )); then
            ACTIVE_REASONS+=("BD_PROCHOT (inferred)")
        else
            ACTIVE_REASONS+=("PROCHOT")
        fi
    fi
    (( ANY_THERMAL || (PERF_ACTIVE & (1 << 1)) )) && ACTIVE_REASONS+=("thermal")

    (( ANY_PROCHOT_LOG || (PERF_LOG & 1) )) && LOG_REASONS+=("PROCHOT")
    (( ANY_POWER_LIMIT_LOG )) && LOG_REASONS+=("power limit")
    (( PERF_LOG & (1 << 1) )) && LOG_REASONS+=("thermal")
    (( PERF_LOG & (1 << 10) )) && LOG_REASONS+=("PL1 power limit")
    (( PERF_LOG & (1 << 11) )) && LOG_REASONS+=("PL2 power limit")

    echo
    tput el
    if (( ${#ACTIVE_REASONS[@]} )); then
        printf "Throttle status: %s%s%s\n" \
            "$RED" "$(join_reasons "${ACTIVE_REASONS[@]}")" "$RESET"
    else
        echo "Throttle status: none"
    fi

    tput el
    if (( ${#LOG_REASONS[@]} )); then
        printf "Throttle log: %s\n" "$(join_reasons "${LOG_REASONS[@]}")"
    else
        echo "Throttle log: none"
    fi

    sleep 1
done

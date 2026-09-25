/* cc -std=c11 -Wall -Wextra -Werror tests/timing_test.c -o /tmp/t2audio-timing-test */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "../timing.h"

static u64 bits(double value)
{
    u64 result;
    memcpy(&result, &value, sizeof(result));
    return result;
}

int main(void)
{
    u64 frames;
    const u64 ring = 16640;

    assert(t2audio_decode_sample_time(bits(0), &frames) && frames == 0);
    assert(t2audio_decode_sample_time(bits(16640), &frames) && frames == ring);
    assert(t2audio_decode_sample_time(bits(33280), &frames) && frames == 2 * ring);
    assert(t2audio_decode_sample_time(bits(9007199254740992.0), &frames) &&
            frames == (1ULL << 53));
    assert(!t2audio_decode_sample_time(bits(-1), &frames));
    assert(!t2audio_decode_sample_time(bits(0.5), &frames));
    assert(!t2audio_decode_sample_time(bits(16640.5), &frames));
    assert(!t2audio_decode_sample_time(0x7ff0000000000000ULL, &frames));
    assert(!t2audio_decode_sample_time(0x7ff8000000000000ULL, &frames));
    assert(!t2audio_decode_sample_time(bits(18014398509481984.0), &frames));

    /* Startup latency cannot wrap backwards to the end of the ring. */
    assert(t2audio_position_frames(0, 10, 40, 0) == 0);
    assert(t2audio_position_frames(0, 50, 40, 0) == 10);
    /* Non-zero sample anchors, missed updates and wrap preserve absolute phase. */
    assert(t2audio_position_frames(4096, 48, 0, 4000) == 4144);
    assert(t2audio_position_frames(3 * ring, 48, 0, ring) == 3 * ring + 48);
    assert(t2audio_position_frames(ring, 48, 0, ring - 1) % ring == 48);
    /* A delayed anchor must not manufacture a forward ring wrap. */
    assert(t2audio_position_frames(ring, 0, 0, ring + 100) == ring + 100);
    assert(t2audio_position_frames(ring, 101, 0, ring + 100) == ring + 101);
    /* A late callback cannot replace the running clock with a stale anchor. */
    assert(t2audio_reanchor_frames(2 * ring, ring, ring + 480) ==
            2 * ring + 480);
    /* A sampleTime that is genuinely ahead remains authoritative. */
    assert(t2audio_reanchor_frames(3 * ring, ring, ring) == 3 * ring);
    puts("t2audio timing tests passed");
    return 0;
}

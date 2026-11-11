/* cc -std=c11 -Wall -Wextra -Werror tests/snd-apple-t2-timing_test.c -o timing-test */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "../sound/pci/apple-t2/timing.h"

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

    /* Capture latency cannot wrap backwards to the end of the ring. */
    assert(t2audio_position_frames(0, 40) == 0);
    assert(t2audio_position_frames(50, 40) == 10);
    /* Absolute sampleTime keeps its ring phase across wraps. */
    assert(t2audio_position_frames(3 * ring + 48, 0) % ring == 48);
    puts("t2audio timing tests passed");
    return 0;
}

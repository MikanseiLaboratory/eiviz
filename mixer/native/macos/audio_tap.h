#ifndef EIVIZ_AUDIO_TAP_H
#define EIVIZ_AUDIO_TAP_H

#include <stddef.h>
#include <stdint.h>

typedef struct EivizAudioTap EivizAudioTap;
typedef void (*EivizAudioTapCallback)(
    void *ctx,
    const float *interleaved,
    uint32_t frames,
    uint32_t channels,
    double sample_rate);

// mode: 1 = output-device loopback, 2 = application audio.
// process_exe / process_aumid choose the application in mode 2.
EivizAudioTap *eiviz_audio_tap_start(
    uint32_t mode,
    const char *device_id,
    const char *process_exe,
    const char *process_aumid,
    EivizAudioTapCallback callback,
    void *ctx,
    char *err,
    size_t err_cap);

void eiviz_audio_tap_stop(EivizAudioTap *tap);

// JSON array of {name, exe, aumid} for running applications, excluding eiviz.
int32_t eiviz_audio_process_list(uint8_t *out, size_t cap);

#endif

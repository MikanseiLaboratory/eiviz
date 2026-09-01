#ifndef EIVIZ_AV_PUMP_H
#define EIVIZ_AV_PUMP_H

#include <stdint.h>

enum {
    EIVIZ_AV_KIND_VIDEO = 1,
    EIVIZ_AV_KIND_AUDIO = 2,
};

typedef struct EivizAvSample {
    int32_t kind;
    int32_t width;
    int32_t height;
    int32_t stride;
    int32_t sample_rate;
    int32_t channels;
    int32_t frames;
    int64_t pts_hns;
    const uint8_t *data;
    uint32_t bytes;
} EivizAvSample;

typedef struct EivizAvPump EivizAvPump;

#ifdef __cplusplus
extern "C" {
#endif

typedef struct EivizAvCaptureInfo {
    char id[512];
    char name[256];
} EivizAvCaptureInfo;

typedef struct EivizAvCaptureMode {
    uint32_t width;
    uint32_t height;
    uint32_t fps_num;
    uint32_t fps_den;
    uint32_t format;
} EivizAvCaptureMode;

EivizAvPump *eiviz_av_open_file(const char *path, int64_t start_hns);
EivizAvPump *eiviz_av_open_capture(const char *device_id, uint32_t width, uint32_t height, uint32_t fps_num, uint32_t fps_den);
int eiviz_av_enum_captures(EivizAvCaptureInfo *out, uint32_t cap);
int eiviz_av_enum_capture_modes(const char *device_id, EivizAvCaptureMode *out, uint32_t cap);
void eiviz_av_close(EivizAvPump *pump);
int64_t eiviz_av_duration_hns(const EivizAvPump *pump);
int eiviz_av_next(EivizAvPump *pump, EivizAvSample *out);

#ifdef __cplusplus
}
#endif

#endif

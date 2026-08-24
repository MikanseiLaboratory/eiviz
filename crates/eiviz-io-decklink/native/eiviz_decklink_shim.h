#ifndef EIVIZ_DECKLINK_SHIM_H
#define EIVIZ_DECKLINK_SHIM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define EIVIZ_DECKLINK_ABI_VERSION 1u
#define EIVIZ_DECKLINK_DEVICE_CAPTURE 1u
#define EIVIZ_DECKLINK_DEVICE_PLAYBACK 2u
#define EIVIZ_DECKLINK_FRAME_NO_INPUT 1u

typedef struct eiviz_decklink_capture eiviz_decklink_capture;
typedef struct eiviz_decklink_playback eiviz_decklink_playback;

typedef struct eiviz_decklink_device {
    const char* persistent_id;
    const char* display_name;
    uint32_t capabilities;
} eiviz_decklink_device;

typedef struct eiviz_decklink_video_frame {
    const uint8_t* data;
    size_t data_len;
    uint32_t width;
    uint32_t height;
    uint32_t row_bytes;
    uint32_t flags;
    int64_t stream_time;
    int64_t duration;
    int64_t time_scale;
} eiviz_decklink_video_frame;

typedef struct eiviz_decklink_audio_packet {
    const int16_t* samples;
    size_t sample_count;
    uint32_t frame_count;
    uint32_t channels;
    uint32_t sample_rate;
    int64_t packet_time;
    int64_t time_scale;
} eiviz_decklink_audio_packet;

typedef struct eiviz_decklink_playback_diagnostics {
    uint64_t scheduled_video;
    uint64_t completed_video;
    uint64_t late_video;
    uint64_t dropped_video;
    uint64_t flushed_video;
    uint32_t buffered_video;
    uint32_t buffered_audio_frames;
    int32_t reference_locked;
} eiviz_decklink_playback_diagnostics;

typedef void (*eiviz_decklink_device_callback)(
    void* context,
    const eiviz_decklink_device* device);
typedef void (*eiviz_decklink_video_callback)(
    void* context,
    const eiviz_decklink_video_frame* frame);
typedef void (*eiviz_decklink_audio_callback)(
    void* context,
    const eiviz_decklink_audio_packet* packet);

uint32_t eiviz_decklink_abi_version(void);

int32_t eiviz_decklink_enumerate(
    eiviz_decklink_device_callback callback,
    void* context,
    char* error,
    size_t error_capacity);

int32_t eiviz_decklink_capture_open(
    const char* persistent_id,
    uint32_t audio_channels,
    eiviz_decklink_video_callback video_callback,
    eiviz_decklink_audio_callback audio_callback,
    void* context,
    eiviz_decklink_capture** capture,
    char* error,
    size_t error_capacity);

void eiviz_decklink_capture_close(eiviz_decklink_capture* capture);

int32_t eiviz_decklink_playback_open(
    const char* persistent_id,
    uint32_t audio_channels,
    eiviz_decklink_playback** playback,
    char* error,
    size_t error_capacity);

int32_t eiviz_decklink_playback_schedule_video(
    eiviz_decklink_playback* playback,
    const uint8_t* bgra,
    size_t data_len,
    uint32_t row_bytes,
    int64_t display_time,
    int64_t duration,
    int64_t time_scale,
    char* error,
    size_t error_capacity);

int32_t eiviz_decklink_playback_schedule_audio(
    eiviz_decklink_playback* playback,
    const int16_t* interleaved_samples,
    uint32_t frame_count,
    int64_t stream_time,
    int64_t time_scale,
    char* error,
    size_t error_capacity);

int32_t eiviz_decklink_playback_start(
    eiviz_decklink_playback* playback,
    int64_t start_time,
    int64_t time_scale,
    char* error,
    size_t error_capacity);

int32_t eiviz_decklink_playback_get_diagnostics(
    eiviz_decklink_playback* playback,
    eiviz_decklink_playback_diagnostics* diagnostics,
    char* error,
    size_t error_capacity);

void eiviz_decklink_playback_close(eiviz_decklink_playback* playback);

#ifdef __cplusplus
}
#endif

#endif

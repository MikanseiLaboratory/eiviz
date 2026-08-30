#ifndef EIVIZ_MIXER_H
#define EIVIZ_MIXER_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define EIVIZ_OK 0

#define EIVIZ_SRC_COLOR 1ull
#define EIVIZ_SRC_BARS 2ull
#define EIVIZ_SRC_BLACK 3ull
#define EIVIZ_SRC_BLUE 4ull

#define EIVIZ_OUTPUT_PROGRAM 0u
#define EIVIZ_OUTPUT_PREVIEW 1u
#define EIVIZ_OUTPUT_MULTIVIEW 2u

#define EIVIZ_SCENE_BASE 0x00010000ull
#define EIVIZ_MULTIVIEW_BASE 0x00020000ull
#define EIVIZ_LABEL_BASE 0x00030000ull
#define EIVIZ_AUDIO_BUS_PEAK_BASE 0x00040000ull
#define EIVIZ_MU_SOURCE_FLAG 0x8000000000000000ull
#define EIVIZ_MU_BUS_PREVIEW 0x1000000000000000ull
#define EIVIZ_MU_ID_MASK 0x0FFFFFFFFFFFFFFFull

#define EIVIZ_NATIVE_WIN32_HWND 1u
#define EIVIZ_NATIVE_APPKIT_NSVIEW 2u

#define EIVIZ_TRANSITION_CUT 0u
#define EIVIZ_TRANSITION_FADE 1u
#define EIVIZ_TRANSITION_DIP 2u

#define EIVIZ_FMT_UYVY 0u
#define EIVIZ_FMT_BGRA 1u
#define EIVIZ_FMT_RGBA 3u

#define EIVIZ_OUT_OMT 0u
#define EIVIZ_OUT_NDI 1u
#define EIVIZ_OUT_DECKLINK 2u

#define EIVIZ_SRC_KIND_SCENE 0u
#define EIVIZ_SRC_KIND_MU_PREVIEW 1u
#define EIVIZ_SRC_KIND_MU_PROGRAM 2u
#define EIVIZ_SRC_KIND_MU_MULTIVIEW 3u
#define EIVIZ_SRC_KIND_INPUT 4u

#define EIVIZ_GEN_SOLID 0u
#define EIVIZ_GEN_BARS 1u

#define EIVIZ_SAVE_ALWAYS_LOW 0u
#define EIVIZ_SAVE_NOT_ON_PROGRAM 1u
#define EIVIZ_SAVE_NOT_ON_PREVIEW_OR_PROGRAM 2u
#define EIVIZ_SAVE_ALWAYS_FULL 3u
#define EIVIZ_SAVE_FLAG_MULTIVIEW 1u

typedef struct EivizRect {
    float x;
    float y;
    float width;
    float height;
} EivizRect;

typedef struct EivizOverlayDesc {
    uint64_t source_id;
    EivizRect rect;
    float opacity;
    int32_t z;
    uint32_t audio_follow;
    uint32_t pad;
} EivizOverlayDesc;

typedef struct EivizUnitState {
    uint64_t program_source;
    uint64_t preview_source;
    float mix;
    uint32_t transition_kind;
    uint32_t overlay_count;
    uint32_t mv_slot_count;
    EivizOverlayDesc overlays[8];
    uint64_t mv_slots[16];
} EivizUnitState;

typedef struct EivizVideoInfo {
    uint32_t playing;
    uint32_t is_file;
    int64_t position_hns;
    int64_t duration_hns;
} EivizVideoInfo;

typedef struct EivizAudioPeak {
    uint64_t source_id;
    float left;
    float right;
} EivizAudioPeak;

typedef struct EivizMixerStats {
    float render_ms;
    float frame_budget_ms;
} EivizMixerStats;

typedef struct EivizSourceUsage {
    uint64_t source_id;
    uint32_t width;
    uint32_t height;
    uint64_t ram_bytes;
    uint64_t vram_bytes;
} EivizSourceUsage;

uint32_t mixer_ping(void);
int32_t mixer_create(uint64_t adapter_luid, uint32_t fps_num, uint32_t fps_den);
void mixer_destroy(void);
int32_t mixer_create_unit(uint64_t unit_id, uint32_t width, uint32_t height);
int32_t mixer_destroy_unit(uint64_t unit_id);
int32_t mixer_unit_configure(uint64_t unit_id, uint32_t width, uint32_t height, uint32_t fps_num, uint32_t fps_den);
int32_t mixer_define_scene(uint64_t scene_id, uint32_t width, uint32_t height, uint32_t count, const EivizOverlayDesc *layers);
int32_t mixer_destroy_scene(uint64_t scene_id);
int32_t mixer_define_generator(uint64_t id, uint32_t kind, float r, float g, float b, float a, uint32_t scroll);
int32_t mixer_unit_attach_native(uint64_t unit_id, uint32_t kind, uint32_t native_kind, intptr_t handle, uint32_t width, uint32_t height);
int32_t mixer_unit_resize_native(uint64_t unit_id, uint32_t kind, uint32_t native_kind, intptr_t handle, uint32_t width, uint32_t height);
int32_t mixer_unit_detach_native(uint64_t unit_id, uint32_t kind, uint32_t native_kind, intptr_t handle);
int32_t mixer_attach_monitor_native(uint64_t monitor_id, uint64_t source_id, uint32_t native_kind, intptr_t handle, uint32_t width, uint32_t height);
int32_t mixer_resize_monitor(uint64_t monitor_id, uint32_t width, uint32_t height);
int32_t mixer_detach_monitor(uint64_t monitor_id);
int32_t mixer_monitor_set_source(uint64_t monitor_id, uint64_t source_id);
int32_t mixer_unit_set_state(uint64_t unit_id, const EivizUnitState *state);
int32_t mixer_unit_get_state(uint64_t unit_id, EivizUnitState *out);
int32_t mixer_unit_cut(uint64_t unit_id, uint32_t swap);
int32_t mixer_unit_auto(uint64_t unit_id, uint32_t duration_ms, uint32_t swap);
int32_t mixer_register_source(uint64_t id, uint32_t width, uint32_t height, uint32_t format);
int32_t mixer_push_frame(uint64_t id, const uint8_t *ptr, uint32_t stride, uint32_t height, int64_t pts);
int32_t mixer_push_audio(uint64_t id, int32_t sample_rate, int32_t channels, uint32_t frames, int64_t pts, const float *planar);
int32_t mixer_load_still(uint64_t id, const char *path);
int32_t mixer_omt_connect(uint64_t id, const char *address, uint32_t use_gpu, uint32_t frame_buffer_frames, uint32_t quality);
int32_t mixer_ndi_connect(uint64_t id, const char *address, uint32_t frame_buffer_frames, uint32_t low_bandwidth);
int32_t mixer_set_live_save(uint64_t id, uint32_t mode, uint32_t flags);
int32_t mixer_omt_set_quality(uint64_t id, uint32_t quality);
int32_t mixer_omt_start_send(uint64_t unit_id, const char *name);
int32_t mixer_output_add(uint64_t output_id, uint32_t transport, const char *name, uint32_t source_kind, uint64_t source_id, uint64_t unit_id, uint32_t use_gpu);
int32_t mixer_output_remove(uint64_t output_id);
int32_t mixer_omt_discover(uint8_t *out, size_t cap);
int32_t mixer_ndi_discover(uint8_t *out, size_t cap);
int32_t mixer_destroy_source(uint64_t id);
int32_t mixer_flush_audio(uint64_t id);
int32_t mixer_bind_multiview(uint64_t scene_id, uint64_t preview_unit, uint64_t program_unit);
int32_t mixer_copy_follow_audio(float *out, uint32_t cap);
int32_t mixer_copy_audio_peaks(EivizAudioPeak *out, uint32_t cap);
int32_t mixer_copy_source_usage(EivizSourceUsage *out, uint32_t cap);
int32_t mixer_copy_stats(EivizMixerStats *out);
int32_t mixer_set_frame_buffer(uint32_t frames);
int32_t mixer_set_monitor_present_interval(uint64_t monitor_id, uint32_t frames);
int32_t mixer_last_error(uint8_t *out, size_t cap);
int32_t mixer_audio_bus_upsert(uint64_t id, const char *name, uint32_t role, uint32_t device_kind, const char *device_id, int32_t map_left, int32_t map_right, uint32_t exclusive);
int32_t mixer_audio_set_input(uint64_t id, uint32_t bus_mask, float gain, uint32_t mute);
int32_t mixer_audio_set_bus_gain(uint64_t id, float gain, uint32_t mute);
int32_t mixer_audio_set_unit_link(uint64_t unit_id, uint64_t bus_id, uint32_t mode);
int32_t mixer_audio_set_headphone_copy_master(uint32_t enabled);

#ifdef __cplusplus
}
#endif

#endif

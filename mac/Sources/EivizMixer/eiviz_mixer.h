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
#define EIVIZ_INCOMING_PREVIEW 0ull
#define EIVIZ_INCOMING_PROGRAM 0xffffffffffffffffull

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
#define EIVIZ_TRANSITION_WIPE 3u
#define EIVIZ_TRANSITION_SLIDE 4u
#define EIVIZ_TRANSITION_PUSH 5u
#define EIVIZ_TRANSITION_IRIS 6u
#define EIVIZ_TRANSITION_BLINDS 7u
#define EIVIZ_TRANSITION_ZOOM 8u
#define EIVIZ_TRANSITION_ADDITIVE 9u
#define EIVIZ_TRANSITION_CUBE 10u
#define EIVIZ_TRANSITION_CROSS_ZOOM 11u
#define EIVIZ_TRANSITION_FLY_ROTATE 12u
#define EIVIZ_TRANSITION_BARN_DOOR 13u
#define EIVIZ_TRANSITION_CLOCK 14u
#define EIVIZ_TRANSITION_LOREZ 15u
#define EIVIZ_TRANSITION_METAMIX 16u
#define EIVIZ_TRANSITION_TILE 17u
#define EIVIZ_TRANSITION_FLIP 18u
#define EIVIZ_TRANSITION_GLITCH 19u
#define EIVIZ_TRANSITION_SWIRL 20u
#define EIVIZ_TRANSITION_LUMA_MORPH 21u
#define EIVIZ_TRANSITION_PARTS 22u
#define EIVIZ_TRANSITION_STATIC 23u
#define EIVIZ_TRANSITION_SHIFT_RGB 24u
#define EIVIZ_TRANSITION_DISPLACE 25u
#define EIVIZ_TRANSITION_RIPPLE 26u
#define EIVIZ_TRANSITION_GRID_DISSOLVE 27u
#define EIVIZ_TRANSITION_CUBE_ZOOM 28u
#define EIVIZ_TRANSITION_PAGE_CURL 29u
#define EIVIZ_TRANSITION_KALEIDOSCOPE 30u
#define EIVIZ_TRANSITION_POLAR 31u
#define EIVIZ_TRANSITION_FILM_BURN 32u
#define EIVIZ_TRANSITION_ZOOM_BLUR 33u
#define EIVIZ_TRANSITION_MULTITASK 34u
#define EIVIZ_TRANSITION_HEART 35u
#define EIVIZ_TRANSITION_DIAMOND 36u
#define EIVIZ_TRANSITION_STAR 37u
#define EIVIZ_TRANSITION_ROLLER_DOOR 38u
#define EIVIZ_TRANSITION_PIXEL_SORT 39u
#define EIVIZ_TRANSITION_DATAMOSH 40u
#define EIVIZ_TRANSITION_VISUAL_DISSOLVE 41u
#define EIVIZ_TRANSITION_OPTICAL_FLOW 42u
#define EIVIZ_TRANSITION_BLOOM 43u
#define EIVIZ_TRANSITION_CUSTOM 50u
#define EIVIZ_TRANSITION_STINGER 100u

#define EIVIZ_DURATION_FRAMES 0u
#define EIVIZ_DURATION_MS 1u
#define EIVIZ_EASING_LINEAR 0u
#define EIVIZ_EASING_IN 1u
#define EIVIZ_EASING_OUT 2u
#define EIVIZ_EASING_IN_OUT 3u
#define EIVIZ_EASING_SMOOTHSTEP 4u
#define EIVIZ_DIR_LEFT 0u
#define EIVIZ_DIR_RIGHT 1u
#define EIVIZ_DIR_UP 2u
#define EIVIZ_DIR_DOWN 3u

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
    EivizRect crop;
    float opacity;
    int32_t z;
    uint32_t audio_follow;
    uint32_t hidden;
    const char *label;
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
    uint32_t transition_easing;
    uint32_t transition_direction;
    uint32_t keep_preview;
    uint32_t pad;
    float dip_r;
    float dip_g;
    float dip_b;
    float dip_a;
    uint64_t incoming_source;
    float softness;
    float param;
} EivizUnitState;

typedef struct EivizVideoInfo {
    uint32_t playing;
    uint32_t is_file;
    int64_t position_hns;
    int64_t duration_hns;
} EivizVideoInfo;

typedef struct EivizVideoCaptureInfo {
    uint8_t id[512];
    uint8_t name[256];
} EivizVideoCaptureInfo;

typedef struct EivizVideoCaptureMode {
    uint32_t width;
    uint32_t height;
    uint32_t fps_num;
    uint32_t fps_den;
    uint32_t format;
} EivizVideoCaptureMode;

typedef struct EivizAudioPeak {
    uint64_t source_id;
    float left;
    float right;
} EivizAudioPeak;

typedef struct EivizMixerStats {
    float render_ms;
    float frame_budget_ms;
    uint64_t ram_bytes;
    uint64_t vram_bytes;
    uint64_t compose_vram_bytes;
    uint64_t delay_vram_bytes;
    uint64_t surface_lost;
} EivizMixerStats;

typedef struct EivizMixerRebarInfo {
    uint32_t available;
    uint32_t active;
    uint32_t uma;
    uint32_t gpu_upload_heaps;
    uint64_t bar_bytes;
    uint64_t vram_bytes;
    uint8_t adapter[128];
} EivizMixerRebarInfo;

typedef struct EivizSourceUsage {
    uint64_t source_id;
    uint32_t width;
    uint32_t height;
    uint64_t ram_bytes;
    uint64_t vram_bytes;
    float gpu_pct;
} EivizSourceUsage;

typedef struct EivizAudioDeviceInfo {
    uint32_t kind;
    uint32_t channels;
    uint8_t id[256];
    uint8_t name[256];
} EivizAudioDeviceInfo;

typedef struct EivizAudioBusInfo {
    uint64_t id;
    uint32_t role;
    uint32_t device_kind;
    int32_t map_left;
    int32_t map_right;
    uint32_t exclusive;
    uint32_t bit;
    uint8_t name[64];
    uint8_t device_id[256];
} EivizAudioBusInfo;

uint32_t mixer_ping(void);
int32_t mixer_create(uint64_t adapter_luid, uint32_t fps_num, uint32_t fps_den);
void mixer_destroy(void);
int32_t mixer_create_unit(uint64_t unit_id, uint32_t width, uint32_t height);
int32_t mixer_destroy_unit(uint64_t unit_id);
int32_t mixer_unit_configure(uint64_t unit_id, uint32_t width, uint32_t height, uint32_t fps_num, uint32_t fps_den);
int32_t mixer_define_scene(uint64_t scene_id, uint32_t width, uint32_t height, uint32_t count, const EivizOverlayDesc *layers);
int32_t mixer_destroy_scene(uint64_t scene_id);
int32_t mixer_define_generator(uint64_t id, uint32_t kind, float r, float g, float b, float a, uint32_t scroll);
int32_t mixer_define_mix_input(uint64_t id, uint64_t target_id, uint32_t source_kind, uint32_t delay, uint64_t audio_bus_id);
int32_t mixer_generator_set_tone(uint64_t id, float hz, float level_dbfs);
int32_t mixer_unit_attach_native(uint64_t unit_id, uint32_t kind, uint32_t native_kind, intptr_t handle, uint32_t width, uint32_t height);
int32_t mixer_unit_resize_native(uint64_t unit_id, uint32_t kind, uint32_t native_kind, intptr_t handle, uint32_t width, uint32_t height);
int32_t mixer_unit_detach_native(uint64_t unit_id, uint32_t kind, uint32_t native_kind, intptr_t handle);
int32_t mixer_attach_monitor_native(uint64_t monitor_id, uint64_t source_id, uint32_t native_kind, intptr_t handle, uint32_t width, uint32_t height);
int32_t mixer_resize_monitor(uint64_t monitor_id, uint32_t width, uint32_t height);
int32_t mixer_detach_monitor(uint64_t monitor_id);
int32_t mixer_monitor_set_source(uint64_t monitor_id, uint64_t source_id);
int32_t mixer_unit_set_state(uint64_t unit_id, const EivizUnitState *state);
int32_t mixer_unit_get_state(uint64_t unit_id, EivizUnitState *out);
int32_t mixer_unit_cut(uint64_t unit_id, uint32_t swap, uint64_t incoming_source);
int32_t mixer_unit_auto(uint64_t unit_id, uint32_t kind, uint32_t duration_ms, uint32_t swap, uint32_t keep_preview, uint32_t easing, uint32_t direction, float dip_r, float dip_g, float dip_b, float dip_a, uint64_t incoming_source, float softness, float param);
int32_t mixer_unit_overlay_auto(uint64_t unit_id, uint32_t target_enabled, uint32_t duration_ms, const EivizOverlayDesc *desc);
int32_t mixer_unit_set_custom_wgsl(uint64_t unit_id, const char *wgsl);
int32_t mixer_validate_custom_wgsl(const char *wgsl);
int32_t mixer_register_source(uint64_t id, uint32_t width, uint32_t height, uint32_t format);
int32_t mixer_push_frame(uint64_t id, const uint8_t *ptr, uint32_t stride, uint32_t height, int64_t pts);
int32_t mixer_push_audio(uint64_t id, int32_t sample_rate, int32_t channels, uint32_t frames, int64_t pts, const float *planar);
int32_t mixer_load_still(uint64_t id, const char *path);
int32_t mixer_video_start(uint64_t id, const char *path, uint32_t capture, uint32_t format, uint32_t width, uint32_t height, uint32_t fps_num, uint32_t fps_den, uint32_t frame_buffer_frames);
int32_t mixer_video_enum_captures(EivizVideoCaptureInfo *out, uint32_t cap);
int32_t mixer_video_enum_capture_modes(const char *device_id, EivizVideoCaptureMode *out, uint32_t cap);
int32_t mixer_video_set_playing(uint64_t id, uint32_t playing);
int32_t mixer_video_set_loop(uint64_t id, uint32_t looping);
int32_t mixer_video_seek(uint64_t id, int64_t hns);
int32_t mixer_video_copy_info(uint64_t id, EivizVideoInfo *out);
int32_t mixer_omt_connect(uint64_t id, const char *address, uint32_t use_gpu, uint32_t frame_buffer_frames, uint32_t quality);
int32_t mixer_ndi_connect(uint64_t id, const char *address, uint32_t frame_buffer_frames, uint32_t low_bandwidth);
int32_t mixer_set_live_save(uint64_t id, uint32_t mode, uint32_t flags);
int32_t mixer_omt_set_quality(uint64_t id, uint32_t quality);
int32_t mixer_omt_start_send(uint64_t unit_id, const char *name);
int32_t mixer_output_add(uint64_t output_id, uint32_t transport, const char *name, uint32_t source_kind, uint64_t source_id, uint64_t unit_id, uint32_t use_gpu, uint64_t audio_bus_id);
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
int32_t mixer_copy_rebar_info(EivizMixerRebarInfo *out);
int32_t mixer_set_rebar_optimization(uint32_t enabled);
int32_t mixer_set_ndi_gpu_upload(uint32_t enabled);
int32_t mixer_set_bus_colors(uint8_t prv_r, uint8_t prv_g, uint8_t prv_b, uint8_t pgm_r, uint8_t pgm_g, uint8_t pgm_b, uint8_t in_r, uint8_t in_g, uint8_t in_b);
int32_t mixer_set_mv_label(uint64_t scene_id, float size, uint32_t percent, uint32_t top);
int32_t mixer_set_frame_buffer(uint32_t frames);
int32_t mixer_set_monitor_present_interval(uint64_t monitor_id, uint32_t frames);
int32_t mixer_thumb_set(uint64_t source_id, uint32_t width, uint32_t height, uint32_t interval);
int32_t mixer_thumb_read(uint64_t source_id, uint8_t *buf, size_t cap, uint32_t *out_w, uint32_t *out_h, uint32_t *out_stride);
int32_t mixer_last_error(uint8_t *out, size_t cap);
int32_t mixer_take_fatal(uint8_t *out, size_t cap);
int32_t mixer_session_load(const char *path, uint8_t *out, size_t cap);
int32_t mixer_session_save(const char *path, const uint8_t *json, size_t len);
int32_t mixer_session_canonicalize(const uint8_t *json, size_t len, uint8_t *out, size_t cap);
int32_t mixer_audio_bus_upsert(uint64_t id, const char *name, uint32_t role, uint32_t device_kind, const char *device_id, int32_t map_left, int32_t map_right, uint32_t exclusive);
int32_t mixer_audio_bus_remove(uint64_t id);
int32_t mixer_audio_bus_count(void);
int32_t mixer_audio_bus_get(uint32_t index, EivizAudioBusInfo *out);
int32_t mixer_audio_set_input(uint64_t id, uint32_t bus_mask, float gain, uint32_t mute);
int32_t mixer_audio_set_bus_gain(uint64_t id, float gain, uint32_t mute);
int32_t mixer_audio_set_unit_link(uint64_t unit_id, uint64_t bus_id, uint32_t mode);
int32_t mixer_audio_set_headphone_cue(uint64_t unit_id);
int32_t mixer_audio_set_headphone_copy_master(uint32_t enabled);
int32_t mixer_audio_enum_devices(uint32_t kind, EivizAudioDeviceInfo *out, uint32_t cap);
int32_t mixer_audio_device_channels(uint32_t kind, const char *device_id);

#ifdef __cplusplus
}
#endif

#endif

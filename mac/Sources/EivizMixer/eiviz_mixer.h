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

#define EIVIZ_SCENE_BASE 0x00010000ull

#define EIVIZ_NATIVE_APPKIT_NSVIEW 2u

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

uint32_t mixer_ping(void);
int32_t mixer_create(uint64_t adapter_luid, uint32_t fps_num, uint32_t fps_den);
void mixer_destroy(void);
int32_t mixer_create_unit(uint64_t unit_id, uint32_t width, uint32_t height);
int32_t mixer_define_scene(
    uint64_t scene_id,
    uint32_t width,
    uint32_t height,
    uint32_t count,
    const EivizOverlayDesc *layers);
int32_t mixer_unit_attach_native(
    uint64_t unit_id,
    uint32_t kind,
    uint32_t native_kind,
    intptr_t handle,
    uint32_t width,
    uint32_t height);
int32_t mixer_unit_resize_native(
    uint64_t unit_id,
    uint32_t kind,
    uint32_t native_kind,
    intptr_t handle,
    uint32_t width,
    uint32_t height);
int32_t mixer_unit_detach_native(
    uint64_t unit_id,
    uint32_t kind,
    uint32_t native_kind,
    intptr_t handle);
int32_t mixer_unit_set_state(uint64_t unit_id, const EivizUnitState *state);
int32_t mixer_unit_get_state(uint64_t unit_id, EivizUnitState *out);
int32_t mixer_unit_cut(uint64_t unit_id, uint32_t swap);
int32_t mixer_unit_auto(uint64_t unit_id, uint32_t duration_ms, uint32_t swap);
int32_t mixer_load_still(uint64_t id, const char *path);
int32_t mixer_omt_start_send(uint64_t unit_id, const char *name);
int32_t mixer_last_error(uint8_t *out, size_t cap);

#ifdef __cplusplus
}
#endif

#endif

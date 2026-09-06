#ifndef EIVIZ_REMOTE_H
#define EIVIZ_REMOTE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

int32_t mixer_remote_open(const char *url, const char *token);
int32_t mixer_remote_close(int32_t handle);
int32_t mixer_remote_copy_snapshot(int32_t handle, uint8_t *out, size_t cap);
int32_t mixer_remote_copy_live(int32_t handle, uint8_t *out, size_t cap);
int32_t mixer_remote_copy_status(int32_t handle, uint8_t *out, size_t cap);
int32_t mixer_remote_cut(int32_t handle, uint64_t unit_id, uint32_t swap);
int32_t mixer_remote_preview(int32_t handle, uint64_t unit_id, uint64_t scene_id);
int32_t mixer_remote_auto(int32_t handle, uint64_t unit_id, uint32_t kind, uint32_t duration_ms, uint32_t swap, uint32_t keep_preview, uint32_t easing, uint32_t direction, float dip_r, float dip_g, float dip_b, float dip_a, float softness, float param);
int32_t mixer_remote_set_mix(int32_t handle, uint64_t unit_id, float value);
int32_t mixer_remote_overlay_auto(int32_t handle, uint64_t unit_id, uint32_t index, uint32_t duration_ms, uint32_t to_on);
int32_t mixer_remote_mutate(int32_t handle, const uint8_t *json, size_t len, uint64_t expected_revision);
int32_t mixer_remote_replace(int32_t handle, const uint8_t *json, size_t len, uint64_t expected_revision);
int32_t mixer_remote_video_play(int32_t handle, uint64_t input_id, uint32_t playing);
int32_t mixer_remote_video_loop(int32_t handle, uint64_t input_id, uint32_t looping);
int32_t mixer_remote_video_seek(int32_t handle, uint64_t input_id, int64_t position_hns);
int32_t mixer_remote_upload(int32_t handle, const char *path, const char *kind, const char *name, uint32_t video_loop, uint64_t expected_revision);

#ifdef __cplusplus
}
#endif

#endif

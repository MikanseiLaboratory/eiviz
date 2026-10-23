---
title: Instant Replay
description: Record every camera, mark the moments that matter, and play them back on air
---

Instant Replay records a set of scenes continuously so the operator can cut a clip out of the last few minutes and play it back while the show continues. It needs a Professional license and the Pro module.

## Adding Replay to a project

Replay exists only while the project has a Replay input.

1. **Add Input** and pick the **Replay** category. The category appears only when the plan and the module allow Replay.
2. Each Replay input is one replay channel, named `Replay A`, `Replay B`, and so on, up to 26.

Until a Replay input exists, the Replay window and Replay Setup are not available, and nothing about Replay is written to the session. Removing the last Replay input asks for confirmation and removes its settings, events, and playlists with it. A scene animation that cues a removed channel loses that cue rather than breaking the session.

## Replay Setup

Replay Setup is a dedicated window, opened from the main window. It holds everything the recording needs:

- **Storage**: one or more drives. Every angle is assigned to exactly one drive, and one drive holding every angle is the usual setup. Remaining time is free space minus the reserve, summed over the drives in the set.
- **Profile**: HQ, SQ, or LQ. Quality is never reduced automatically.
- **Segment frames**: how many frames one segment file holds (default 120, range 30–1800).
- **Encode queue**: unencoded frames kept per angle (default 6, range 2–60).
- **Write buffer**: bytes held before a segment is written (default 512 MiB, range 64–8192).
- **Reserve**: space left untouched on every drive (MiB).
- **Overflow**: drop frames, or stop recording, when encoding falls behind.
- **When the disk is full**: stop recording and report, overwrite the oldest segments, or overwrite everything except the clips and protected events still in use.
- **New clips**: the transition clips get when they are added later.
- **Angles**: each angle records either a scene's composited output or an input's own picture, and can take its audio from a mixing unit's program mix or record silence. An input angle needs no scene, which is what makes recording one camera cheap. An **OMT input** angle can also use **passthrough**: the frames the sender already encoded are stored as they arrive, with no GPU readback and no second encoder, so the recording costs neither picture quality nor CPU. The recording format (resolution and frame rate) comes from the first mixing unit, and every angle shares it.
- **Encoder**: per angle, either **CPU** (the default) or **GPU**. CPU encodes the frame after reading it back; GPU encodes straight from the scene texture and never copies the frame back. At 1080p the CPU encoder is faster per frame, and the GPU one pays off when the CPU has to be freed for many angles. **Measure this PC** says which one is faster here.
- **Add every new event to the playlist automatically**: uses the angle selected in the Replay window. Clips are never filed against a guessed camera.
- **Decode ahead**: on by default. A frame or two is decoded ahead of the picture, so a decode that runs long cannot make the picture late. It costs about 8 MB of video memory per frame, and 4K keeps one frame rather than two. Turn it off only when a playback problem is being narrowed down.
- **Performance log**: off by default. While on, playout stage timings (decode, publish, readback, frames taken from the queue, late frames) are written to the mixer log every two seconds.

The angle set is fixed while recording: stop the recording before adding or removing angles.

## How many angles this PC can record

**Measure this PC** runs the GPU encoder, the CPU encoder, and the targets' read and write speed, then reports how many angles hold 60 fps (about ten seconds; recording keeps running throughout). Recording more angles than it reports means frames are dropped, or the recording stops, following the overflow setting. If the CPU encoder turns out to be the faster one, the result says so, and you can switch encoders.

## The Replay window

The Replay window is the operator's surface.

- **Angles**: every angle is shown live. Click one to make it the angle the mark and the playlist use. **Multiview** gathers every angle into one compact list, which is what a machine with many cameras needs.
- **Record / Stop**: starts and stops the recording. The state line shows the remaining time, or that the storage is full, and the health line shows written and read-back frames, frames lost at the readback, encoder and writer losses, segments, queue depth, and write latency. Problems are also spelled out in words, for example "storage low" or "encoder behind".
- **Mark In (I)** and **Mark Out (O)**: mark the clip. The in point, out point, and duration are shown as timecode, and the marks and the recording position are drawn on the angle's waveform. Cancel mark clears the range.
- **Event list**: every marked event with its range and duration. Events can be protected from retention, renamed, deleted, and added to the playlist. Deleting an event keeps the clips that came from it.
- **Playlist**: the clips a channel plays, in order. A clip can be cued, removed, tagged, and edited: transition (Cut or Dissolve), transition timing (Overwrap or Additive), transition length, gain (−60 to +12 dB), keep pitch, playback speed, and where its audio comes from (its own angle, another angle, or silence).
- While the channel plays, the clip on air is marked ▶. With playlist looping on, playback returns to the first clip after the last one; with it off the playlist ends and the transport reads stopped.
- **Play all** sends the clips of the selected playlist to the channel in order, each with its own transition. Playback goes to the channel and does not depend on the channel being on air (to show it on the program, add the Replay input to a scene layer and take that scene). The deck says so when the channel is in no scene at all.
- **Transport**: play, pause, step one frame, jog, 25 %, 50 %, 100 %, and Live to jump back to the recording edge. Stepping and jogging stay inside the mark.
- **Keyboard**: `I` and `O` mark, `Space` plays or pauses, `←` and `→` step one frame, and `L` returns to the recording edge.
- **Export**: write the selected clip, the whole playlist, one angle, or every angle to an MP4. Export runs on the machine that owns the recording.

## Transitions

A clip's transition comes from the playlist's **New clips** setting unless the clip sets its own.

- **Overwrap** shares the transition duration between the two clips: the outgoing clip's tail overlaps the incoming clip's head.
- **Additive** inserts the duration between the clips, so the playlist becomes `A + duration + B` long. The outgoing clip's last frame and the incoming clip's first frame are held for the insertion and dissolve across it.

A Dissolve with a duration of zero, and a Cut, occupy no time at all.

Decoding and those transitions are both done on the GPU during playback. Switching between two clips never sends a frame back to the CPU, so playing several channels at once hardly adds CPU load.

## Driving Replay from an automation panel

The vMix-compatible API can record, mark, cue, and play Replay channels, so a Stream Deck or Companion can run a replay. See the [function reference](/en/developers/function-reference/) for the Replay functions.

A scene's animation can also cue Replay: a sequence step can name a channel and a clip (or the newest clip on the channel) and start it, so a running order plays clips by its own timecode.

The native API (`eivizctl`) runs the same operations. `input add --kind Replay` creates the channel together with its Replay settings, `replay configure --from-json` writes the drives and angles, `replay angle --id N` chooses the angle the mark uses, and `replay record`, `mark-in`, `mark-out`, `cue`, and `play` drive the deck. `replay state` answers with the session's settings and the mixer's measurements together (recording state, each channel's position and speed, drop counters), so monitoring and automation need one call.

## Remote

A Remote can drive Replay: the recording, the marks, the cue, and the transport go to the machine that owns the recording, and the session document comes back with the events and clips they created.

The recording itself stays there. Recording state, health, the waveform, the angle previews, and MP4 export only exist on the machine that owns the recording, so the Replay window says so instead of showing figures from a machine that is not recording. Take the marks on the recording machine when the operator needs to see the frames.

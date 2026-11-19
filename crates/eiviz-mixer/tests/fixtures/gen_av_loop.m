// Regenerates av_loop.mov, the loop-seam A/V drift fixture for
// crates/eiviz-mixer/src/media_macos.rs::loop_restart_re_anchors_audio_to_the_picture_seam.
//
// The file is 1.0 s of 30 fps all-intra H.264 (64x48 moving bars) plus a
// 0.17 s stereo 48 kHz PCM tone, so the audio track ends ~0.83 s before the
// video track. A pump that restarts audio on its own EOF instead of the
// picture's loop seam drifts one duration gap per pass; the test asserts
// the seam signal re-anchors it.
//
// Build and run (the output path argument defaults to this file's directory):
//   clang -x objective-c -w \
//     -framework AVFoundation -framework CoreMedia -framework CoreVideo \
//     -framework CoreAudio -framework Foundation \
//     crates/eiviz-mixer/tests/fixtures/gen_av_loop.m -o /tmp/gen_av_loop
//   /tmp/gen_av_loop [crates/eiviz-mixer/tests/fixtures/av_loop.mov]

#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <CoreAudio/CoreAudioTypes.h>
#import <math.h>

static BOOL appendPCM(AVAssetWriterInput *ain, int frames, double freq) {
    AudioStreamBasicDescription asbd = {0};
    asbd.mSampleRate = 48000.0;
    asbd.mFormatID = kAudioFormatLinearPCM;
    asbd.mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked;
    asbd.mFramesPerPacket = 1;
    asbd.mChannelsPerFrame = 2;
    asbd.mBitsPerChannel = 32;
    asbd.mBytesPerFrame = 8;
    asbd.mBytesPerPacket = 8;
    CMFormatDescriptionRef fmt = NULL;
    if (CMAudioFormatDescriptionCreate(NULL, &asbd, 0, NULL, 0, NULL, NULL, &fmt) != noErr) {
        return NO;
    }
    size_t bytes = (size_t)frames * 8;
    float *pcm = malloc(bytes);
    if (!pcm) {
        CFRelease(fmt);
        return NO;
    }
    for (int i = 0; i < frames; i++) {
        double v = sin(2 * M_PI * freq * (double)i / 48000.0) * 0.5;
        pcm[i * 2 + 0] = (float)v;
        pcm[i * 2 + 1] = (float)v;
    }
    CMBlockBufferRef block = NULL;
    // The block allocator frees `pcm` when the CMBlockBuffer is released.
    if (CMBlockBufferCreateWithMemoryBlock(NULL, pcm, bytes, kCFAllocatorDefault, NULL, 0, bytes, 0, &block) != kCMBlockBufferNoErr) {
        free(pcm);
        CFRelease(fmt);
        return NO;
    }
    CMSampleTimingInfo timing = {
        .duration = CMTimeMake(1, 48000),
        .presentationTimeStamp = CMTimeMake(0, 48000),
        .decodeTimeStamp = kCMTimeInvalid,
    };
    CMSampleBufferRef sample = NULL;
    size_t sample_size = 8;
    if (CMSampleBufferCreate(NULL, block, true, NULL, NULL, fmt, frames, 1, &timing, 1, &sample_size, &sample) != noErr) {
        CFRelease(block);
        CFRelease(fmt);
        return NO;
    }
    CFRelease(block);
    while (!ain.isReadyForMoreMediaData) {
        usleep(1000);
    }
    BOOL ok = [ain appendSampleBuffer:sample];
    CFRelease(sample);
    CFRelease(fmt);
    return ok;
}

int main(int argc, char *argv[]) {
    @autoreleasepool {
        NSString *dst_path = argc > 1
            ? [NSString stringWithUTF8String:argv[1]]
            : @"crates/eiviz-mixer/tests/fixtures/av_loop.mov";
        NSURL *dst = [NSURL fileURLWithPath:dst_path];
        if ([[NSFileManager defaultManager] fileExistsAtPath:dst_path]) {
            [[NSFileManager defaultManager] removeItemAtURL:dst error:nil];
        }
        NSError *err = nil;
        // All-intra and no frame reordering keep the presentation timestamps
        // monotonic, which the seam test samples.
        NSDictionary *vset = @{
            AVVideoCodecKey: AVVideoCodecH264,
            AVVideoWidthKey: @64,
            AVVideoHeightKey: @48,
            AVVideoCompressionPropertiesKey: @{
                AVVideoMaxKeyFrameIntervalKey: @1,
                AVVideoAllowFrameReorderingKey: @NO,
            },
        };
        AVAssetWriter *writer = [[AVAssetWriter alloc] initWithURL:dst fileType:AVFileTypeQuickTimeMovie error:&err];
        if (!writer) {
            printf("writer: %s\n", err.localizedDescription.UTF8String);
            return 1;
        }
        AVAssetWriterInput *vin = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:vset];
        vin.expectsMediaDataInRealTime = NO;
        AVAssetWriterInputPixelBufferAdaptor *vad = [AVAssetWriterInputPixelBufferAdaptor assetWriterInputPixelBufferAdaptorWithAssetWriterInput:vin sourcePixelBufferAttributes:nil];
        if (![writer canAddInput:vin]) {
            printf("cannot add video input\n");
            return 1;
        }
        [writer addInput:vin];
        AVAssetWriterInput *ain = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeAudio outputSettings:nil];
        ain.expectsMediaDataInRealTime = NO;
        if (![writer canAddInput:ain]) {
            printf("cannot add audio input\n");
            return 1;
        }
        [writer addInput:ain];
        if (![writer startWriting]) {
            printf("startWriting failed\n");
            return 1;
        }
        [writer startSessionAtSourceTime:kCMTimeZero];
        for (int32_t f = 0; f < 30; f++) {
            while (!vin.isReadyForMoreMediaData) {
                usleep(1000);
            }
            CVPixelBufferRef pb = NULL;
            if (CVPixelBufferPoolCreatePixelBuffer(NULL, vad.pixelBufferPool, &pb) != kCVReturnSuccess || !pb) {
                printf("pixel buffer failed\n");
                return 1;
            }
            CVPixelBufferLockBaseAddress(pb, 0);
            uint8_t *base = CVPixelBufferGetBaseAddress(pb);
            size_t stride = CVPixelBufferGetBytesPerRow(pb);
            for (int y = 0; y < 48; y++) {
                uint32_t *row = (uint32_t *)(base + y * stride);
                for (int x = 0; x < 64; x++) {
                    row[x] = ((x + f * 2) / 8 % 2) ? 0xFF00FF00u : 0xFF0000FFu;
                }
            }
            CVPixelBufferUnlockBaseAddress(pb, 0);
            if (![vad appendPixelBuffer:pb withPresentationTime:CMTimeMake(f * 20, 600)]) {
                printf("append frame %d failed\n", f);
                return 1;
            }
            CVPixelBufferRelease(pb);
        }
        [vin markAsFinished];
        // 8160 frames = 0.17 s of audio under 1.0 s of picture.
        if (!appendPCM(ain, 8160, 440.0)) {
            printf("append audio failed\n");
            return 1;
        }
        [ain markAsFinished];
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        [writer finishWritingWithCompletionHandler:^{
            dispatch_semaphore_signal(done);
        }];
        if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 30 * NSEC_PER_SEC)) != 0) {
            printf("finishWriting timeout, status %ld\n", (long)writer.status);
            return 1;
        }
        if (writer.status != AVAssetWriterStatusCompleted) {
            printf("writer failed: %s\n", writer.error.localizedDescription.UTF8String);
            return 1;
        }
        printf("ok %s\n", dst_path.UTF8String);
        return 0;
    }
}

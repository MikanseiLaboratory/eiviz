#import "av_pump.h"

#import <stdlib.h>
#import <string.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>

enum {
    EIVIZ_AV_RETRY = 0,
    EIVIZ_AV_OK = 1,
    EIVIZ_AV_EOF = -1,
    EIVIZ_AV_ERR = -2,
};

static int64_t cm_to_hns(CMTime time) {
    if (!CMTIME_IS_NUMERIC(time) || time.timescale == 0) {
        return 0;
    }
    return (int64_t)((double)time.value * 10000000.0 / (double)time.timescale);
}

static void release_sample(CMSampleBufferRef *slot) {
    if (slot && *slot) {
        CFRelease(*slot);
        *slot = NULL;
    }
}

@interface EivizAvPumpObj : NSObject <AVCaptureVideoDataOutputSampleBufferDelegate, AVCaptureAudioDataOutputSampleBufferDelegate>
@property (nonatomic) BOOL capture;
@property (nonatomic) int64_t durationHns;
@property (nonatomic) BOOL stopped;
@property (nonatomic, strong) AVAssetReader *reader;
@property (nonatomic, strong) AVAssetReaderTrackOutput *videoOut;
@property (nonatomic, strong) AVAssetReaderTrackOutput *audioOut;
@property (nonatomic) CMSampleBufferRef pendingVideo;
@property (nonatomic) CMSampleBufferRef pendingAudio;
@property (nonatomic, strong) NSMutableData *scratch;
@property (nonatomic, strong) AVCaptureSession *session;
@property (nonatomic, strong) AVCaptureVideoDataOutput *capVideo;
@property (nonatomic, strong) AVCaptureAudioDataOutput *capAudio;
@property (nonatomic, strong) dispatch_queue_t capQueue;
@property (nonatomic, strong) NSCondition *cond;
@property (nonatomic, strong) NSMutableArray<NSValue *> *inbox;
@property (nonatomic, strong) NSMutableArray<NSNumber *> *inboxAudio;
@end

@implementation EivizAvPumpObj

- (instancetype)init {
    self = [super init];
    if (self) {
        _scratch = [NSMutableData dataWithCapacity:1024];
        _cond = [[NSCondition alloc] init];
        _inbox = [NSMutableArray array];
        _inboxAudio = [NSMutableArray array];
    }
    return self;
}

- (void)dealloc {
    [self shutdown];
}

- (void)shutdown {
    self.stopped = YES;
    [self.cond lock];
    [self.cond broadcast];
    [self.cond unlock];
    if (self.session) {
        [self.session stopRunning];
        self.session = nil;
    }
    [self.cond lock];
    for (NSValue *value in self.inbox) {
        CMSampleBufferRef buffer = value.pointerValue;
        if (buffer) {
            CFRelease(buffer);
        }
    }
    [self.inbox removeAllObjects];
    [self.inboxAudio removeAllObjects];
    [self.cond unlock];
    if (self.reader) {
        [self.reader cancelReading];
        self.reader = nil;
    }
    release_sample(&_pendingVideo);
    release_sample(&_pendingAudio);
}

- (BOOL)fillVideo:(CMSampleBufferRef)sample out:(EivizAvSample *)out {
    CVPixelBufferRef pixels = CMSampleBufferGetImageBuffer(sample);
    if (!pixels) {
        return NO;
    }
    CVPixelBufferLockBaseAddress(pixels, kCVPixelBufferLock_ReadOnly);
    size_t width = CVPixelBufferGetWidth(pixels);
    size_t height = CVPixelBufferGetHeight(pixels);
    size_t stride = CVPixelBufferGetBytesPerRow(pixels);
    const void *base = CVPixelBufferGetBaseAddress(pixels);
    if (!base || width == 0 || height == 0 || stride == 0) {
        CVPixelBufferUnlockBaseAddress(pixels, kCVPixelBufferLock_ReadOnly);
        return NO;
    }
    size_t bytes = stride * height;
    [self.scratch setLength:bytes];
    memcpy(self.scratch.mutableBytes, base, bytes);
    CVPixelBufferUnlockBaseAddress(pixels, kCVPixelBufferLock_ReadOnly);
    out->kind = EIVIZ_AV_KIND_VIDEO;
    out->width = (int32_t)width;
    out->height = (int32_t)height;
    out->stride = (int32_t)stride;
    out->sample_rate = 0;
    out->channels = 0;
    out->frames = 0;
    out->pts_hns = cm_to_hns(CMSampleBufferGetPresentationTimeStamp(sample));
    out->data = self.scratch.bytes;
    out->bytes = (uint32_t)bytes;
    return YES;
}

- (BOOL)fillAudio:(CMSampleBufferRef)sample out:(EivizAvSample *)out {
    CMFormatDescriptionRef format = CMSampleBufferGetFormatDescription(sample);
    if (!format) {
        return NO;
    }
    const AudioStreamBasicDescription *asbd = CMAudioFormatDescriptionGetStreamBasicDescription(format);
    if (!asbd || asbd->mChannelsPerFrame == 0) {
        return NO;
    }
    CMItemCount frames = CMSampleBufferGetNumSamples(sample);
    if (frames <= 0) {
        return NO;
    }
    AudioBufferList *list = NULL;
    CMBlockBufferRef block = NULL;
    size_t needed = 0;
    OSStatus status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
        sample,
        &needed,
        NULL,
        0,
        kCFAllocatorDefault,
        kCFAllocatorDefault,
        kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
        NULL
    );
    if (status != noErr && needed == 0) {
        return NO;
    }
    list = calloc(1, needed > 0 ? needed : sizeof(AudioBufferList));
    if (!list) {
        return NO;
    }
    status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
        sample,
        NULL,
        list,
        needed > 0 ? needed : sizeof(AudioBufferList),
        kCFAllocatorDefault,
        kCFAllocatorDefault,
        kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
        &block
    );
    if (status != noErr) {
        free(list);
        return NO;
    }
    uint32_t channels = asbd->mChannelsPerFrame;
    BOOL planar = (asbd->mFormatFlags & kAudioFormatFlagIsNonInterleaved) != 0;
    BOOL is_float = (asbd->mFormatFlags & kAudioFormatFlagIsFloat) != 0;
    uint32_t bits = asbd->mBitsPerChannel != 0 ? asbd->mBitsPerChannel : 32;
    [self.scratch setLength:(size_t)frames * channels * sizeof(float)];
    float *dst = self.scratch.mutableBytes;
    for (uint32_t ch = 0; ch < channels; ch++) {
        for (CMItemCount i = 0; i < frames; i++) {
            float sample_v = 0;
            if (planar) {
                if (ch < list->mNumberBuffers && list->mBuffers[ch].mData) {
                    const void *src = list->mBuffers[ch].mData;
                    if (is_float && bits == 32) {
                        sample_v = ((const float *)src)[i];
                    } else if (bits == 16) {
                        sample_v = ((const int16_t *)src)[i] / 32768.0f;
                    }
                }
            } else if (list->mNumberBuffers > 0 && list->mBuffers[0].mData) {
                const void *src = list->mBuffers[0].mData;
                if (is_float && bits == 32) {
                    sample_v = ((const float *)src)[i * channels + ch];
                } else if (bits == 16) {
                    sample_v = ((const int16_t *)src)[i * channels + ch] / 32768.0f;
                }
            }
            dst[ch * frames + i] = sample_v;
        }
    }
    if (block) {
        CFRelease(block);
    }
    free(list);
    out->kind = EIVIZ_AV_KIND_AUDIO;
    out->width = 0;
    out->height = 0;
    out->stride = 0;
    out->sample_rate = (int32_t)asbd->mSampleRate;
    out->channels = (int32_t)channels;
    out->frames = (int32_t)frames;
    out->pts_hns = cm_to_hns(CMSampleBufferGetPresentationTimeStamp(sample));
    out->data = self.scratch.bytes;
    out->bytes = (uint32_t)self.scratch.length;
    return YES;
}

- (int)nextFile:(EivizAvSample *)out {
    if (!self.pendingVideo && self.videoOut) {
        self.pendingVideo = [self.videoOut copyNextSampleBuffer];
    }
    if (!self.pendingAudio && self.audioOut) {
        self.pendingAudio = [self.audioOut copyNextSampleBuffer];
    }
    if (!self.pendingVideo && !self.pendingAudio) {
        if (self.reader.status == AVAssetReaderStatusCompleted ||
            self.reader.status == AVAssetReaderStatusFailed ||
            self.reader.status == AVAssetReaderStatusCancelled) {
            return EIVIZ_AV_EOF;
        }
        return EIVIZ_AV_RETRY;
    }
    BOOL take_audio = NO;
    if (self.pendingVideo && self.pendingAudio) {
        int64_t video_pts = cm_to_hns(CMSampleBufferGetPresentationTimeStamp(self.pendingVideo));
        int64_t audio_pts = cm_to_hns(CMSampleBufferGetPresentationTimeStamp(self.pendingAudio));
        take_audio = audio_pts <= video_pts;
    } else if (self.pendingAudio) {
        take_audio = YES;
    }
    CMSampleBufferRef chosen = take_audio ? self.pendingAudio : self.pendingVideo;
    BOOL ok = take_audio ? [self fillAudio:chosen out:out] : [self fillVideo:chosen out:out];
    if (take_audio) {
        release_sample(&_pendingAudio);
    } else {
        release_sample(&_pendingVideo);
    }
    return ok ? EIVIZ_AV_OK : EIVIZ_AV_RETRY;
}

- (int)nextCapture:(EivizAvSample *)out {
    [self.cond lock];
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:0.016];
    while (self.inbox.count == 0 && !self.stopped) {
        if (![self.cond waitUntilDate:deadline]) {
            break;
        }
    }
    CMSampleBufferRef buffer = NULL;
    BOOL audio = NO;
    if (self.inbox.count > 0) {
        buffer = self.inbox.firstObject.pointerValue;
        audio = self.inboxAudio.firstObject.boolValue;
        [self.inbox removeObjectAtIndex:0];
        [self.inboxAudio removeObjectAtIndex:0];
    }
    [self.cond unlock];
    if (!buffer) {
        return self.stopped ? EIVIZ_AV_EOF : EIVIZ_AV_RETRY;
    }
    BOOL ok = audio ? [self fillAudio:buffer out:out] : [self fillVideo:buffer out:out];
    CFRelease(buffer);
    return ok ? EIVIZ_AV_OK : EIVIZ_AV_RETRY;
}

- (void)enqueueCapture:(CMSampleBufferRef)sample audio:(BOOL)audio {
    if (self.stopped || !sample) {
        return;
    }
    CMSampleBufferRef copy = NULL;
    if (CMSampleBufferCreateCopy(kCFAllocatorDefault, sample, &copy) != noErr || !copy) {
        return;
    }
    [self.cond lock];
    while (self.inbox.count >= 8) {
        CMSampleBufferRef old = self.inbox.firstObject.pointerValue;
        [self.inbox removeObjectAtIndex:0];
        [self.inboxAudio removeObjectAtIndex:0];
        if (old) {
            CFRelease(old);
        }
    }
    [self.inbox addObject:[NSValue valueWithPointer:copy]];
    [self.inboxAudio addObject:@(audio)];
    [self.cond signal];
    [self.cond unlock];
}

- (void)captureOutput:(AVCaptureOutput *)output
    didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
           fromConnection:(AVCaptureConnection *)connection {
    (void)connection;
    [self enqueueCapture:sampleBuffer audio:(output == self.capAudio)];
}

@end

static NSDictionary *video_attrs(void) {
    return @{
        (NSString *)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA)
    };
}

static NSDictionary *audio_attrs(void) {
    return @{
        AVFormatIDKey: @(kAudioFormatLinearPCM),
        AVLinearPCMBitDepthKey: @32,
        AVLinearPCMIsFloatKey: @YES,
        AVLinearPCMIsNonInterleaved: @YES,
        AVLinearPCMIsBigEndianKey: @NO
    };
}

static BOOL wait_keys(AVAsset *asset, NSArray<NSString *> *keys) {
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    [asset loadValuesAsynchronouslyForKeys:keys completionHandler:^{
        dispatch_semaphore_signal(done);
    }];
    return dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 15 * NSEC_PER_SEC)) == 0;
}

EivizAvPump *eiviz_av_open_file(const char *path, int64_t start_hns) {
    if (!path || path[0] == 0) {
        return NULL;
    }
    NSString *ns_path = [NSString stringWithUTF8String:path];
    if (![[NSFileManager defaultManager] fileExistsAtPath:ns_path]) {
        return NULL;
    }
    NSURL *url = [NSURL fileURLWithPath:ns_path];
    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:nil];
    if (!wait_keys(asset, @[@"tracks", @"duration"])) {
        return NULL;
    }
    NSError *error = nil;
    if ([asset statusOfValueForKey:@"tracks" error:&error] != AVKeyValueStatusLoaded) {
        return NULL;
    }
    AVAssetTrack *video = [asset tracksWithMediaType:AVMediaTypeVideo].firstObject;
    if (!video) {
        return NULL;
    }
    AVAssetReader *reader = [AVAssetReader assetReaderWithAsset:asset error:&error];
    if (!reader) {
        return NULL;
    }
    if (start_hns > 0) {
        reader.timeRange = CMTimeRangeMake(CMTimeMake(start_hns, 10000000), kCMTimePositiveInfinity);
    }
    AVAssetReaderTrackOutput *video_out =
        [AVAssetReaderTrackOutput assetReaderTrackOutputWithTrack:video outputSettings:video_attrs()];
    video_out.alwaysCopiesSampleData = NO;
    if (![reader canAddOutput:video_out]) {
        return NULL;
    }
    [reader addOutput:video_out];
    AVAssetReaderTrackOutput *audio_out = nil;
    AVAssetTrack *audio = [asset tracksWithMediaType:AVMediaTypeAudio].firstObject;
    if (audio) {
        audio_out = [AVAssetReaderTrackOutput assetReaderTrackOutputWithTrack:audio outputSettings:audio_attrs()];
        audio_out.alwaysCopiesSampleData = NO;
        if ([reader canAddOutput:audio_out]) {
            [reader addOutput:audio_out];
        } else {
            audio_out = nil;
        }
    }
    if (![reader startReading]) {
        return NULL;
    }
    EivizAvPumpObj *pump = [[EivizAvPumpObj alloc] init];
    pump.reader = reader;
    pump.videoOut = video_out;
    pump.audioOut = audio_out;
    pump.durationHns = cm_to_hns(asset.duration);
    return (__bridge_retained EivizAvPump *)pump;
}

EivizAvPump *eiviz_av_open_capture(const char *device_id) {
    NSString *uid = device_id ? [NSString stringWithUTF8String:device_id] : @"";
    AVCaptureDevice *device = nil;
    if (uid.length > 0) {
        device = [AVCaptureDevice deviceWithUniqueID:uid];
    }
    if (!device) {
        device = [AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeVideo];
    }
    if (!device) {
        return NULL;
    }
    NSError *error = nil;
    AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:device error:&error];
    if (!input) {
        return NULL;
    }
    AVCaptureSession *session = [[AVCaptureSession alloc] init];
    if ([session canSetSessionPreset:AVCaptureSessionPresetHigh]) {
        session.sessionPreset = AVCaptureSessionPresetHigh;
    }
    if (![session canAddInput:input]) {
        return NULL;
    }
    [session addInput:input];
    EivizAvPumpObj *pump = [[EivizAvPumpObj alloc] init];
    pump.capture = YES;
    pump.capQueue = dispatch_queue_create("eiviz.av.capture", DISPATCH_QUEUE_SERIAL);
    AVCaptureVideoDataOutput *video = [[AVCaptureVideoDataOutput alloc] init];
    video.videoSettings = video_attrs();
    video.alwaysDiscardsLateVideoFrames = YES;
    [video setSampleBufferDelegate:pump queue:pump.capQueue];
    if ([session canAddOutput:video]) {
        [session addOutput:video];
        pump.capVideo = video;
    }
    if (!pump.capVideo) {
        return NULL;
    }
    pump.session = session;
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        [session startRunning];
    });
    return (__bridge_retained EivizAvPump *)pump;
}

void eiviz_av_close(EivizAvPump *pump) {
    if (!pump) {
        return;
    }
    EivizAvPumpObj *owned = (__bridge_transfer EivizAvPumpObj *)pump;
    [owned shutdown];
}

int64_t eiviz_av_duration_hns(const EivizAvPump *pump) {
    if (!pump) {
        return 0;
    }
    EivizAvPumpObj *owned = (__bridge EivizAvPumpObj *)pump;
    return owned.durationHns;
}

int eiviz_av_next(EivizAvPump *pump, EivizAvSample *out) {
    if (!pump || !out) {
        return EIVIZ_AV_ERR;
    }
    memset(out, 0, sizeof(*out));
    EivizAvPumpObj *owned = (__bridge EivizAvPumpObj *)pump;
    if (owned.stopped) {
        return EIVIZ_AV_EOF;
    }
    return owned.capture ? [owned nextCapture:out] : [owned nextFile:out];
}

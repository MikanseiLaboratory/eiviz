#import "audio_tap.h"

#import <AppKit/AppKit.h>
#import <CoreAudio/AudioHardware.h>
#import <CoreAudio/AudioHardwareTapping.h>
#import <CoreAudio/CATapDescription.h>
#import <CoreAudio/CoreAudio.h>
#import <Foundation/Foundation.h>
#import <dispatch/dispatch.h>
#import <unistd.h>

static void set_error(char *err, size_t cap, NSString *message) {
    if (!err || cap == 0) {
        return;
    }
    const char *text = message.UTF8String ?: "";
    strncpy(err, text, cap - 1);
    err[cap - 1] = 0;
}

static NSString *key_string(const char *text) {
    return [NSString stringWithUTF8String:text];
}

static NSString *cfstring_to_ns(CFStringRef text) {
    if (!text) {
        return nil;
    }
    return (__bridge_transfer NSString *)text;
}

static CFStringRef copy_default_output_uid(void) {
    AudioObjectPropertyAddress address = {
        .mSelector = kAudioHardwarePropertyDefaultSystemOutputDevice,
        .mScope = kAudioObjectPropertyScopeGlobal,
        .mElement = kAudioObjectPropertyElementMain,
    };
    AudioDeviceID device = kAudioObjectUnknown;
    UInt32 size = sizeof(device);
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, &device) != noErr ||
        device == kAudioObjectUnknown) {
        return NULL;
    }
    address.mSelector = kAudioDevicePropertyDeviceUID;
    CFStringRef uid = NULL;
    size = sizeof(uid);
    if (AudioObjectGetPropertyData(device, &address, 0, NULL, &size, &uid) != noErr) {
        return NULL;
    }
    return uid;
}

@interface EivizAudioTapObj : NSObject
@property(nonatomic) AudioObjectID tapID;
@property(nonatomic) AudioDeviceID aggregateID;
@property(nonatomic) AudioDeviceIOProcID procID;
@property(nonatomic) AudioStreamBasicDescription format;
@property(nonatomic) dispatch_queue_t queue;
@property(nonatomic) EivizAudioTapCallback callback;
@property(nonatomic) void *callbackCtx;
@property(nonatomic) float *scratch;
@property(nonatomic) size_t scratchCapacity;
@property(nonatomic) BOOL running;
@end

@implementation EivizAudioTapObj

- (void)dealloc {
    [self stop];
}

- (void)stop {
    if (!self.running) {
        return;
    }
    self.running = NO;
    self.callback = NULL;
    if (self.aggregateID != kAudioObjectUnknown) {
        AudioDeviceStop(self.aggregateID, self.procID);
        if (self.procID) {
            AudioDeviceDestroyIOProcID(self.aggregateID, self.procID);
            self.procID = NULL;
        }
        AudioHardwareDestroyAggregateDevice(self.aggregateID);
        self.aggregateID = kAudioObjectUnknown;
    }
    if (self.tapID != kAudioObjectUnknown) {
        AudioHardwareDestroyProcessTap(self.tapID);
        self.tapID = kAudioObjectUnknown;
    }
    free(self.scratch);
    self.scratch = NULL;
    self.scratchCapacity = 0;
}

- (BOOL)openMode:(uint32_t)mode
        deviceId:(NSString *)deviceId
      processExe:(NSString *)processExe
    processAumid:(NSString *)processAumid
        callback:(EivizAudioTapCallback)callback
             ctx:(void *)ctx
             err:(char *)err
          errCap:(size_t)errCap {
    if (@available(macOS 14.4, *)) {
        // fall through
    } else {
        set_error(err, errCap, @"Core Audio process taps need macOS 14.4 or later");
        return NO;
    }
    self.tapID = kAudioObjectUnknown;
    self.aggregateID = kAudioObjectUnknown;
    self.procID = NULL;
    self.callback = callback;
    self.callbackCtx = ctx;

    CATapDescription *description = nil;
    if (mode == 1) {
        description = [[CATapDescription alloc]
            initStereoGlobalTapButExcludeProcesses:@[ @(getpid()) ]];
    } else {
        NSMutableArray<NSNumber *> *objects = [NSMutableArray array];
        NSString *wantedExe = processExe ?: @"";
        NSString *wantedBundle = processAumid ?: @"";
        for (NSRunningApplication *app in NSWorkspace.sharedWorkspace.runningApplications) {
            if (app.processIdentifier == getpid()) {
                continue;
            }
            BOOL match = NO;
            if (wantedBundle.length > 0 && [app.bundleIdentifier isEqualToString:wantedBundle]) {
                match = YES;
            }
            if (!match && wantedExe.length > 0 && [app.executableURL.path isEqualToString:wantedExe]) {
                match = YES;
            }
            if (!match && wantedExe.length > 0 &&
                [[app.executableURL.path lastPathComponent] isEqualToString:[wantedExe lastPathComponent]]) {
                match = YES;
            }
            if (!match) {
                continue;
            }
            AudioObjectPropertyAddress address = {
                .mSelector = kAudioHardwarePropertyTranslatePIDToProcessObject,
                .mScope = kAudioObjectPropertyScopeGlobal,
                .mElement = kAudioObjectPropertyElementMain,
            };
            pid_t pid = app.processIdentifier;
            AudioObjectID objectID = kAudioObjectUnknown;
            UInt32 size = sizeof(objectID);
            if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, sizeof(pid), &pid, &size,
                                           &objectID) == noErr &&
                objectID != kAudioObjectUnknown) {
                [objects addObject:@(objectID)];
            }
        }
        if (objects.count == 0) {
            set_error(err, errCap, @"No running application matched the selected audio source");
            return NO;
        }
        description = [[CATapDescription alloc] initStereoMixdownOfProcesses:objects];
    }
    if (!description) {
        set_error(err, errCap, @"Could not build the Core Audio tap description");
        return NO;
    }
    description.UUID = [NSUUID UUID];
    description.muteBehavior = CATapUnmuted;

    OSStatus status = AudioHardwareCreateProcessTap(description, &_tapID);
    if (status != noErr || self.tapID == kAudioObjectUnknown) {
        set_error(err, errCap, [NSString stringWithFormat:@"AudioHardwareCreateProcessTap failed: %d", (int)status]);
        return NO;
    }

    AudioObjectPropertyAddress formatAddress = {
        .mSelector = kAudioTapPropertyFormat,
        .mScope = kAudioObjectPropertyScopeGlobal,
        .mElement = kAudioObjectPropertyElementMain,
    };
    AudioStreamBasicDescription format = {0};
    UInt32 formatSize = sizeof(format);
    status = AudioObjectGetPropertyData(self.tapID, &formatAddress, 0, NULL, &formatSize, &format);
    if (status != noErr) {
        [self stop];
        set_error(err, errCap, [NSString stringWithFormat:@"AudioTap format failed: %d", (int)status]);
        return NO;
    }

    self.format = format;
    CFStringRef outputUID = copy_default_output_uid();
    if (!outputUID) {
        [self stop];
        set_error(err, errCap, @"No default output device is available for the tap");
        return NO;
    }
    NSString *outputUIDText = cfstring_to_ns(outputUID);
    NSString *aggregateUID = [NSUUID UUID].UUIDString;
    NSDictionary *aggregate = @{
        key_string(kAudioAggregateDeviceNameKey) : @"eiviz audio tap",
        key_string(kAudioAggregateDeviceUIDKey) : aggregateUID,
        key_string(kAudioAggregateDeviceMainSubDeviceKey) : outputUIDText,
        key_string(kAudioAggregateDeviceIsPrivateKey) : @YES,
        key_string(kAudioAggregateDeviceIsStackedKey) : @NO,
        key_string(kAudioAggregateDeviceTapAutoStartKey) : @YES,
        key_string(kAudioAggregateDeviceSubDeviceListKey) : @[
            @{ key_string(kAudioSubDeviceUIDKey) : outputUIDText }
        ],
        key_string(kAudioAggregateDeviceTapListKey) : @[
            @{
                key_string(kAudioSubTapDriftCompensationKey) : @YES,
                key_string(kAudioSubTapUIDKey) : description.UUID.UUIDString,
            }
        ],
    };
    status = AudioHardwareCreateAggregateDevice((__bridge CFDictionaryRef)aggregate, &_aggregateID);
    if (status != noErr || self.aggregateID == kAudioObjectUnknown) {
        [self stop];
        set_error(err, errCap, [NSString stringWithFormat:@"AudioHardwareCreateAggregateDevice failed: %d", (int)status]);
        return NO;
    }

    self.queue = dispatch_queue_create("eiviz.audio.tap", DISPATCH_QUEUE_SERIAL);
    __weak EivizAudioTapObj *weakSelf = self;
    status = AudioDeviceCreateIOProcIDWithBlock(&_procID, self.aggregateID, self.queue,
        ^(const AudioTimeStamp *inNow, const AudioBufferList *inInputData,
          const AudioTimeStamp *inInputTime, AudioBufferList *outOutputData, const AudioTimeStamp *inOutputTime) {
            (void)inNow; (void)inInputTime; (void)outOutputData; (void)inOutputTime;
            [weakSelf handleInput:inInputData];
        });
    if (status != noErr) {
        [self stop];
        set_error(err, errCap, [NSString stringWithFormat:@"AudioDeviceCreateIOProcIDWithBlock failed: %d", (int)status]);
        return NO;
    }
    status = AudioDeviceStart(self.aggregateID, self.procID);
    if (status != noErr) {
        [self stop];
        set_error(err, errCap, [NSString stringWithFormat:@"AudioDeviceStart failed: %d", (int)status]);
        return NO;
    }
    self.running = YES;
    return YES;
}

- (void)handleInput:(const AudioBufferList *)input {
    if (!input || !self.callback || input->mNumberBuffers == 0) {
        return;
    }
    uint32_t channels = self.format.mChannelsPerFrame;
    if (channels == 0 || channels > 32) {
        return;
    }
    const AudioBuffer *first = &input->mBuffers[0];
    uint32_t frames = (uint32_t)(first->mDataByteSize / MAX(1, (self.format.mBitsPerChannel / 8) * first->mNumberChannels));
    if (frames == 0) {
        return;
    }
    size_t needed = (size_t)frames * channels;
    if (needed > self.scratchCapacity) {
        float *grown = realloc(self.scratch, needed * sizeof(float));
        if (!grown) {
            return;
        }
        self.scratch = grown;
        self.scratchCapacity = needed;
    }
    BOOL isFloat = (self.format.mFormatFlags & kAudioFormatFlagIsFloat) != 0;
    BOOL planar = (self.format.mFormatFlags & kAudioFormatFlagIsNonInterleaved) != 0;
    uint32_t bits = self.format.mBitsPerChannel ?: 32;

    if (!planar) {
        uint32_t inChannels = first->mNumberChannels ?: channels;
        for (uint32_t frame = 0; frame < frames; frame++) {
            for (uint32_t channel = 0; channel < channels; channel++) {
                float value = 0;
                uint32_t sourceChannel = MIN(channel, inChannels - 1);
                if (isFloat && bits == 32) {
                    value = ((const float *)first->mData)[(size_t)frame * inChannels + sourceChannel];
                } else if (bits == 16) {
                    value = ((const int16_t *)first->mData)[(size_t)frame * inChannels + sourceChannel] / 32768.0f;
                } else if (isFloat && bits == 64) {
                    value = (float)((const double *)first->mData)[(size_t)frame * inChannels + sourceChannel];
                }
                self.scratch[(size_t)frame * channels + channel] = value;
            }
        }
    } else {
        uint32_t buffers = MIN(input->mNumberBuffers, channels);
        if (buffers == 0) {
            return;
        }
        for (uint32_t channel = 0; channel < channels; channel++) {
            const AudioBuffer *buffer = &input->mBuffers[MIN(channel, buffers - 1)];
            for (uint32_t frame = 0; frame < frames; frame++) {
                float value = 0;
                if (isFloat && bits == 32) {
                    value = ((const float *)buffer->mData)[frame];
                } else if (bits == 16) {
                    value = ((const int16_t *)buffer->mData)[frame] / 32768.0f;
                } else if (isFloat && bits == 64) {
                    value = (float)((const double *)buffer->mData)[frame];
                }
                self.scratch[(size_t)frame * channels + channel] = value;
            }
        }
    }
    self.callback(self.callbackCtx, self.scratch, frames, channels, self.format.mSampleRate);
}

@end

EivizAudioTap *eiviz_audio_tap_start(
    uint32_t mode,
    const char *device_id,
    const char *process_exe,
    const char *process_aumid,
    EivizAudioTapCallback callback,
    void *ctx,
    char *err,
    size_t err_cap) {
    @autoreleasepool {
        (void)device_id;
        EivizAudioTapObj *tap = [[EivizAudioTapObj alloc] init];
        NSString *exe = process_exe ? [NSString stringWithUTF8String:process_exe] : @"";
        NSString *aumid = process_aumid ? [NSString stringWithUTF8String:process_aumid] : @"";
        NSString *device = device_id ? [NSString stringWithUTF8String:device_id] : @"";
        if (![tap openMode:mode deviceId:device processExe:exe processAumid:aumid callback:callback ctx:ctx err:err errCap:err_cap]) {
            return NULL;
        }
        return (__bridge_retained EivizAudioTap *)tap;
    }
}

void eiviz_audio_tap_stop(EivizAudioTap *tap) {
    if (!tap) {
        return;
    }
    EivizAudioTapObj *obj = (__bridge_transfer EivizAudioTapObj *)tap;
    [obj stop];
}

int32_t eiviz_audio_process_list(uint8_t *out, size_t cap) {
    @autoreleasepool {
        NSMutableArray *items = [NSMutableArray array];
        pid_t selfPid = getpid();
        for (NSRunningApplication *app in NSWorkspace.sharedWorkspace.runningApplications) {
            if (app.processIdentifier == selfPid || app.activationPolicy != NSApplicationActivationPolicyRegular) {
                continue;
            }
            NSString *name = app.localizedName ?: app.bundleIdentifier ?: @"Unknown";
            NSString *exe = app.executableURL.path ?: @"";
            NSString *aumid = app.bundleIdentifier ?: @"";
            [items addObject:@{ @"name": name, @"exe": exe, @"aumid": aumid }];
        }
        NSData *data = [NSJSONSerialization dataWithJSONObject:items options:0 error:nil];
        if (!data || data.length > cap) {
            return -1;
        }
        memcpy(out, data.bytes, data.length);
        return (int32_t)data.length;
    }
}

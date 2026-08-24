#include "eiviz_decklink_shim.h"

#include <DeckLinkAPI.h>

#include <algorithm>
#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>

#if defined(_WIN32)
#include <combaseapi.h>
#include <oleauto.h>
#include <windows.h>
#elif defined(__APPLE__)
#include <CoreFoundation/CoreFoundation.h>
#endif

namespace {

constexpr BMDTimeScale kVideoTimeScale = 60000;
constexpr BMDTimeScale kAudioTimeScale = 48000;
constexpr int32_t kWidth = 1920;
constexpr int32_t kHeight = 1080;

void set_error(char* destination, size_t capacity, const std::string& message) {
    if (destination == nullptr || capacity == 0) {
        return;
    }
    std::snprintf(destination, capacity, "%s", message.c_str());
}

std::string hr_error(const char* operation, HRESULT result) {
    char message[128];
    std::snprintf(
        message,
        sizeof(message),
        "%s failed (HRESULT 0x%08x)",
        operation,
        static_cast<unsigned int>(result));
    return message;
}

std::string decklink_string(BMD_STR value) {
    if (value == nullptr) {
        return {};
    }
#if defined(_WIN32)
    const int required =
        WideCharToMultiByte(CP_UTF8, 0, value, -1, nullptr, 0, nullptr, nullptr);
    std::string converted;
    if (required > 1) {
        converted.resize(static_cast<size_t>(required));
        WideCharToMultiByte(
            CP_UTF8,
            0,
            value,
            -1,
            converted.data(),
            required,
            nullptr,
            nullptr);
        converted.resize(static_cast<size_t>(required - 1));
    }
    SysFreeString(value);
    return converted;
#elif defined(__APPLE__)
    const CFIndex length = CFStringGetLength(value);
    const CFIndex capacity =
        CFStringGetMaximumSizeForEncoding(length, kCFStringEncodingUTF8) + 1;
    std::string converted(static_cast<size_t>(capacity), '\0');
    if (!CFStringGetCString(
            value, converted.data(), capacity, kCFStringEncodingUTF8)) {
        converted.clear();
    } else {
        converted.resize(std::strlen(converted.c_str()));
    }
    CFRelease(value);
    return converted;
#else
    std::string converted(value);
    std::free(const_cast<char*>(value));
    return converted;
#endif
}

IDeckLinkIterator* create_iterator(bool* com_initialized, std::string* error) {
#if defined(_WIN32)
    const HRESULT init = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(init) && init != RPC_E_CHANGED_MODE) {
        *error = hr_error("CoInitializeEx", init);
        return nullptr;
    }
    *com_initialized = SUCCEEDED(init);
    IDeckLinkIterator* iterator = nullptr;
    const HRESULT result = CoCreateInstance(
        CLSID_CDeckLinkIterator,
        nullptr,
        CLSCTX_ALL,
        IID_IDeckLinkIterator,
        reinterpret_cast<void**>(&iterator));
    if (FAILED(result)) {
        if (*com_initialized) {
            CoUninitialize();
            *com_initialized = false;
        }
        *error = hr_error("CoCreateInstance(CDeckLinkIterator)", result);
        return nullptr;
    }
    return iterator;
#else
    (void)com_initialized;
    IDeckLinkIterator* iterator = CreateDeckLinkIteratorInstance();
    if (iterator == nullptr) {
        *error = "Desktop Video driver did not create a DeckLink iterator";
    }
    return iterator;
#endif
}

void release_com(bool initialized) {
#if defined(_WIN32)
    if (initialized) {
        CoUninitialize();
    }
#else
    (void)initialized;
#endif
}

std::string display_name(IDeckLink* device) {
    BMD_STR value = nullptr;
    if (device->GetDisplayName(&value) != S_OK) {
        return "Unnamed DeckLink";
    }
    return decklink_string(value);
}

std::string persistent_id(IDeckLink* device, const std::string& name) {
    IDeckLinkProfileAttributes* attributes = nullptr;
    if (device->QueryInterface(
            IID_IDeckLinkProfileAttributes,
            reinterpret_cast<void**>(&attributes)) == S_OK) {
        int64_t value = 0;
        if (attributes->GetInt(BMDDeckLinkPersistentID, &value) == S_OK) {
            attributes->Release();
            char id[32];
            std::snprintf(
                id, sizeof(id), "persistent:%016llx", static_cast<long long>(value));
            return id;
        }
        if (attributes->GetInt(BMDDeckLinkTopologicalID, &value) == S_OK) {
            attributes->Release();
            char id[32];
            std::snprintf(
                id, sizeof(id), "topological:%016llx", static_cast<long long>(value));
            return id;
        }
        attributes->Release();
    }
    return "unstable:" + name;
}

uint32_t device_capabilities(IDeckLink* device) {
    uint32_t capabilities = 0;
    IDeckLinkInput* input = nullptr;
    if (device->QueryInterface(IID_IDeckLinkInput, reinterpret_cast<void**>(&input)) ==
        S_OK) {
        capabilities |= EIVIZ_DECKLINK_DEVICE_CAPTURE;
        input->Release();
    }
    IDeckLinkOutput* output = nullptr;
    if (device->QueryInterface(
            IID_IDeckLinkOutput, reinterpret_cast<void**>(&output)) == S_OK) {
        capabilities |= EIVIZ_DECKLINK_DEVICE_PLAYBACK;
        output->Release();
    }
    return capabilities;
}

IDeckLink* find_device(
    const char* wanted_id, bool* com_initialized, std::string* error) {
    IDeckLinkIterator* iterator = create_iterator(com_initialized, error);
    if (iterator == nullptr) {
        return nullptr;
    }
    IDeckLink* selected = nullptr;
    IDeckLink* device = nullptr;
    while (iterator->Next(&device) == S_OK) {
        const std::string name = display_name(device);
        if (persistent_id(device, name) == wanted_id) {
            selected = device;
            break;
        }
        device->Release();
        device = nullptr;
    }
    iterator->Release();
    if (selected == nullptr) {
        *error = "the selected DeckLink persistent hardware ID is not present";
        release_com(*com_initialized);
        *com_initialized = false;
    }
    return selected;
}

bool supports_capture_mode(IDeckLinkInput* input, std::string* error) {
    BMDDisplayMode actual_mode = bmdModeUnknown;
    bool supported = false;
    const HRESULT result = input->DoesSupportVideoMode(
        bmdVideoConnectionUnspecified,
        bmdModeHD1080p5994,
        bmdFormat8BitBGRA,
        bmdNoVideoInputConversion,
        bmdSupportedVideoModeDefault,
        &actual_mode,
        &supported);
    if (result != S_OK || !supported || actual_mode != bmdModeHD1080p5994) {
        *error = "device does not support exact 1080p59.94 8-bit BGRA capture";
        return false;
    }
    return true;
}

bool supports_playback_mode(IDeckLinkOutput* output, std::string* error) {
    BMDDisplayMode actual_mode = bmdModeUnknown;
    bool supported = false;
    const HRESULT result = output->DoesSupportVideoMode(
        bmdVideoConnectionUnspecified,
        bmdModeHD1080p5994,
        bmdFormat8BitBGRA,
        bmdNoVideoOutputConversion,
        bmdSupportedVideoModeDefault,
        &actual_mode,
        &supported);
    if (result != S_OK || !supported || actual_mode != bmdModeHD1080p5994) {
        *error = "device does not support exact 1080p59.94 8-bit BGRA playback";
        return false;
    }
    return true;
}

class CaptureCallback final : public IDeckLinkInputCallback {
public:
    CaptureCallback(
        eiviz_decklink_video_callback video_callback,
        eiviz_decklink_audio_callback audio_callback,
        void* context,
        uint32_t audio_channels)
        : references_(1),
          video_callback_(video_callback),
          audio_callback_(audio_callback),
          context_(context),
          audio_channels_(audio_channels) {}

    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID interface_id, LPVOID* object) override {
        if (object == nullptr) {
            return E_INVALIDARG;
        }
        if (interface_id == IID_IUnknown ||
            interface_id == IID_IDeckLinkInputCallback) {
            *object = static_cast<IDeckLinkInputCallback*>(this);
            AddRef();
            return S_OK;
        }
        *object = nullptr;
        return E_NOINTERFACE;
    }

    ULONG STDMETHODCALLTYPE AddRef() override {
        return ++references_;
    }

    ULONG STDMETHODCALLTYPE Release() override {
        const ULONG remaining = --references_;
        if (remaining == 0) {
            delete this;
        }
        return remaining;
    }

    HRESULT STDMETHODCALLTYPE VideoInputFormatChanged(
        BMDVideoInputFormatChangedEvents,
        IDeckLinkDisplayMode*,
        BMDDetectedVideoInputFormatFlags) override {
        // This vertical slice is intentionally fixed to 1080p59.94.
        return S_OK;
    }

    HRESULT STDMETHODCALLTYPE VideoInputFrameArrived(
        IDeckLinkVideoInputFrame* video,
        IDeckLinkAudioInputPacket* audio) override {
        if (video != nullptr && video_callback_ != nullptr) {
            void* bytes = nullptr;
            BMDTimeValue stream_time = 0;
            BMDTimeValue duration = 0;
            if (video->GetBytes(&bytes) == S_OK &&
                video->GetStreamTime(
                    &stream_time, &duration, kVideoTimeScale) == S_OK) {
                const int32_t height = video->GetHeight();
                const int32_t row_bytes = video->GetRowBytes();
                if (height > 0 && row_bytes > 0) {
                    eiviz_decklink_video_frame frame = {};
                    frame.data = static_cast<const uint8_t*>(bytes);
                    frame.data_len =
                        static_cast<size_t>(height) * static_cast<size_t>(row_bytes);
                    frame.width = static_cast<uint32_t>(video->GetWidth());
                    frame.height = static_cast<uint32_t>(height);
                    frame.row_bytes = static_cast<uint32_t>(row_bytes);
                    frame.flags =
                        (video->GetFlags() & bmdFrameHasNoInputSource) != 0
                            ? EIVIZ_DECKLINK_FRAME_NO_INPUT
                            : 0;
                    frame.stream_time = stream_time;
                    frame.duration = duration;
                    frame.time_scale = kVideoTimeScale;
                    video_callback_(context_, &frame);
                }
            }
        }
        if (audio != nullptr && audio_callback_ != nullptr) {
            void* bytes = nullptr;
            BMDTimeValue packet_time = 0;
            if (audio->GetBytes(&bytes) == S_OK &&
                audio->GetPacketTime(&packet_time, kAudioTimeScale) == S_OK) {
                const uint32_t frames =
                    static_cast<uint32_t>(audio->GetSampleFrameCount());
                eiviz_decklink_audio_packet packet = {};
                packet.samples = static_cast<const int16_t*>(bytes);
                packet.sample_count =
                    static_cast<size_t>(frames) * audio_channels_;
                packet.frame_count = frames;
                packet.channels = audio_channels_;
                packet.sample_rate = static_cast<uint32_t>(kAudioTimeScale);
                packet.packet_time = packet_time;
                packet.time_scale = kAudioTimeScale;
                audio_callback_(context_, &packet);
            }
        }
        return S_OK;
    }

private:
    std::atomic<ULONG> references_;
    eiviz_decklink_video_callback video_callback_;
    eiviz_decklink_audio_callback audio_callback_;
    void* context_;
    uint32_t audio_channels_;
};

class PlaybackCallback final : public IDeckLinkVideoOutputCallback {
public:
    PlaybackCallback()
        : references_(1),
          completed_(0),
          late_(0),
          dropped_(0),
          flushed_(0) {}

    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID interface_id, LPVOID* object) override {
        if (object == nullptr) {
            return E_INVALIDARG;
        }
        if (interface_id == IID_IUnknown ||
            interface_id == IID_IDeckLinkVideoOutputCallback) {
            *object = static_cast<IDeckLinkVideoOutputCallback*>(this);
            AddRef();
            return S_OK;
        }
        *object = nullptr;
        return E_NOINTERFACE;
    }

    ULONG STDMETHODCALLTYPE AddRef() override {
        return ++references_;
    }

    ULONG STDMETHODCALLTYPE Release() override {
        const ULONG remaining = --references_;
        if (remaining == 0) {
            delete this;
        }
        return remaining;
    }

    HRESULT STDMETHODCALLTYPE ScheduledFrameCompleted(
        IDeckLinkVideoFrame* frame,
        BMDOutputFrameCompletionResult result) override {
        completed_.fetch_add(1, std::memory_order_relaxed);
        switch (result) {
            case bmdOutputFrameDisplayedLate:
                late_.fetch_add(1, std::memory_order_relaxed);
                break;
            case bmdOutputFrameDropped:
                dropped_.fetch_add(1, std::memory_order_relaxed);
                break;
            case bmdOutputFrameFlushed:
                flushed_.fetch_add(1, std::memory_order_relaxed);
                break;
            default:
                break;
        }
        if (frame != nullptr) {
            frame->Release();
        }
        return S_OK;
    }

    HRESULT STDMETHODCALLTYPE ScheduledPlaybackHasStopped() override {
        return S_OK;
    }

    uint64_t completed() const {
        return completed_.load(std::memory_order_relaxed);
    }
    uint64_t late() const {
        return late_.load(std::memory_order_relaxed);
    }
    uint64_t dropped() const {
        return dropped_.load(std::memory_order_relaxed);
    }
    uint64_t flushed() const {
        return flushed_.load(std::memory_order_relaxed);
    }

private:
    std::atomic<ULONG> references_;
    std::atomic<uint64_t> completed_;
    std::atomic<uint64_t> late_;
    std::atomic<uint64_t> dropped_;
    std::atomic<uint64_t> flushed_;
};

}  // namespace

struct eiviz_decklink_capture {
    IDeckLink* device;
    IDeckLinkInput* input;
    CaptureCallback* callback;
    bool com_initialized;
};

struct eiviz_decklink_playback {
    IDeckLink* device;
    IDeckLinkOutput* output;
    IDeckLinkStatus* status;
    PlaybackCallback* callback;
    uint32_t audio_channels;
    std::atomic<uint64_t> scheduled_video;
    bool started;
    bool com_initialized;
};

extern "C" uint32_t eiviz_decklink_abi_version(void) {
    return EIVIZ_DECKLINK_ABI_VERSION;
}

extern "C" int32_t eiviz_decklink_enumerate(
    eiviz_decklink_device_callback callback,
    void* context,
    char* error,
    size_t error_capacity) {
    if (callback == nullptr) {
        set_error(error, error_capacity, "device callback is null");
        return -1;
    }
    bool com_initialized = false;
    std::string detail;
    IDeckLinkIterator* iterator = create_iterator(&com_initialized, &detail);
    if (iterator == nullptr) {
        set_error(error, error_capacity, detail);
        return -1;
    }
    IDeckLink* device = nullptr;
    while (iterator->Next(&device) == S_OK) {
        const std::string name = display_name(device);
        const std::string id = persistent_id(device, name);
        const eiviz_decklink_device info = {
            id.c_str(), name.c_str(), device_capabilities(device)};
        callback(context, &info);
        device->Release();
        device = nullptr;
    }
    iterator->Release();
    release_com(com_initialized);
    return 0;
}

extern "C" int32_t eiviz_decklink_capture_open(
    const char* persistent_id_value,
    uint32_t audio_channels,
    eiviz_decklink_video_callback video_callback,
    eiviz_decklink_audio_callback audio_callback,
    void* context,
    eiviz_decklink_capture** capture,
    char* error,
    size_t error_capacity) {
    if (persistent_id_value == nullptr || video_callback == nullptr ||
        audio_callback == nullptr || capture == nullptr || audio_channels == 0) {
        set_error(error, error_capacity, "invalid capture-open arguments");
        return -1;
    }
    *capture = nullptr;
    bool com_initialized = false;
    std::string detail;
    IDeckLink* device =
        find_device(persistent_id_value, &com_initialized, &detail);
    if (device == nullptr) {
        set_error(error, error_capacity, detail);
        return -1;
    }
    IDeckLinkInput* input = nullptr;
    HRESULT result =
        device->QueryInterface(IID_IDeckLinkInput, reinterpret_cast<void**>(&input));
    if (result != S_OK || input == nullptr) {
        device->Release();
        release_com(com_initialized);
        set_error(error, error_capacity, "selected device has no DeckLink input");
        return -1;
    }
    if (!supports_capture_mode(input, &detail)) {
        input->Release();
        device->Release();
        release_com(com_initialized);
        set_error(error, error_capacity, detail);
        return -1;
    }
    CaptureCallback* callback =
        new CaptureCallback(video_callback, audio_callback, context, audio_channels);
    result = input->SetCallback(callback);
    if (result == S_OK) {
        result = input->EnableVideoInput(
            bmdModeHD1080p5994, bmdFormat8BitBGRA, bmdVideoInputFlagDefault);
    }
    if (result == S_OK) {
        result = input->EnableAudioInput(
            bmdAudioSampleRate48kHz,
            bmdAudioSampleType16bitInteger,
            audio_channels);
    }
    if (result == S_OK) {
        result = input->StartStreams();
    }
    if (result != S_OK) {
        input->StopStreams();
        input->DisableAudioInput();
        input->DisableVideoInput();
        input->SetCallback(nullptr);
        callback->Release();
        input->Release();
        device->Release();
        release_com(com_initialized);
        set_error(error, error_capacity, hr_error("starting DeckLink capture", result));
        return -1;
    }
    *capture = new eiviz_decklink_capture{
        device, input, callback, com_initialized};
    return 0;
}

extern "C" void eiviz_decklink_capture_close(
    eiviz_decklink_capture* capture) {
    if (capture == nullptr) {
        return;
    }
    capture->input->StopStreams();
    capture->input->DisableAudioInput();
    capture->input->DisableVideoInput();
    capture->input->SetCallback(nullptr);
    capture->callback->Release();
    capture->input->Release();
    capture->device->Release();
    release_com(capture->com_initialized);
    delete capture;
}

extern "C" int32_t eiviz_decklink_playback_open(
    const char* persistent_id_value,
    uint32_t audio_channels,
    eiviz_decklink_playback** playback,
    char* error,
    size_t error_capacity) {
    if (persistent_id_value == nullptr || playback == nullptr || audio_channels == 0) {
        set_error(error, error_capacity, "invalid playback-open arguments");
        return -1;
    }
    *playback = nullptr;
    bool com_initialized = false;
    std::string detail;
    IDeckLink* device =
        find_device(persistent_id_value, &com_initialized, &detail);
    if (device == nullptr) {
        set_error(error, error_capacity, detail);
        return -1;
    }
    IDeckLinkOutput* output = nullptr;
    HRESULT result = device->QueryInterface(
        IID_IDeckLinkOutput, reinterpret_cast<void**>(&output));
    if (result != S_OK || output == nullptr) {
        device->Release();
        release_com(com_initialized);
        set_error(error, error_capacity, "selected device has no DeckLink output");
        return -1;
    }
    if (!supports_playback_mode(output, &detail)) {
        output->Release();
        device->Release();
        release_com(com_initialized);
        set_error(error, error_capacity, detail);
        return -1;
    }
    PlaybackCallback* callback = new PlaybackCallback();
    result = output->SetScheduledFrameCompletionCallback(callback);
    if (result == S_OK) {
        result =
            output->EnableVideoOutput(bmdModeHD1080p5994, bmdVideoOutputFlagDefault);
    }
    if (result == S_OK) {
        result = output->EnableAudioOutput(
            bmdAudioSampleRate48kHz,
            bmdAudioSampleType16bitInteger,
            audio_channels,
            bmdAudioOutputStreamTimestamped);
    }
    if (result != S_OK) {
        output->DisableAudioOutput();
        output->DisableVideoOutput();
        output->SetScheduledFrameCompletionCallback(nullptr);
        callback->Release();
        output->Release();
        device->Release();
        release_com(com_initialized);
        set_error(error, error_capacity, hr_error("enabling DeckLink output", result));
        return -1;
    }
    IDeckLinkStatus* status = nullptr;
    device->QueryInterface(
        IID_IDeckLinkStatus, reinterpret_cast<void**>(&status));
    *playback = new eiviz_decklink_playback{
        device,
        output,
        status,
        callback,
        audio_channels,
        0,
        false,
        com_initialized};
    return 0;
}

extern "C" int32_t eiviz_decklink_playback_schedule_video(
    eiviz_decklink_playback* playback,
    const uint8_t* bgra,
    size_t data_len,
    uint32_t row_bytes,
    int64_t display_time,
    int64_t duration,
    int64_t time_scale,
    char* error,
    size_t error_capacity) {
    const size_t required =
        static_cast<size_t>(row_bytes) * static_cast<size_t>(kHeight);
    if (playback == nullptr || bgra == nullptr ||
        row_bytes < static_cast<uint32_t>(kWidth * 4) || data_len < required ||
        duration <= 0 || time_scale <= 0) {
        set_error(error, error_capacity, "invalid scheduled video frame");
        return -1;
    }
    IDeckLinkMutableVideoFrame* frame = nullptr;
    HRESULT result = playback->output->CreateVideoFrame(
        kWidth,
        kHeight,
        kWidth * 4,
        bmdFormat8BitBGRA,
        bmdFrameFlagDefault,
        &frame);
    if (result != S_OK || frame == nullptr) {
        set_error(error, error_capacity, hr_error("CreateVideoFrame", result));
        return -1;
    }
    void* destination = nullptr;
    result = frame->GetBytes(&destination);
    if (result == S_OK) {
        for (int32_t row = 0; row < kHeight; ++row) {
            std::memcpy(
                static_cast<uint8_t*>(destination) +
                    static_cast<size_t>(row) * kWidth * 4,
                bgra + static_cast<size_t>(row) * row_bytes,
                static_cast<size_t>(kWidth) * 4);
        }
        result = playback->output->ScheduleVideoFrame(
            frame, display_time, duration, time_scale);
    }
    if (result != S_OK) {
        frame->Release();
        set_error(error, error_capacity, hr_error("ScheduleVideoFrame", result));
        return -1;
    }
    // The completion callback releases the frame after the SDK is finished with it.
    playback->scheduled_video.fetch_add(1, std::memory_order_relaxed);
    return 0;
}

extern "C" int32_t eiviz_decklink_playback_schedule_audio(
    eiviz_decklink_playback* playback,
    const int16_t* interleaved_samples,
    uint32_t frame_count,
    int64_t stream_time,
    int64_t time_scale,
    char* error,
    size_t error_capacity) {
    if (playback == nullptr || interleaved_samples == nullptr ||
        frame_count == 0 || time_scale <= 0) {
        set_error(error, error_capacity, "invalid scheduled audio packet");
        return -1;
    }
    uint32_t written = 0;
    const HRESULT result = playback->output->ScheduleAudioSamples(
        const_cast<int16_t*>(interleaved_samples),
        frame_count,
        stream_time,
        time_scale,
        &written);
    if (result != S_OK || written != frame_count) {
        set_error(
            error,
            error_capacity,
            result == S_OK ? "ScheduleAudioSamples accepted a partial packet"
                           : hr_error("ScheduleAudioSamples", result));
        return -1;
    }
    return 0;
}

extern "C" int32_t eiviz_decklink_playback_start(
    eiviz_decklink_playback* playback,
    int64_t start_time,
    int64_t time_scale,
    char* error,
    size_t error_capacity) {
    if (playback == nullptr || time_scale <= 0) {
        set_error(error, error_capacity, "invalid playback-start arguments");
        return -1;
    }
    if (playback->started) {
        return 0;
    }
    const HRESULT result =
        playback->output->StartScheduledPlayback(start_time, time_scale, 1.0);
    if (result != S_OK) {
        set_error(error, error_capacity, hr_error("StartScheduledPlayback", result));
        return -1;
    }
    playback->started = true;
    return 0;
}

extern "C" int32_t eiviz_decklink_playback_get_diagnostics(
    eiviz_decklink_playback* playback,
    eiviz_decklink_playback_diagnostics* diagnostics,
    char* error,
    size_t error_capacity) {
    if (playback == nullptr || diagnostics == nullptr) {
        set_error(error, error_capacity, "invalid diagnostics arguments");
        return -1;
    }
    uint32_t buffered_video = 0;
    uint32_t buffered_audio = 0;
    const HRESULT video_result =
        playback->output->GetBufferedVideoFrameCount(&buffered_video);
    const HRESULT audio_result =
        playback->output->GetBufferedAudioSampleFrameCount(&buffered_audio);
    if (video_result != S_OK || audio_result != S_OK) {
        set_error(error, error_capacity, "failed to query DeckLink buffer depths");
        return -1;
    }
    int32_t reference_locked = -1;
    if (playback->status != nullptr) {
        bool locked = false;
        if (playback->status->GetFlag(
                bmdDeckLinkStatusReferenceSignalLocked, &locked) == S_OK) {
            reference_locked = locked ? 1 : 0;
        }
    }
    diagnostics->scheduled_video =
        playback->scheduled_video.load(std::memory_order_relaxed);
    diagnostics->completed_video = playback->callback->completed();
    diagnostics->late_video = playback->callback->late();
    diagnostics->dropped_video = playback->callback->dropped();
    diagnostics->flushed_video = playback->callback->flushed();
    diagnostics->buffered_video = buffered_video;
    diagnostics->buffered_audio_frames = buffered_audio;
    diagnostics->reference_locked = reference_locked;
    return 0;
}

extern "C" void eiviz_decklink_playback_close(
    eiviz_decklink_playback* playback) {
    if (playback == nullptr) {
        return;
    }
    if (playback->started) {
        BMDTimeValue actual_stop = 0;
        playback->output->StopScheduledPlayback(
            0, &actual_stop, kVideoTimeScale);
    }
    playback->output->FlushBufferedAudioSamples();
    playback->output->DisableAudioOutput();
    playback->output->DisableVideoOutput();
    playback->output->SetScheduledFrameCompletionCallback(nullptr);
    if (playback->status != nullptr) {
        playback->status->Release();
    }
    playback->callback->Release();
    playback->output->Release();
    playback->device->Release();
    release_com(playback->com_initialized);
    delete playback;
}

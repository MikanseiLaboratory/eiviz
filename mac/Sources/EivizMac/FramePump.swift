import AVFoundation
import CoreVideo
import EivizMixer
import Foundation

final class FramePump {
    private var readers: [UInt64: Pump] = [:]

    func startFile(id: UInt64, path: String) {
        stop(id)
        let url = URL(fileURLWithPath: path)
        let asset = AVURLAsset(url: url)
        readers[id] = Pump(id: id, asset: asset)
    }

    func startCapture(id: UInt64, deviceId: String) {
        stop(id)
        readers[id] = Pump(id: id, deviceId: deviceId)
    }

    func stop(_ id: UInt64) {
        readers[id]?.stop()
        readers.removeValue(forKey: id)
        _ = mixer_destroy_source(id)
    }

    func setPlaying(_ id: UInt64, playing: Bool) {
        readers[id]?.playing = playing
    }

    func seek(_ id: UInt64, fraction: Double) {
        readers[id]?.seek(fraction: fraction)
    }

    var activeFileId: UInt64? {
        readers.first { $0.value.isFile }?.key
    }

    func info(_ id: UInt64) -> (position: Double, duration: Double, playing: Bool)? {
        readers[id].map { ($0.position, $0.duration, $0.playing) }
    }
}

private final class Pump: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    let id: UInt64
    let isFile: Bool
    var playing = true
    var position: Double = 0
    var duration: Double = 1
    private var output: AVPlayerItemVideoOutput?
    private var player: AVPlayer?
    private var capture: AVCaptureSession?
    private var timer: Timer?
    private var registered = false

    init(id: UInt64, asset: AVURLAsset) {
        self.id = id
        self.isFile = true
        super.init()
        let item = AVPlayerItem(asset: asset)
        let attrs: [String: Any] = [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA
        ]
        let videoOutput = AVPlayerItemVideoOutput(pixelBufferAttributes: attrs)
        item.add(videoOutput)
        output = videoOutput
        player = AVPlayer(playerItem: item)
        player?.play()
        timer = Timer.scheduledTimer(withTimeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
            self?.pull()
        }
    }

    init(id: UInt64, deviceId: String) {
        self.id = id
        self.isFile = false
        super.init()
        let session = AVCaptureSession()
        if session.canSetSessionPreset(.high) {
            session.sessionPreset = .high
        }
        let device = AVCaptureDevice.DiscoverySession(
            deviceTypes: [.builtInWideAngleCamera, .external],
            mediaType: .video,
            position: .unspecified
        ).devices.first { $0.uniqueID == deviceId } ?? AVCaptureDevice.default(for: .video)
        guard let device, let input = try? AVCaptureDeviceInput(device: device) else { return }
        if session.canAddInput(input) { session.addInput(input) }
        let output = AVCaptureVideoDataOutput()
        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA]
        output.setSampleBufferDelegate(self, queue: DispatchQueue(label: "eiviz.uvc.\(id)"))
        if session.canAddOutput(output) { session.addOutput(output) }
        capture = session
        DispatchQueue.global(qos: .userInitiated).async {
            session.startRunning()
        }
    }

    func stop() {
        timer?.invalidate()
        timer = nil
        player?.pause()
        player = nil
        capture?.stopRunning()
        capture = nil
    }

    func seek(fraction: Double) {
        guard let player, duration > 0 else { return }
        let t = max(0, min(1, fraction)) * duration
        player.seek(to: CMTime(seconds: t, preferredTimescale: 600))
    }

    private func pull() {
        guard playing, let output, let player else { return }
        let time = player.currentTime()
        position = CMTimeGetSeconds(time)
        if let item = player.currentItem {
            let itemDuration = CMTimeGetSeconds(item.duration)
            if itemDuration > 0 {
                duration = itemDuration
            }
        }
        guard output.hasNewPixelBuffer(forItemTime: time),
              let buffer = output.copyPixelBuffer(forItemTime: time, itemTimeForDisplay: nil)
        else { return }
        push(buffer, pts: Int64(position * 10_000_000))
    }

    func captureOutput(
        _ output: AVCaptureOutput,
        didOutput sampleBuffer: CMSampleBuffer,
        from connection: AVCaptureConnection
    ) {
        guard playing, let buffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        push(buffer, pts: Int64(CMTimeGetSeconds(pts) * 10_000_000))
    }

    private func push(_ buffer: CVPixelBuffer, pts: Int64) {
        CVPixelBufferLockBaseAddress(buffer, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
        let width = UInt32(CVPixelBufferGetWidth(buffer))
        let height = UInt32(CVPixelBufferGetHeight(buffer))
        let stride = UInt32(CVPixelBufferGetBytesPerRow(buffer))
        guard let base = CVPixelBufferGetBaseAddress(buffer) else { return }
        if !registered {
            _ = mixer_register_source(id, width, height, EIVIZ_FMT_BGRA)
            registered = true
        }
        _ = mixer_push_frame(id, base.assumingMemoryBound(to: UInt8.self), stride, height, pts)
    }
}

import AVFoundation
import EivizMixer
import Foundation

final class MasterAudioOut {
    private let engine = AVAudioEngine()
    private var source: AVAudioSourceNode?

    func start() {
        stop()
        guard let format = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 2) else { return }
        let node = AVAudioSourceNode(format: format) { _, _, frameCount, ablPointer in
            guard let ablPointer else { return noErr }
            let frames = Int(frameCount)
            var interleaved = [Float](repeating: 0, count: frames * 2)
            let n = interleaved.withUnsafeMutableBufferPointer { ptr in
                mixer_copy_follow_audio(ptr.baseAddress, UInt32(ptr.count))
            }
            let abl = UnsafeMutableAudioBufferListPointer(ablPointer)
            if abl.count == 1, let data = abl[0].mData?.assumingMemoryBound(to: Float.self) {
                let copy = min(Int(n), frames * 2)
                for i in 0..<copy { data[i] = interleaved[i] }
            } else if abl.count >= 2 {
                let left = abl[0].mData?.assumingMemoryBound(to: Float.self)
                let right = abl[1].mData?.assumingMemoryBound(to: Float.self)
                let copy = min(Int(n) / 2, frames)
                for i in 0..<copy {
                    left?[i] = interleaved[i * 2]
                    right?[i] = interleaved[i * 2 + 1]
                }
            }
            return noErr
        }
        source = node
        engine.attach(node)
        engine.connect(node, to: engine.mainMixerNode, format: format)
        engine.mainMixerNode.outputVolume = 1
        do {
            try engine.start()
        } catch {
            NSLog("eiviz audio: \(error.localizedDescription)")
        }
    }

    func stop() {
        engine.stop()
        if let source {
            engine.detach(source)
        }
        source = nil
    }
}

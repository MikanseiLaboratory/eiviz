#!/usr/bin/env swift
// Prints the VideoToolbox encoders and hardware decode flags on this Mac.
// Usage: swift scripts/macos-video-encoders.swift
import CoreMedia
import Foundation
import VideoToolbox

func fourcc(_ code: FourCharCode) -> String {
    let bytes = [UInt8((code >> 24) & 0xff), UInt8((code >> 16) & 0xff), UInt8((code >> 8) & 0xff), UInt8(code & 0xff)]
    return String(bytes: bytes, encoding: .ascii) ?? "?"
}

var list: CFArray?
let status = VTCopyVideoEncoderList(nil, &list)
print("VTCopyVideoEncoderList status=\(status)")
if let entries = list as? [[String: Any]] {
    for entry in entries {
        let codec = (entry[kVTVideoEncoderList_CodecType as String] as? NSNumber)?.uint32Value ?? 0
        let name = entry[kVTVideoEncoderList_CodecName as String] as? String ?? ""
        let id = entry[kVTVideoEncoderList_EncoderID as String] as? String ?? ""
        let hardware = (entry[kVTVideoEncoderList_IsHardwareAccelerated as String] as? NSNumber)?.boolValue ?? false
        print("codec=\(fourcc(codec)) hardware=\(hardware) name=\(name) id=\(id)")
    }
}
for (name, codec) in [
    ("H.264", kCMVideoCodecType_H264),
    ("HEVC", kCMVideoCodecType_HEVC),
    ("AV1", kCMVideoCodecType_AV1),
] {
    print("\(name) hardware decode=\(VTIsHardwareDecodeSupported(codec))")
}

import Darwin
import Foundation
import Metal

@MainActor
enum HostResources {
    private static var lastCpu: Double = 0
    private static var lastStamp = Date.distantPast

    static func hud(renderMs: Float, budgetMs: Float) -> (text: String, warn: String) {
        let cpu = sampleCpu()
        let ram = sampleRam()
        let vram = sampleVram()
        let gpu = budgetMs > 0.1 ? min(100, renderMs / budgetMs * 100) : 0
        let text = String(
            format: "CPU %.0f%%   GPU %.0f%%   RAM %.0f%%   VRAM %.0f%%   Render %.1f ms / %.1f ms",
            cpu, gpu, ram, vram, renderMs, budgetMs
        )
        var hits: [String] = []
        if cpu >= 85 { hits.append(String(format: "CPU %.0f%%", cpu)) }
        if gpu >= 85 { hits.append(String(format: "GPU %.0f%%", gpu)) }
        if ram >= 85 { hits.append(String(format: "RAM %.0f%%", ram)) }
        if vram >= 85 { hits.append(String(format: "VRAM %.0f%%", vram)) }
        if budgetMs > 0 && renderMs >= budgetMs * 0.85 {
            hits.append(String(format: "Render %.1f ms", renderMs))
        }
        return (text, hits.isEmpty ? "" : "High load: " + hits.joined(separator: "  "))
    }

    private static func sampleCpu() -> Float {
        var usage = rusage()
        guard getrusage(RUSAGE_SELF, &usage) == 0 else { return 0 }
        let cpu = Double(usage.ru_utime.tv_sec + usage.ru_stime.tv_sec)
            + Double(usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) / 1_000_000
        let now = Date()
        let elapsed = now.timeIntervalSince(lastStamp)
        defer {
            lastCpu = cpu
            lastStamp = now
        }
        guard lastStamp != .distantPast, elapsed > 0.05 else { return 0 }
        let cores = max(1, Double(ProcessInfo.processInfo.activeProcessorCount))
        return Float(min(100, max(0, (cpu - lastCpu) / elapsed / cores * 100)))
    }

    private static func sampleRam() -> Float {
        var info = mach_task_basic_info()
        var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size / MemoryLayout<natural_t>.size)
        let kr = withUnsafeMutablePointer(to: &info) { ptr in
            ptr.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count)
            }
        }
        guard kr == KERN_SUCCESS else { return 0 }
        let total = Double(ProcessInfo.processInfo.physicalMemory)
        guard total > 0 else { return 0 }
        return Float(Double(info.resident_size) / total * 100)
    }

    private static func sampleVram() -> Float {
        guard let device = MTLCreateSystemDefaultDevice() else { return 0 }
        let cap = max(1, device.recommendedMaxWorkingSetSize)
        return Float(min(100, Double(device.currentAllocatedSize) / Double(cap) * 100))
    }
}

func formatBytes(_ bytes: UInt64) -> String {
    if bytes == 0 { return "—" }
    if bytes < 1024 { return "\(bytes) B" }
    if bytes < 1024 * 1024 { return String(format: "%.1f KB", Double(bytes) / 1024) }
    return String(format: "%.1f MB", Double(bytes) / (1024 * 1024))
}

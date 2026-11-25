import AVFoundation
import CoreMedia
import CoreVideo

// usage: capture <width> <height> <fourcc 420v|yuvs> <fps> <seconds>
let a = CommandLine.arguments
let (W, H, CC, FPS, SECS) = (Int32(a[1])!, Int32(a[2])!, a[3], Double(a[4])!, Double(a[5])!)
func fourcc(_ v: FourCharCode) -> String { String(bytes: [24,16,8,0].map{UInt8((v >> $0) & 0xff)}, encoding: .ascii)! }

final class Sink: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    var n = 0, first = 0.0, last = 0.0, ymin = 255, ymax = 0, pix = "", info = ""
    var deltas: [Double] = []
    var gaps: [String] = []
    var dims = ""
    var wasDark = false
    var hist = [Int](repeating: 0, count: 256)
    func captureOutput(_ o: AVCaptureOutput, didOutput sb: CMSampleBuffer, from c: AVCaptureConnection) {
        let t = CMSampleBufferGetPresentationTimeStamp(sb).seconds
        if n == 0 { first = t } else { deltas.append(t - last) }
        if n > 0 && t - last > 1.5 / FPS { gaps.append(String(format: "  gap at +%.2fs: %.0f ms", t - first, (t - last) * 1000)) }
        last = t; n += 1
        guard let pb = CMSampleBufferGetImageBuffer(sb) else { return }
        let dd = "\(CVPixelBufferGetWidth(pb))x\(CVPixelBufferGetHeight(pb))"; if dd != dims { gaps.append(String(format: "  size %@ at +%.2fs", dd, n == 0 ? 0 : t - first)); dims = dd }
        if n == 1 {
            pix = fourcc(CVPixelBufferGetPixelFormatType(pb))
            let m = CVBufferCopyAttachment(pb, kCVImageBufferYCbCrMatrixKey, nil) as? String ?? "-"
            let p = CVBufferCopyAttachment(pb, kCVImageBufferColorPrimariesKey, nil) as? String ?? "-"
            info = "buffer=\(pix) \(CVPixelBufferGetWidth(pb))x\(CVPixelBufferGetHeight(pb)) matrix=\(m) primaries=\(p)"
        }
        do {  // per-frame black detection
            CVPixelBufferLockBaseAddress(pb, .readOnly)
            let pl = CVPixelBufferIsPlanar(pb)
            let b = (pl ? CVPixelBufferGetBaseAddressOfPlane(pb, 0) : CVPixelBufferGetBaseAddress(pb))!.assumingMemoryBound(to: UInt8.self)
            let r = pl ? CVPixelBufferGetBytesPerRowOfPlane(pb, 0) : CVPixelBufferGetBytesPerRow(pb)
            var sum = 0, cnt = 0
            for y in stride(from: 0, to: CVPixelBufferGetHeight(pb), by: 32) { for x in stride(from: 0, to: CVPixelBufferGetWidth(pb), by: 32) { sum += Int(b[y * r + x * (pl ? 1 : 2)]); cnt += 1 } }
            CVPixelBufferUnlockBaseAddress(pb, .readOnly)
            let dark = sum / max(cnt, 1) < 20
            if dark != wasDark { gaps.append(String(format: "  picture %@ at +%.2fs", dark ? "BLACK" : "back", n <= 1 ? 0 : t - first)); wasDark = dark }
        }
        if n % 10 == 1 {  // sample luma of every 10th frame
            CVPixelBufferLockBaseAddress(pb, .readOnly)
            let planar = CVPixelBufferIsPlanar(pb)
            let base = (planar ? CVPixelBufferGetBaseAddressOfPlane(pb, 0) : CVPixelBufferGetBaseAddress(pb))!.assumingMemoryBound(to: UInt8.self)
            let bpr = planar ? CVPixelBufferGetBytesPerRowOfPlane(pb, 0) : CVPixelBufferGetBytesPerRow(pb)
            let w = CVPixelBufferGetWidth(pb), h = CVPixelBufferGetHeight(pb)
            let step = planar ? 1 : 2  // yuvs = Y0 U Y1 V: luma on even bytes
            for y in stride(from: 0, to: h, by: 4) { for x in stride(from: 0, to: w, by: 4) {
                let v = Int(base[y * bpr + x * step]); hist[v] += 1; ymin = min(ymin, v); ymax = max(ymax, v) } }
            CVPixelBufferUnlockBaseAddress(pb, .readOnly)
        }
    }
}

let dev = AVCaptureDevice.DiscoverySession(deviceTypes: [.external], mediaType: .video, position: .unspecified)
    .devices.first { $0.manufacturer == "MACROSILICON" }!
let fmt = dev.formats.first { f in
    let d = CMVideoFormatDescriptionGetDimensions(f.formatDescription)
    return d.width == W && d.height == H && fourcc(CMFormatDescriptionGetMediaSubType(f.formatDescription)) == CC
        && f.videoSupportedFrameRateRanges.contains { abs($0.maxFrameRate - FPS) < 0.01 }
}!
let session = AVCaptureSession()
session.beginConfiguration()
session.addInput(try! AVCaptureDeviceInput(device: dev))
let out = AVCaptureVideoDataOutput()
out.alwaysDiscardsLateVideoFrames = true
out.videoSettings = [:]  // deliver the device's native format, no conversion
let sink = Sink()
let q = DispatchQueue(label: "cap")
out.setSampleBufferDelegate(sink, queue: q)
session.addOutput(out)
session.commitConfiguration()
try! dev.lockForConfiguration()
dev.activeFormat = fmt
let dur = fmt.videoSupportedFrameRateRanges.first { abs($0.maxFrameRate - FPS) < 0.01 }!.minFrameDuration
dev.activeVideoMinFrameDuration = dur; dev.activeVideoMaxFrameDuration = dur
session.startRunning()
dev.unlockForConfiguration()
let af = dev.activeFormat; let ad = CMVideoFormatDescriptionGetDimensions(af.formatDescription)
let activeDesc = "active=\(fourcc(CMFormatDescriptionGetMediaSubType(af.formatDescription))) \(ad.width)x\(ad.height) minDur=\(dev.activeVideoMinFrameDuration.seconds) preset=\(session.sessionPreset.rawValue)"
Thread.sleep(forTimeInterval: SECS)
session.stopRunning()
q.sync {}
let fps = sink.n > 1 ? Double(sink.n - 1) / (sink.last - sink.first) : 0
let sorted = sink.deltas.sorted()
let p50 = sorted.isEmpty ? 0 : sorted[sorted.count / 2] * 1000, p99 = sorted.isEmpty ? 0 : sorted[min(sorted.count - 1, Int(Double(sorted.count) * 0.99))] * 1000
print(String(format: "%@ %dx%d @%.0f -> %d frames, %.2f fps, interval p50 %.1f ms p99 %.1f ms, Y %d..%d | %@",
             CC, W, H, FPS, sink.n, fps, p50, p99, sink.ymin, sink.ymax, sink.info + " | " + activeDesc))
for g in sink.gaps.prefix(200) { print(g) }
let tot = sink.hist.reduce(0, +)
func pct(_ p: Double) -> Int { var acc = 0; for (i, c) in sink.hist.enumerated() { acc += c; if Double(acc) >= p * Double(tot) { return i } }; return 255 }
print("  luma p0.5=\(pct(0.005)) p5=\(pct(0.05)) p50=\(pct(0.5)) p95=\(pct(0.95)) p99.5=\(pct(0.995))  (share of samples <16: \(String(format: "%.1f%%", 100 * Double(sink.hist[0..<16].reduce(0,+)) / Double(max(tot,1)))), >235: \(String(format: "%.1f%%", 100 * Double(sink.hist[236...].reduce(0,+)) / Double(max(tot,1)))))")

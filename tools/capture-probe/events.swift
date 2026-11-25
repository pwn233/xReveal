import AVFoundation
import CoreMedia
import CoreVideo

// usage: events <width> <height> <fps> <seconds>
// Streams the MACROSILICON card (NV12 420v) and logs, with timestamps since start:
// device connect/disconnect notifications, isInUseByAnotherApplication / isSuspended changes,
// session runtime errors / interruptions, frame gaps, picture size changes and BLACK/back transitions.
// On re-plug it rebuilds the session and reports time from connect to first frame.
let a = CommandLine.arguments
let (W, H, FPS, SECS) = (Int32(a[1])!, Int32(a[2])!, Double(a[3])!, Double(a[4])!)
let t0 = Date()
func log(_ s: String) { print(String(format: "[%7.2fs] ", Date().timeIntervalSince(t0)) + s); fflush(stdout) }
func fourcc(_ v: FourCharCode) -> String { String(bytes: [24,16,8,0].map{UInt8((v >> $0) & 0xff)}, encoding: .ascii)! }

final class Sink: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    var n = 0, last = 0.0, dims = "", wasDark: Bool? = nil
    var firstFrameWaiter: Date? = nil
    func captureOutput(_ o: AVCaptureOutput, didOutput sb: CMSampleBuffer, from c: AVCaptureConnection) {
        let t = CMSampleBufferGetPresentationTimeStamp(sb).seconds
        if let w = firstFrameWaiter { log(String(format: "FIRST FRAME after %.0f ms", Date().timeIntervalSince(w) * 1000)); firstFrameWaiter = nil }
        if n > 0 && t - last > 1.5 / FPS { log(String(format: "gap %.0f ms", (t - last) * 1000)) }
        last = t; n += 1
        if n % Int(FPS * 10) == 0 { log("… \(n) frames") }
        guard let pb = CMSampleBufferGetImageBuffer(sb) else { return }
        let dd = "\(CVPixelBufferGetWidth(pb))x\(CVPixelBufferGetHeight(pb)) \(fourcc(CVPixelBufferGetPixelFormatType(pb)))"
        if dd != dims { log("picture size \(dd)"); dims = dd }
        CVPixelBufferLockBaseAddress(pb, .readOnly)
        let b = CVPixelBufferGetBaseAddressOfPlane(pb, 0)!.assumingMemoryBound(to: UInt8.self)
        let r = CVPixelBufferGetBytesPerRowOfPlane(pb, 0)
        var sum = 0, cnt = 0, mn = 255, mx = 0
        for y in stride(from: 0, to: CVPixelBufferGetHeight(pb), by: 32) { for x in stride(from: 0, to: CVPixelBufferGetWidth(pb), by: 32) {
            let v = Int(b[y * r + x]); sum += v; cnt += 1; mn = min(mn, v); mx = max(mx, v) } }
        CVPixelBufferUnlockBaseAddress(pb, .readOnly)
        let dark = mx - mn < 24
        if dark != wasDark { log((dark ? "picture FLAT (no image)" : "picture has CONTENT") + " (Y mean \(sum / max(cnt, 1)), min \(mn), max \(mx))"); wasDark = dark }
    }
}

var session: AVCaptureSession? = nil
var device: AVCaptureDevice? = nil
var kvo: [NSKeyValueObservation] = []
let sink = Sink()
let q = DispatchQueue(label: "cap")
let nc = NotificationCenter.default

func findCard() -> AVCaptureDevice? {
    AVCaptureDevice.DiscoverySession(deviceTypes: [.external], mediaType: .video, position: .unspecified)
        .devices.first { $0.manufacturer == "MACROSILICON" }
}

func start(_ dev: AVCaptureDevice) {
    device = dev
    log("device \(dev.localizedName) uniqueID=\(dev.uniqueID) inUseByAnotherApp=\(dev.isInUseByAnotherApplication) suspended=\(dev.isSuspended) connected=\(dev.isConnected)")
    kvo = [
        dev.observe(\.isInUseByAnotherApplication, options: [.new]) { d, _ in log("isInUseByAnotherApplication -> \(d.isInUseByAnotherApplication)") },
        dev.observe(\.isSuspended, options: [.new]) { d, _ in log("isSuspended -> \(d.isSuspended)") },
        dev.observe(\.isConnected, options: [.new]) { d, _ in log("isConnected -> \(d.isConnected)") },
    ]
    guard let fmt = dev.formats.first(where: { f in
        let d = CMVideoFormatDescriptionGetDimensions(f.formatDescription)
        return d.width == W && d.height == H && fourcc(CMFormatDescriptionGetMediaSubType(f.formatDescription)) == "420v"
            && f.videoSupportedFrameRateRanges.contains { abs($0.maxFrameRate - FPS) < 0.01 }
    }) else { log("no 420v \(W)x\(H)@\(FPS) format"); return }
    let s = AVCaptureSession()
    s.beginConfiguration()
    do { s.addInput(try AVCaptureDeviceInput(device: dev)) } catch { log("addInput FAILED: \(error)"); return }
    let out = AVCaptureVideoDataOutput()
    out.alwaysDiscardsLateVideoFrames = true
    out.videoSettings = [:]
    out.setSampleBufferDelegate(sink, queue: q)
    s.addOutput(out)
    s.commitConfiguration()
    for name in [AVCaptureSession.runtimeErrorNotification, AVCaptureSession.wasInterruptedNotification,
                 AVCaptureSession.interruptionEndedNotification, AVCaptureSession.didStartRunningNotification,
                 AVCaptureSession.didStopRunningNotification] {
        nc.addObserver(forName: name, object: s, queue: nil) { n in
            let err = (n.userInfo?[AVCaptureSessionErrorKey] as? NSError).map { " error=\($0.domain) \($0.code) \($0.localizedDescription)" } ?? ""
            log("session \(n.name.rawValue)\(err)")
        }
    }
    do { try dev.lockForConfiguration() } catch { log("lockForConfiguration FAILED: \(error)") }
    dev.activeFormat = fmt
    let dur = fmt.videoSupportedFrameRateRanges.first { abs($0.maxFrameRate - FPS) < 0.01 }!.minFrameDuration
    dev.activeVideoMinFrameDuration = dur; dev.activeVideoMaxFrameDuration = dur
    sink.firstFrameWaiter = Date()
    s.startRunning()
    dev.unlockForConfiguration()
    log("session running=\(s.isRunning)")
    session = s
}

nc.addObserver(forName: AVCaptureDevice.wasDisconnectedNotification, object: nil, queue: nil) { n in
    let d = n.object as? AVCaptureDevice
    log("NOTIFY wasDisconnected: \(d?.localizedName ?? "?") \(d?.uniqueID ?? "")")
    if d?.manufacturer == "MACROSILICON" || d?.uniqueID == device?.uniqueID {
        session?.stopRunning(); session = nil; kvo = []; device = nil; sink.dims = ""; sink.wasDark = nil
    }
}
nc.addObserver(forName: AVCaptureDevice.wasConnectedNotification, object: nil, queue: nil) { n in
    let d = n.object as? AVCaptureDevice
    log("NOTIFY wasConnected: \(d?.localizedName ?? "?") \(d?.uniqueID ?? "") manufacturer=\(d?.manufacturer ?? "")")
    if let d, d.manufacturer == "MACROSILICON", d.hasMediaType(.video), session == nil {
        DispatchQueue.main.async { start(d) }
    }
}

guard let dev = findCard() else { log("card not found"); exit(1) }
start(dev)
RunLoop.main.run(until: Date().addingTimeInterval(SECS))
session?.stopRunning()
log("done, \(sink.n) frames total")

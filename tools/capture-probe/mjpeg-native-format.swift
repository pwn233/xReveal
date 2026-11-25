import AVFoundation
import CoreMedia

func fourcc(_ v: FourCharCode) -> String {
    let b = [24, 16, 8, 0].map { UInt8((v >> UInt32($0)) & 0xff) }
    return String(bytes: b, encoding: .ascii) ?? "\(v)"
}

let devs = AVCaptureDevice.DiscoverySession(deviceTypes: [.external], mediaType: .video, position: .unspecified).devices
guard let dev = devs.first(where: { $0.modelID.contains("ProductID_8497") }) ?? devs.first else {
    print("no external device"); exit(1)
}
print("device:", dev.localizedName, dev.modelID)
var subs = Set<String>()
for f in dev.formats {
    let d = CMVideoFormatDescriptionGetDimensions(f.formatDescription)
    let st = fourcc(CMFormatDescriptionGetMediaSubType(f.formatDescription))
    let rates = f.videoSupportedFrameRateRanges.map { String(format: "%.0f", $0.maxFrameRate) }.joined(separator: ",")
    subs.insert(st)
    if d.width == 1920 { print("  format \(st) \(d.width)x\(d.height) @ \(rates)") }
}
print("distinct format subtypes:", subs.sorted())

final class Sink: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    var n = 0
    let label: String
    init(_ l: String) { label = l }
    func captureOutput(_ o: AVCaptureOutput, didOutput sb: CMSampleBuffer, from c: AVCaptureConnection) {
        n += 1
        if n == 5 {
            let fd = CMSampleBufferGetFormatDescription(sb)!
            let st = fourcc(CMFormatDescriptionGetMediaSubType(fd))
            let hasImg = CMSampleBufferGetImageBuffer(sb) != nil
            let dataLen = CMSampleBufferGetDataBuffer(sb).map { CMBlockBufferGetDataLength($0) } ?? -1
            print("[\(label)] sample subtype=\(st) hasImageBuffer=\(hasImg) dataBufferLen=\(dataLen)")
        }
    }
}

func run(_ label: String, _ settings: [String: Any]?) {
    let s = AVCaptureSession()
    let input = try! AVCaptureDeviceInput(device: dev)
    s.addInput(input)
    let out = AVCaptureVideoDataOutput()
    print("[\(label)] availableVideoCodecTypes:", out.availableVideoCodecTypes.map { $0.rawValue })
    print("[\(label)] availableVideoPixelFormatTypes:", out.availableVideoPixelFormatTypes.map { fourcc($0) })
    out.videoSettings = settings
    print("[\(label)] effective videoSettings:", out.videoSettings ?? [:])
    let sink = Sink(label)
    let q = DispatchQueue(label: "q")
    out.setSampleBufferDelegate(sink, queue: q)
    s.addOutput(out)
    s.startRunning()
    Thread.sleep(forTimeInterval: 2.0)
    s.stopRunning()
    print("[\(label)] frames in 2s: \(sink.n)")
}

let sem = DispatchSemaphore(value: 0)
AVCaptureDevice.requestAccess(for: .video) { ok in print("access:", ok); sem.signal() }
sem.wait()
run("empty-dict(device-native)", [:])
run("nil(default)", nil)

import AVFoundation
import CoreMedia

func fourcc(_ v: FourCharCode) -> String {
    let b = [24, 16, 8, 0].map { UInt8((v >> $0) & 0xff) }
    return String(bytes: b, encoding: .ascii) ?? String(v)
}
let session = AVCaptureDevice.DiscoverySession(
    deviceTypes: [.external], mediaType: nil, position: .unspecified)
for d in session.devices {
    print("== \(d.localizedName) | uniqueID=\(d.uniqueID) | modelID=\(d.modelID) | manufacturer=\(d.manufacturer)")
    print("   hasMedia video=\(d.hasMediaType(.video)) audio=\(d.hasMediaType(.audio)) muxed=\(d.hasMediaType(.muxed))")
    for f in d.formats {
        let desc = f.formatDescription
        let dims = CMVideoFormatDescriptionGetDimensions(desc)
        let sub = fourcc(CMFormatDescriptionGetMediaSubType(desc))
        let rates = f.videoSupportedFrameRateRanges.map { r in
            r.minFrameRate == r.maxFrameRate ? String(format: "%.2f", r.maxFrameRate)
                : String(format: "%.2f-%.2f", r.minFrameRate, r.maxFrameRate) }
        let ext = CMFormatDescriptionGetExtensions(desc) as? [String: Any] ?? [:]
        let matrix = ext[kCVImageBufferYCbCrMatrixKey as String] ?? "-"
        let full = ext["FullRangeVideo"] ?? "-"
        print("   \(sub) \(dims.width)x\(dims.height) @ \(rates.joined(separator: ",")) matrix=\(matrix) fullRange=\(full)")
    }
}

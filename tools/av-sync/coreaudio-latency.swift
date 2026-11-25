// Throwaway probe: print what Core Audio reports as the fixed latency of every audio device
// (device latency + safety offset + first stream's latency) and the current IO buffer size.
// Build and run: swiftc -O -o coreaudio-latency coreaudio-latency.swift && ./coreaudio-latency
import CoreAudio
func get<T>(_ id: AudioObjectID, _ sel: AudioObjectPropertySelector, _ scope: AudioObjectPropertyScope, _ v: inout T) -> Bool {
  var a = AudioObjectPropertyAddress(mSelector: sel, mScope: scope, mElement: kAudioObjectPropertyElementMain)
  var sz = UInt32(MemoryLayout<T>.size)
  return AudioObjectGetPropertyData(id, &a, 0, nil, &sz, &v) == noErr
}
var a = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDevices, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
var sz: UInt32 = 0
AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &a, 0, nil, &sz)
var ids = [AudioObjectID](repeating: 0, count: Int(sz)/4)
AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &a, 0, nil, &sz, &ids)
for id in ids {
  var name: CFString = "" as CFString
  _ = get(id, kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal, &name)
  var rate: Float64 = 0; _ = get(id, kAudioDevicePropertyNominalSampleRate, kAudioObjectPropertyScopeGlobal, &rate)
  var tt: UInt32 = 0; _ = get(id, kAudioDevicePropertyTransportType, kAudioObjectPropertyScopeGlobal, &tt)
  for (label, scope) in [("in", kAudioObjectPropertyScopeInput), ("out", kAudioObjectPropertyScopeOutput)] {
    var sa = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyStreams, mScope: scope, mElement: kAudioObjectPropertyElementMain)
    var ssz: UInt32 = 0
    AudioObjectGetPropertyDataSize(id, &sa, 0, nil, &ssz)
    if ssz == 0 { continue }
    var streams = [AudioStreamID](repeating: 0, count: Int(ssz)/4)
    AudioObjectGetPropertyData(id, &sa, 0, nil, &ssz, &streams)
    var lat: UInt32 = 0; _ = get(id, kAudioDevicePropertyLatency, scope, &lat)
    var so: UInt32 = 0; _ = get(id, kAudioDevicePropertySafetyOffset, scope, &so)
    var buf: UInt32 = 0; _ = get(id, kAudioDevicePropertyBufferFrameSize, scope, &buf)
    var sl: UInt32 = 0; _ = get(streams[0], kAudioStreamPropertyLatency, kAudioObjectPropertyScopeGlobal, &sl)
    var rng = AudioValueRange(mMinimum: 0, mMaximum: 0); _ = get(id, kAudioDevicePropertyBufferFrameSizeRange, scope, &rng)
    print("  bufferRange=\(rng.mMinimum)-\(rng.mMaximum)")
    let tot = lat + so + sl + buf
    let fourcc = String(bytes: [UInt8(tt>>24 & 0xff),UInt8(tt>>16 & 0xff),UInt8(tt>>8 & 0xff),UInt8(tt & 0xff)], encoding: .ascii) ?? "?"
    print("\(name) [\(fourcc)] \(label) rate=\(rate) devLat=\(lat) safety=\(so) streamLat=\(sl) buffer=\(buf) => \(tot) frames = \(String(format: "%.2f", Double(tot)/rate*1000)) ms")
  }
}

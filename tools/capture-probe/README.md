# capture-probe (macOS)

Throwaway probes used to measure the Reference card for the ticket "Record what the reference card (Onten) delivers on Mac and Windows". Not part of xReveal.

```sh
swiftc -O -o formats formats.swift && ./formats            # list every AVFoundation format of external capture devices
swiftc -O -o capture capture.swift
./capture 1280 720 420v 60 3                              # width height fourcc fps seconds
```

`capture` picks the exact format, holds the device configuration lock across `startRunning()` so the session preset can't override it (macOS has no `inputPriority` preset), and reports real fps, frame-interval p50/p99, frame gaps, black-picture transitions, luma percentiles and the buffer's colour tags.

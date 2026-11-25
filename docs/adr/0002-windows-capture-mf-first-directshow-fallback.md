# Windows capture: Media Foundation first, DirectShow as fallback

xReveal supports capture cards as widely as possible: every driver-free UVC/UAC card is promised, and driver-based cards are supported on a best-effort basis. On Windows many driver-based cards (PCIe and older models) are visible only through DirectShow, sometimes with their audio on a DirectShow pin. So xReveal captures through **Media Foundation** for every card it lists, because it is the modern API and its sample timestamps share the clock used for presentation (which makes the latency budget measurable), and falls back to **DirectShow** only for cards Media Foundation doesn't list. On macOS no second path is needed: cards with a vendor camera extension appear in AVFoundation like any other camera.

## Considered Options

- **DirectShow only** (what OBS does): one path, with the vendor dialog and audio pins built in, and the same measured latency (~18 ms on one card). Rejected because it is a legacy API and its timestamps are harder to line up with presentation time for in-app latency checks.
- **UVC/UAC only**: least code, but leaves out driver-based cards, against the goal of working with capture cards in general.

## Consequences

- Two Windows capture backends to maintain behind one Source interface. DirectShow-only cards may also need a DirectShow audio path, because `cpal` can't reach a DirectShow audio pin.

Decided in [Which capture cards does v1 support: UVC/UAC only, or driver-based cards too?](https://github.com/pwn233/xReveal/issues/12).

// Tracer-bullet video shader: NV12 → RGB, BT.709, drawn into the Fit rectangle (set as the viewport).
// The full shader (Colour range/space rules, Picture adjustments, all Fill modes, bicubic) replaces
// this in the "Full video shader" slice (docs/spec/v1.md §3.4).
#include <metal_stdlib>
using namespace metal;

struct VertexOut {
    float4 position [[position]];
    float2 uv;
};

// One triangle that covers the viewport; no vertex buffer needed.
vertex VertexOut fullscreen_vertex(uint vid [[vertex_id]]) {
    float2 p = float2((vid << 1) & 2, vid & 2);
    VertexOut out;
    out.position = float4(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = float2(p.x, 1.0 - p.y);
    return out;
}

struct Params {
    // 1 when the buffer is video range (`420v`): macOS squeezed the card's full-range levels into
    // 16-235 / 16-240, so expand them back (docs/spec/v1.md §2.6).
    uint video_range;
};

fragment float4 nv12_fragment(VertexOut in [[stage_in]],
                              texture2d<float> luma [[texture(0)]],
                              texture2d<float> chroma [[texture(1)]],
                              constant Params& params [[buffer(0)]]) {
    constexpr sampler s(filter::linear, address::clamp_to_edge);
    float y = luma.sample(s, in.uv).r;
    float2 c = chroma.sample(s, in.uv).rg;
    if (params.video_range != 0) {
        y = (y - 16.0 / 255.0) * (255.0 / 219.0);
        c = (c - 128.0 / 255.0) * (255.0 / 224.0);
    } else {
        c = c - 128.0 / 255.0;
    }
    // BT.709
    float3 rgb = float3(y + 1.5748 * c.y,
                        y - 0.1873 * c.x - 0.4681 * c.y,
                        y + 1.8556 * c.x);
    return float4(saturate(rgb), 1.0);
}

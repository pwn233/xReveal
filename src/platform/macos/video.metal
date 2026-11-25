// The video shader (docs/spec/v1.md §3.4). One pass: map the screen pixel back onto the Source
// through the Fill mode, sample the YUV planes with Catmull-Rom bicubic filtering, then apply the
// colour matrix (OS level fix-up, Colour range, Colour space, Picture adjustments) and gamma.
//
// Every number comes from `video::picture::plan` on the CPU, and this file mirrors
// `video::picture::reference::shade` line for line; `video.hlsl` is its twin. GPU tests compare
// both against that reference.
#include <metal_stdlib>
using namespace metal;

struct Params {
    float4 colour[3]; // RGB = row · (Y, Cb, Cr, 1)
    float4 mapping;   // xy offset, zw scale: source UV = offset + UV × scale
    float4 smart;     // k, t, p, enabled
    float4 sizes;     // luma size (texels), chroma size (texels)
    float4 misc;      // 1 / gamma, layout (0 NV12, 1 YUY2, 2 Y-Cb-Cr 4:4:4)
};

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

static float2 source_uv(constant Params& p, float2 uv) {
    float u = p.mapping.x + uv.x * p.mapping.z;
    float v = p.mapping.y + uv.y * p.mapping.w;
    if (p.smart.w > 0.5) {
        float k = p.smart.x, t = p.smart.y, e = p.smart.z;
        float s = uv.x * 2.0 - 1.0;
        float a = abs(s);
        float g = (1.0 - t) * a + t * (k * a - (k - 1.0) * pow(a, e));
        u = 0.5 + 0.5 * (s < 0.0 ? -1.0 : 1.0) * g;
    }
    return float2(u, v);
}

static float4 weights(float t) {
    float t2 = t * t, t3 = t2 * t;
    return 0.5 * float4(-t3 + 2.0 * t2 - t, 3.0 * t3 - 5.0 * t2 + 2.0, -3.0 * t3 + 4.0 * t2 + t, t3 - t2);
}

static float fetch_luma(texture2d<float> first, int x, int y, int layout) {
    if (layout == 2) {
        return first.read(uint2(x * 3, y)).r;
    }
    if (layout == 1) {
        float4 texel = first.read(uint2(x / 2, y));
        return (x & 1) ? texel.b : texel.r;
    }
    return first.read(uint2(x, y)).r;
}

static float2 fetch_chroma(texture2d<float> first, texture2d<float> second, int x, int y, int layout) {
    if (layout == 2) {
        return float2(first.read(uint2(x * 3 + 1, y)).r, first.read(uint2(x * 3 + 2, y)).r);
    }
    if (layout == 1) {
        return first.read(uint2(x, y)).ga;
    }
    return second.read(uint2(x, y)).rg;
}

fragment float4 video_fragment(VertexOut in [[stage_in]],
                               texture2d<float> first [[texture(0)]],
                               texture2d<float> second [[texture(1)]],
                               constant Params& p [[buffer(0)]]) {
    int layout = int(p.misc.y + 0.5);
    float2 src = source_uv(p, in.uv);

    // Luma: 4×4 Catmull-Rom taps, clamped at the edges.
    float2 lp = src * p.sizes.xy - 0.5;
    float2 li = floor(lp);
    float4 lwx = weights(lp.x - li.x), lwy = weights(lp.y - li.y);
    int2 lmax = int2(p.sizes.xy) - 1;
    float luma = 0.0;
    for (int j = 0; j < 4; j++) {
        int y = clamp(int(li.y) + j - 1, 0, lmax.y);
        float row = 0.0;
        for (int i = 0; i < 4; i++) {
            int x = clamp(int(li.x) + i - 1, 0, lmax.x);
            row += lwx[i] * fetch_luma(first, x, y, layout);
        }
        luma += lwy[j] * row;
    }

    // Chroma: the same on the (smaller) chroma grid.
    float2 cp = src * p.sizes.zw - 0.5;
    float2 ci = floor(cp);
    float4 cwx = weights(cp.x - ci.x), cwy = weights(cp.y - ci.y);
    int2 cmax = int2(p.sizes.zw) - 1;
    float2 chroma = 0.0;
    for (int j = 0; j < 4; j++) {
        int y = clamp(int(ci.y) + j - 1, 0, cmax.y);
        float2 row = 0.0;
        for (int i = 0; i < 4; i++) {
            int x = clamp(int(ci.x) + i - 1, 0, cmax.x);
            row += cwx[i] * fetch_chroma(first, second, x, y, layout);
        }
        chroma += cwy[j] * row;
    }

    float4 ycc = float4(luma, chroma, 1.0);
    float3 rgb = saturate(float3(dot(p.colour[0], ycc), dot(p.colour[1], ycc), dot(p.colour[2], ycc)));
    return float4(pow(rgb, float3(p.misc.x)), 1.0);
}

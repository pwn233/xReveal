// The video shader (docs/spec/v1.md §3.4). One pass: map the screen pixel back onto the Source
// through the Fill mode, sample the YUV planes with Catmull-Rom bicubic filtering, then apply the
// colour matrix (OS level fix-up, Colour range, Colour space, Picture adjustments) and gamma.
//
// Every number comes from `video::picture::plan` on the CPU, and this file mirrors
// `video::picture::reference::shade` line for line; `video.metal` is its twin. GPU tests compare
// both against that reference.

cbuffer Params : register(b0) {
    float4 colour[3]; // RGB = row · (Y, Cb, Cr, 1)
    float4 mapping;   // xy offset, zw scale: source UV = offset + UV × scale
    float4 smart;     // k, t, p, enabled
    float4 sizes;     // luma size (texels), chroma size (texels)
    float4 misc;      // 1 / gamma, layout (0 NV12, 1 YUY2, 2 Y-Cb-Cr 4:4:4)
};

Texture2D<float4> first : register(t0);  // NV12: luma (R8). YUY2: packed (Y0, Cb, Y1, Cr). 4:4:4: R8, 3 texels per pixel.
Texture2D<float4> second : register(t1); // NV12: CbCr (R8G8). YUY2: unused.
SamplerState bilinear : register(s0);    // Linear filtering, clamped at the edges.

struct VertexOut {
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
};

// One triangle that covers the viewport; no vertex buffer needed.
VertexOut vs_main(uint id : SV_VertexID) {
    float2 p = float2((id << 1) & 2, id & 2);
    VertexOut o;
    o.position = float4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = float2(p.x, 1.0 - p.y);
    return o;
}

float2 source_uv(float2 uv) {
    float u = mapping.x + uv.x * mapping.z;
    float v = mapping.y + uv.y * mapping.w;
    if (smart.w > 0.5) {
        float k = smart.x, t = smart.y, e = smart.z;
        float s = uv.x * 2.0 - 1.0;
        float a = abs(s);
        float g = (1.0 - t) * a + t * (k * a - (k - 1.0) * pow(a, e));
        u = 0.5 + 0.5 * (s < 0.0 ? -1.0 : 1.0) * g;
    }
    return float2(u, v);
}

float4 weights(float t) {
    float t2 = t * t, t3 = t2 * t;
    return 0.5 * float4(-t3 + 2.0 * t2 - t, 3.0 * t3 - 5.0 * t2 + 2.0, -3.0 * t3 + 4.0 * t2 + t, t3 - t2);
}

float fetch_luma(int x, int y, int layout) {
    if (layout == 2) {
        return first.Load(int3(x * 3, y, 0)).r;
    }
    if (layout == 1) {
        float4 texel = first.Load(int3(x / 2, y, 0));
        return (x & 1) ? texel.b : texel.r;
    }
    return first.Load(int3(x, y, 0)).r;
}

float2 fetch_chroma(int x, int y, int layout) {
    if (layout == 2) {
        return float2(first.Load(int3(x * 3 + 1, y, 0)).r, first.Load(int3(x * 3 + 2, y, 0)).r);
    }
    if (layout == 1) {
        return first.Load(int3(x, y, 0)).ga;
    }
    return second.Load(int3(x, y, 0)).rg;
}

// Catmull-Rom over 4×4 texels in 3×3 filtered fetches (#56): the middle two taps of each axis
// have positive weights, so one bilinear fetch between them gives both, weighted. Exactly the
// 16-tap result, give or take the hardware's filtering precision. Mirrors `catmull_rom_9` in
// `video.metal`.
float4 catmull_rom_9(Texture2D<float4> plane, float2 position, float2 size) {
    float2 lp = position * size - 0.5;
    float2 li = floor(lp);
    float4 wx = weights(lp.x - li.x), wy = weights(lp.y - li.y);
    float3 sx = float3(wx.x, wx.y + wx.z, wx.w), sy = float3(wy.x, wy.y + wy.z, wy.w);
    // Texel centres (+0.5) of the outer taps, and the weighted point between the middle two.
    float3 px = li.x + 0.5 + float3(-1.0, wx.z / sx.y, 2.0);
    float3 py = li.y + 0.5 + float3(-1.0, wy.z / sy.y, 2.0);
    float4 sum = 0.0;
    [unroll] for (int j = 0; j < 3; j++) {
        float4 row = 0.0;
        [unroll] for (int k = 0; k < 3; k++) {
            row += sx[k] * plane.SampleLevel(bilinear, float2(px[k], py[j]) / size, 0);
        }
        sum += sy[j] * row;
    }
    return sum;
}

float4 ps_main(VertexOut i) : SV_Target {
    int layout = int(misc.y + 0.5);
    float2 src = source_uv(i.uv);

    // NV12: each plane is its own texture, so the hardware can filter.
    if (layout == 0) {
        float4 ycc = float4(catmull_rom_9(first, src, sizes.xy).r, catmull_rom_9(second, src, sizes.zw).rg, 1.0);
        float3 rgb = saturate(float3(dot(colour[0], ycc), dot(colour[1], ycc), dot(colour[2], ycc)));
        return float4(pow(rgb, misc.xxx), 1.0);
    }

    // Luma: 4×4 Catmull-Rom taps, clamped at the edges.
    float2 lp = src * sizes.xy - 0.5;
    float2 li = floor(lp);
    float4 lwx = weights(lp.x - li.x), lwy = weights(lp.y - li.y);
    int2 lmax = int2(sizes.xy) - 1;
    float luma = 0.0;
    [unroll] for (int j = 0; j < 4; j++) {
        int y = clamp(int(li.y) + j - 1, 0, lmax.y);
        float row = 0.0;
        [unroll] for (int k = 0; k < 4; k++) {
            int x = clamp(int(li.x) + k - 1, 0, lmax.x);
            row += lwx[k] * fetch_luma(x, y, layout);
        }
        luma += lwy[j] * row;
    }

    // Chroma: the same on the (smaller) chroma grid.
    float2 cp = src * sizes.zw - 0.5;
    float2 ci = floor(cp);
    float4 cwx = weights(cp.x - ci.x), cwy = weights(cp.y - ci.y);
    int2 cmax = int2(sizes.zw) - 1;
    float2 chroma = 0.0;
    [unroll] for (int m = 0; m < 4; m++) {
        int y = clamp(int(ci.y) + m - 1, 0, cmax.y);
        float2 row = 0.0;
        [unroll] for (int n = 0; n < 4; n++) {
            int x = clamp(int(ci.x) + n - 1, 0, cmax.x);
            row += cwx[n] * fetch_chroma(x, y, layout);
        }
        chroma += cwy[m] * row;
    }

    float4 ycc = float4(luma, chroma, 1.0);
    float3 rgb = saturate(float3(dot(colour[0], ycc), dot(colour[1], ycc), dot(colour[2], ycc)));
    return float4(pow(rgb, misc.xxx), 1.0);
}

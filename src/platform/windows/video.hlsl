// Tracer-bullet video shader: NV12 or YUY2 → RGB, BT.709, drawn into the Fit rectangle (the
// viewport). The full shader replaces this in the "Full video shader" slice (docs/spec/v1.md §3.4).

cbuffer Params : register(b0) {
    uint video_range;   // 1: expand 16-235 / 16-240 levels
    uint picture_width; // in pixels (YUY2 needs it to pick Y0 or Y1)
    uint picture_height;
    uint pad;
};

Texture2D<float> luma : register(t0);     // NV12 Y plane
Texture2D<float2> chroma : register(t1);  // NV12 CbCr plane
Texture2D<float4> packed : register(t2);  // YUY2 as RGBA = (Y0, Cb, Y1, Cr), half width
SamplerState linear_clamp : register(s0);

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

float4 to_rgb(float y, float2 c) {
    if (video_range != 0) {
        y = (y - 16.0 / 255.0) * (255.0 / 219.0);
        c = (c - 128.0 / 255.0) * (255.0 / 224.0);
    } else {
        c = c - 128.0 / 255.0;
    }
    // BT.709
    float3 rgb = float3(y + 1.5748 * c.y, y - 0.1873 * c.x - 0.4681 * c.y, y + 1.8556 * c.x);
    return float4(saturate(rgb), 1.0);
}

float4 ps_nv12(VertexOut i) : SV_Target {
    return to_rgb(luma.Sample(linear_clamp, i.uv), chroma.Sample(linear_clamp, i.uv));
}

float4 ps_yuy2(VertexOut i) : SV_Target {
    int2 pixel = int2(min(i.uv * float2(picture_width, picture_height), float2(picture_width - 1, picture_height - 1)));
    float4 texel = packed.Load(int3(pixel.x / 2, pixel.y, 0));
    float y = (pixel.x & 1) ? texel.b : texel.r;
    return to_rgb(y, texel.ga);
}

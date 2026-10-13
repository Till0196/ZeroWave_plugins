// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

// 仕上げ：最後のレベルの大きさのフローと重みを元の大きさへ伸ばし（PyTorch の bilinear、
// align_corners=False と同じ）、前後をフローでワープして（境目は端の値）、シグモイドの重みで混ぜる。
// 画像は RGB の三つのプレーンを縦に並べた一成分（プレーン c の y 行目は c × 高さ + y 行目）。
// 実行時に newLibraryWithSource でコンパイルする（interpolator.rs が文字列で持つ）。

#include <metal_stdlib>
using namespace metal;

struct Params {
    uint width;
    uint height;
    uint low_w;
    uint low_h;
    float inv_k;
};

static float up(texture2d<float, access::read> t, uint plane, constant Params &p, uint2 at) {
    float sx = max((float(at.x) + 0.5f) * p.inv_k - 0.5f, 0.0f);
    float sy = max((float(at.y) + 0.5f) * p.inv_k - 0.5f, 0.0f);
    uint x0 = min(uint(sx), p.low_w - 1);
    uint y0 = min(uint(sy), p.low_h - 1);
    uint x1 = min(x0 + 1, p.low_w - 1);
    uint y1 = min(y0 + 1, p.low_h - 1);
    float ax = sx - float(x0);
    float ay = sy - float(y0);
    uint base = plane * p.low_h;
    float a = t.read(uint2(x0, base + y0)).r;
    float b = t.read(uint2(x1, base + y0)).r;
    float c = t.read(uint2(x0, base + y1)).r;
    float d = t.read(uint2(x1, base + y1)).r;
    return mix(mix(a, b, ax), mix(c, d, ax), ay);
}

static float pick(texture2d<float, access::read> t, uint plane, float x, float y, constant Params &p) {
    x = clamp(x, 0.0f, float(p.width - 1));
    y = clamp(y, 0.0f, float(p.height - 1));
    uint x0 = uint(x);
    uint y0 = uint(y);
    uint x1 = min(x0 + 1, p.width - 1);
    uint y1 = min(y0 + 1, p.height - 1);
    float ax = x - float(x0);
    float ay = y - float(y0);
    uint base = plane * p.height;
    float a = t.read(uint2(x0, base + y0)).r;
    float b = t.read(uint2(x1, base + y0)).r;
    float c = t.read(uint2(x0, base + y1)).r;
    float d = t.read(uint2(x1, base + y1)).r;
    return mix(mix(a, b, ax), mix(c, d, ax), ay);
}

kernel void finish(texture2d<float, access::read> previous [[texture(0)]],
                   texture2d<float, access::read> current [[texture(1)]],
                   texture2d<float, access::write> middle [[texture(2)]],
                   texture2d<float, access::read> flow [[texture(3)]],
                   texture2d<float, access::read> mask [[texture(4)]],
                   constant Params &p [[buffer(0)]],
                   uint2 at [[thread_position_in_grid]]) {
    if (at.x >= p.width || at.y >= p.height) return;
    float x = float(at.x);
    float y = float(at.y);
    float x0 = x + up(flow, 0, p, at);
    float y0 = y + up(flow, 1, p, at);
    float x1 = x + up(flow, 2, p, at);
    float y1 = y + up(flow, 3, p, at);
    float m = 1.0f / (1.0f + exp(-up(mask, 0, p, at)));
    for (uint c = 0; c < 3; c++) {
        float v = pick(previous, c, x0, y0, p) * m + pick(current, c, x1, y1, p) * (1.0f - m);
        middle.write(float4(v), uint2(at.x, c * p.height + at.y));
    }
}

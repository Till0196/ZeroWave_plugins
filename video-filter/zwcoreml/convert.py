# Copyright 2026 Till0196
# SPDX-License-Identifier: Apache-2.0
"""RIFE（Practical-RIFE の IFNet、vs-rife の重み）を CoreML の mlpackage にする。

  python convert.py --vsrife <vs-rife のフォルダー> --model 4.6 --pkl <flownet_v4.6.pkl> \
      --out <フォルダー> [--scale 0.5] [--size 1440x1080 1920x1080 ...]

大きさごとに一つ（ANE は形の決まった物しか載らない）、`<フォルダー>/rife-v<版>-s<scale>-<幅>x<高さ>.mlpackage`。
入力は前（img0）・今（img1）が [1,3,高さ,幅] の f16、位相（t）が [1,1,1,1] の f16。
出力は最後のレベルの大きさのフロー（flow、[1,4,h,w]、元の大きさの画素）と重み（mask、[1,1,h,w]、シグモイドの前）。
元の大きさへ伸ばして前後をワープし、混ぜるのは zwcoreml（Metal）がする（元の大きさの演算をモデルに
入れると ANE に載らず、CPU との行き来で遅くなる）。
"""

import argparse
import importlib
import math
import os
import sys
import types

import coremltools as ct
import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F

# 版ごとの（IFNet のファイル、Head の有無、縦横の倍数）。vs-rife の __init__.py に合わせる。
MODELS = {
    "4.6": ("IFNet_HDv3_v4_6", False, 32),
    "4.22.lite": ("IFNet_HDv3_v4_22_lite", True, 32),
    "4.25.lite": ("IFNet_HDv3_v4_25_lite", True, 128),
}


def load_ifnet(vsrife, model):
    # vs-rife の __init__.py は vapoursynth を読むので、ラッパーを作って中の IFNet だけ読む。
    package = types.ModuleType("vsr")
    package.__path__ = [os.path.join(vsrife, "vsrife")]
    sys.modules["vsr"] = package
    return importlib.import_module(f"vsr.{MODELS[model][0]}")


def padded_size(model, scale, width, height):
    """RIFE が要る倍数へ切り上げた大きさ（vs-rife と同じ）。"""
    modulo = MODELS[model][2]
    step = max(modulo, int(modulo / scale))
    return math.ceil(width / step) * step, math.ceil(height / step) * step


def load_net(module, model, pkl, scale):
    """重みを読んだ IFNet と Head（無いバージョンは None）。"""
    state = torch.load(pkl, map_location="cpu")
    state = {k.replace("module.", ""): v for k, v in state.items() if "module." in k}
    net = module.IFNet(scale, False)
    net.load_state_dict(state, strict=False)
    head = None
    if MODELS[model][1]:
        head = module.Head()
        head.load_state_dict({k.replace("encode.", ""): v for k, v in state.items() if "encode." in k})
        head.eval()
    return net.eval(), head


def resize(x, size):
    return F.interpolate(x, size=size, mode="bilinear", align_corners=False)


def warp(img, flow, size):
    """フロー（img の画素）でワープする。境目は端の値（vs-rife の warplayer と同じ考え）。size は (高さ, 幅)。"""
    h, w = size
    xs = torch.linspace(-1.0, 1.0, w).view(1, 1, 1, w).expand(1, 1, h, w)
    ys = torch.linspace(-1.0, 1.0, h).view(1, 1, h, 1).expand(1, 1, h, w)
    grid = torch.cat([xs + flow[:, 0:1] * (2.0 / (w - 1)), ys + flow[:, 1:2] * (2.0 / (h - 1))], 1)
    return F.grid_sample(img, grid.permute(0, 2, 3, 1), mode="bilinear", padding_mode="border", align_corners=True)


class LowRes(nn.Module):
    """IFNet のフローのレベルだけを、レベルごとの小さな大きさのまま計算する。

    元の IFNet はレベルごとにフローと重みを元の大きさへ戻し、元の大きさで前後をワープしてから次のレベルの
    大きさへ縮める。ここでは前後と Head の特徴をレベルの大きさへ縮めてからワープし、フローと重みもレベルの
    大きさの間で直に伸ばす（元の大きさの演算が Head のほかに無いので ANE に載る）。
    """

    def __init__(self, net, head, width, height, padded_w, padded_h):
        super().__init__()
        names = ["block0", "block1", "block2", "block3", "block4"]
        self.blocks = nn.ModuleList([getattr(net, n) for n in names if hasattr(net, n)])
        self.ks = list(net.scale_list)
        self.head = head
        self.size = (width, height, padded_w, padded_h)

    def forward(self, img0, img1, t):
        width, height, padded_w, padded_h = self.size
        pad = (0, padded_w - width, 0, padded_h - height)
        x0 = F.pad(img0, pad).clamp(0.0, 1.0)
        x1 = F.pad(img1, pad).clamp(0.0, 1.0)
        lite = self.head is not None
        if lite:
            f0, f1 = self.head(x0), self.head(x1)
        flow = mask = feat = None
        for block, k in zip(self.blocks, self.ks):
            size = (int(padded_h / k), int(padded_w / k))
            i0, i1 = resize(x0, size), resize(x1, size)
            tk = t.expand(1, 1, size[0], size[1])
            if flow is None:
                parts = [i0, i1] + ([resize(f0, size), resize(f1, size)] if lite else []) + [tk]
            else:
                flow, mask = resize(flow, size), resize(mask, size)
                fk = flow / k
                parts = [warp(i0, fk[:, :2], size), warp(i1, fk[:, 2:4], size)]
                if lite:
                    parts += [warp(resize(f0, size), fk[:, :2], size), warp(resize(f1, size), fk[:, 2:4], size)]
                parts += [tk, mask] + ([resize(feat, size)] if lite else []) + [fk]
            tmp = block.lastconv(block.convblock(block.conv0(torch.cat(parts, 1))))
            if flow is None:
                flow, mask = tmp[:, :4] * k, tmp[:, 4:5]
            else:
                flow = flow + tmp[:, :4] * k
                # lite は重みを足さずに置き換える。
                mask = tmp[:, 4:5] if lite else mask + tmp[:, 4:5]
            if lite:
                feat = tmp[:, 5:]
        return flow, mask


def warp_pixels(img, flow):
    """元の大きさで、画素の位置にフローを足してワープする（境目は本当の画像の端）。"""
    _, _, h, w = img.shape
    xs = torch.arange(w, dtype=torch.float32).view(1, 1, 1, w)
    ys = torch.arange(h, dtype=torch.float32).view(1, 1, h, 1)
    gx = (xs + flow[:, 0:1]).clamp(0, w - 1) * (2.0 / (w - 1)) - 1
    gy = (ys + flow[:, 1:2]).clamp(0, h - 1) * (2.0 / (h - 1)) - 1
    grid = torch.cat([gx, gy], 1).permute(0, 2, 3, 1)
    return F.grid_sample(img, grid, mode="bilinear", padding_mode="border", align_corners=True)


def finish(img0, img1, flow, mask, padded_w, padded_h):
    """zwcoreml の Metal と同じ仕上げ（確かめ用）：伸ばして前後をワープし、シグモイドの重みで混ぜる。"""
    _, _, h, w = img0.shape
    up = lambda z: resize(z, (padded_h, padded_w))[:, :, :h, :w]
    flow, mask = up(flow), torch.sigmoid(up(mask))
    return warp_pixels(img0, flow[:, :2]) * mask + warp_pixels(img1, flow[:, 2:4]) * (1 - mask)


def build(args, module, width, height):
    padded_w, padded_h = padded_size(args.model, args.scale, width, height)
    net, head = load_net(module, args.model, args.pkl, args.scale)
    low = LowRes(net, head, width, height, padded_w, padded_h).eval()

    shape = (1, 3, height, width)
    example = (torch.rand(shape), torch.rand(shape), torch.full((1, 1, 1, 1), 0.5))
    with torch.no_grad():
        traced = torch.jit.trace(low, example)
    mlmodel = ct.convert(
        traced,
        inputs=[
            ct.TensorType(name="img0", shape=shape, dtype=np.float16),
            ct.TensorType(name="img1", shape=shape, dtype=np.float16),
            ct.TensorType(name="t", shape=(1, 1, 1, 1), dtype=np.float16),
        ],
        outputs=[ct.TensorType(name="flow", dtype=np.float16), ct.TensorType(name="mask", dtype=np.float16)],
        convert_to="mlprogram",
        compute_precision=ct.precision.FLOAT16,
        minimum_deployment_target=ct.target.macOS13,
    )
    mlmodel.short_description = f"RIFE v{args.model}（scale {args.scale:g}、{width}x{height}）"
    mlmodel.license = "MIT（Practical-RIFE、vs-rife）"
    # zwcoreml がフローを伸ばす倍率を知るため。
    mlmodel.user_defined_metadata["padded_width"] = str(padded_w)
    mlmodel.user_defined_metadata["padded_height"] = str(padded_h)
    path = os.path.join(args.out, f"rife-v{args.model}-s{args.scale:g}-{width}x{height}.mlpackage")
    mlmodel.save(path)
    print(f"保存した：{path}（パディング後 {padded_w}x{padded_h}）")


def main():
    parser = argparse.ArgumentParser(description="RIFE を CoreML にする")
    parser.add_argument("--vsrife", required=True, help="vs-rife を git clone したフォルダー")
    parser.add_argument("--model", required=True, choices=sorted(MODELS))
    parser.add_argument("--pkl", help="vs-rife の重み（無ければ <vsrife>/vsrife/models/flownet_v<版>.pkl）")
    parser.add_argument("--out", required=True)
    parser.add_argument("--scale", type=float, default=0.5, choices=[0.25, 0.5, 1.0],
                        help="フローを求める大きさ（小さいほど軽い）")
    parser.add_argument("--size", nargs="+", default=["1440x1080", "1920x1080"])
    args = parser.parse_args()
    if args.pkl is None:
        args.pkl = os.path.join(args.vsrife, "vsrife", "models", f"flownet_v{args.model}.pkl")
    os.makedirs(args.out, exist_ok=True)
    module = load_ifnet(args.vsrife, args.model)
    for size in args.size:
        width, height = map(int, size.lower().split("x"))
        build(args, module, width, height)


if __name__ == "__main__":
    main()

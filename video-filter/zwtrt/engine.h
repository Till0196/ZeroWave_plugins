// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

// RIFE の TensorRT のエンジンを ONNX から作る・フォルダーから読む。

#pragma once

#include <NvInfer.h>

#include <cstddef>
#include <memory>
#include <span>
#include <string>
#include <vector>

namespace zwtrt {

// RIFE の入力のプレーンの数（vs-mlrt の形：前の RGB、今の RGB、時刻、−1〜1 のグリッド 2 プレーン、グリッドの倍率 2 プレーン）。
inline constexpr int kRifePlanes = 11;
// RIFE は縦横を 32 の倍数で受ける。パディングした所は 0。
inline constexpr int kRifeAlignment = 32;

struct Extent {
    int width = 0;
    int height = 0;

    size_t area() const { return static_cast<size_t>(width) * static_cast<size_t>(height); }
};

// 縦横を 32 の倍数へ切り上げる。
Extent padded(Extent extent);

// フォルダーの `<ONNX の名前>-<幅>x<高さ>.engine`（大きさはパディングした物）。
std::string engine_path(const std::string &directory, const std::string &onnx, Extent padded_extent);

struct BuildOptions {
    int planes = kRifePlanes;
    bool fp16 = true;
};

// ONNX からエンジンを作り、直列にした物を返す。進み具合は `@status` で通知する。
std::vector<char> build_engine(nvinfer1::ILogger &logger, const std::string &onnx, Extent padded_extent,
                               BuildOptions options = {});

std::vector<char> read_file(const std::string &path);
bool write_file(const std::string &path, std::span<const char> bytes);

// 推論できるようにしたエンジン（入力の形まで決めた物）。
class RifeEngine {
public:
    // フォルダーのエンジンを読む。無いか、別の GPU・TensorRT で作った物なら作り直して保存する。
    static RifeEngine prepare(nvinfer1::ILogger &logger, const std::string &onnx, const std::string &path,
                              Extent padded_extent);

    nvinfer1::IExecutionContext &context() const { return *context_; }
    const std::string &input_name() const { return input_name_; }
    const std::string &output_name() const { return output_name_; }
    // 出力の要素の数。
    size_t output_count() const;

private:
    // 読めないか、形が合わなければ false。
    bool load(nvinfer1::ILogger &logger, std::span<const char> blob, Extent padded_extent);

    // runtime は engine より、engine は context より長く生きる（メンバーの並びが破棄の順を決める）。
    std::unique_ptr<nvinfer1::IRuntime> runtime_;
    std::unique_ptr<nvinfer1::ICudaEngine> engine_;
    std::unique_ptr<nvinfer1::IExecutionContext> context_;
    std::string input_name_;
    std::string output_name_;
};

} // namespace zwtrt

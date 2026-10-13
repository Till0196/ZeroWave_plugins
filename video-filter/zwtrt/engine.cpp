// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

#include "engine.h"

#include "failure.h"
#include "report.h"

#include <NvOnnxParser.h>

#include <algorithm>
#include <format>
#include <fstream>
#include <iterator>

namespace zwtrt {

namespace {

// エンジンを作る進み具合（TensorRT の IProgressMonitor）。
// 入れ子の段階の完了した割合を、外の段階の一歩の中へ重ねて数える。
class BuildProgress final : public nvinfer1::IProgressMonitor {
public:
    void phaseStart(const char *name, const char *, int32_t steps) noexcept override {
        stack_.push_back({name ? name : "", std::max(steps, 1), 0});
        report();
    }

    bool stepComplete(const char *name, int32_t step) noexcept override {
        const std::string_view key = name ? name : "";
        for (auto &phase : stack_) {
            if (phase.name == key) phase.done = step + 1;
        }
        report();
        return true;
    }

    void phaseFinish(const char *name) noexcept override {
        const std::string_view key = name ? name : "";
        const auto found = std::find_if(stack_.rbegin(), stack_.rend(), [&](const Phase &phase) { return phase.name == key; });
        if (found != stack_.rend()) stack_.erase(std::prev(found.base()), stack_.end());
    }

private:
    struct Phase {
        std::string name;
        int32_t steps;
        int32_t done;
    };

    // 完了するまで 99 % で止める（完了したことは読み込みと応答で分かる）。
    static constexpr int kMaxPercent = 99;

    void report() {
        double at = 0.0;
        double span = 1.0;
        for (const auto &phase : stack_) {
            at += span * std::min(phase.done, phase.steps) / phase.steps;
            span /= phase.steps;
        }
        // 戻らないように、百分の一ずつ進んだときだけ。
        const int percent = std::min(static_cast<int>(at * 100), kMaxPercent);
        if (percent <= said_) return;
        said_ = percent;
        report_status(percent / 100.0, kBuildingStatus);
    }

    std::vector<Phase> stack_;
    int said_ = -1;
};

std::string stem_of(const std::string &path) {
    const size_t slash = path.find_last_of("/\\");
    const std::string name = slash == std::string::npos ? path : path.substr(slash + 1);
    const size_t dot = name.find_last_of('.');
    return dot == std::string::npos ? name : name.substr(0, dot);
}

int round_up(int value, int multiple) { return (value + multiple - 1) / multiple * multiple; }

} // namespace

Extent padded(Extent extent) {
    return {round_up(extent.width, kRifeAlignment), round_up(extent.height, kRifeAlignment)};
}

std::string engine_path(const std::string &directory, const std::string &onnx, Extent padded_extent) {
    return std::format("{}/{}-{}x{}.engine", directory, stem_of(onnx), padded_extent.width, padded_extent.height);
}

std::vector<char> build_engine(nvinfer1::ILogger &logger, const std::string &onnx, Extent padded_extent,
                               BuildOptions options) {
    log_line(std::format("エンジンを作る（{}x{}。初めの一回だけ、数分かかる）", padded_extent.width, padded_extent.height));
    std::unique_ptr<nvinfer1::IBuilder> builder(nvinfer1::createInferBuilder(logger));
    std::unique_ptr<nvinfer1::INetworkDefinition> network(builder->createNetworkV2(0));
    std::unique_ptr<nvonnxparser::IParser> parser(nvonnxparser::createParser(*network, logger));
    if (!parser->parseFromFile(onnx.c_str(), static_cast<int>(nvinfer1::ILogger::Severity::kWARNING))) {
        throw Failure("ONNX を読めない：" + onnx);
    }
    std::unique_ptr<nvinfer1::IBuilderConfig> config(builder->createBuilderConfig());
    BuildProgress progress;
    config->setProgressMonitor(&progress);
    report_status(0.0, kBuildingStatus);
    if (options.fp16) config->setFlag(nvinfer1::BuilderFlag::kFP16);

    // 大きさは一つに決める（最小・最適・最大を同じに）。プロファイルは builder が持つ。
    auto *profile = builder->createOptimizationProfile();
    const char *input = network->getInput(0)->getName();
    const nvinfer1::Dims4 dims(1, options.planes, padded_extent.height, padded_extent.width);
    for (auto selector : {nvinfer1::OptProfileSelector::kMIN, nvinfer1::OptProfileSelector::kOPT,
                          nvinfer1::OptProfileSelector::kMAX}) {
        profile->setDimensions(input, selector, dims);
    }
    config->addOptimizationProfile(profile);

    std::unique_ptr<nvinfer1::IHostMemory> plan(builder->buildSerializedNetwork(*network, *config));
    if (!plan) throw Failure("エンジンを作れない");
    const auto *data = static_cast<const char *>(plan->data());
    return std::vector<char>(data, data + plan->size());
}

std::vector<char> read_file(const std::string &path) {
    std::ifstream in(path, std::ios::binary);
    if (!in) return {};
    return std::vector<char>(std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>());
}

bool write_file(const std::string &path, std::span<const char> bytes) {
    std::ofstream out(path, std::ios::binary);
    out.write(bytes.data(), static_cast<std::streamsize>(bytes.size()));
    return static_cast<bool>(out);
}

RifeEngine RifeEngine::prepare(nvinfer1::ILogger &logger, const std::string &onnx, const std::string &path,
                               Extent padded_extent) {
    // 一度目はフォルダーの物を、合わなければ二度目に作り直した物を読む。
    constexpr int kAttempts = 2;
    RifeEngine engine;
    for (int attempt = 0; attempt < kAttempts; attempt++) {
        std::vector<char> blob = attempt == 0 ? read_file(path) : std::vector<char>();
        if (!blob.empty()) {
            report_status(std::nullopt, kLoadingStatus);
        } else {
            blob = build_engine(logger, onnx, padded_extent);
            if (!write_file(path, blob)) log_line("エンジンを保存できない：" + path);
        }
        if (engine.load(logger, blob, padded_extent)) return engine;
        log_line("エンジンが合わない（別の GPU・TensorRT・大きさ）。作り直す：" + path);
    }
    throw Failure("エンジンを読めない");
}

bool RifeEngine::load(nvinfer1::ILogger &logger, std::span<const char> blob, Extent padded_extent) {
    context_.reset();
    engine_.reset();
    runtime_.reset(nvinfer1::createInferRuntime(logger));
    engine_.reset(runtime_->deserializeCudaEngine(blob.data(), blob.size()));
    if (!engine_) return false;
    context_.reset(engine_->createExecutionContext());
    if (!context_) return false;
    for (int i = 0; i < engine_->getNbIOTensors(); i++) {
        const char *name = engine_->getIOTensorName(i);
        (engine_->getTensorIOMode(name) == nvinfer1::TensorIOMode::kINPUT ? input_name_ : output_name_) = name;
    }
    return context_->setInputShape(input_name_.c_str(),
                                   nvinfer1::Dims4(1, kRifePlanes, padded_extent.height, padded_extent.width));
}

size_t RifeEngine::output_count() const {
    const auto shape = context_->getTensorShape(output_name_.c_str());
    size_t count = 1;
    for (int i = 0; i < shape.nbDims; i++) count *= static_cast<size_t>(shape.d[i]);
    return count;
}

} // namespace zwtrt

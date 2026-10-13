// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

#pragma once

#include <NvInfer.h>

#include <optional>
#include <string_view>

namespace zwtrt {

// 標準エラーへ一行。本体がログに書く。
void log_line(std::string_view line);

// 準備の進み具合を本体へ（`@status <0〜1 か -> <テキスト>`、形は process.rs の先頭）。
// 割合が分からなければ nullopt。
void report_status(std::optional<double> progress, std::string_view said);

inline constexpr std::string_view kBuildingStatus = "RIFE のエンジン";
inline constexpr std::string_view kLoadingStatus = "RIFE のエンジンの読み込み";

// TensorRT のログのうち、警告より重い物だけを標準エラーへ。
class TrtLogger final : public nvinfer1::ILogger {
public:
    void log(Severity severity, const char *message) noexcept override;
};

} // namespace zwtrt

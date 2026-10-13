// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

#include "report.h"

#include <cstdio>
#include <format>
#include <string>

namespace zwtrt {

void log_line(std::string_view line) {
    // 一行を一度に書く（本体は行ごとに読む）。
    std::string text(line);
    text += '\n';
    std::fwrite(text.data(), 1, text.size(), stderr);
    std::fflush(stderr);
}

void report_status(std::optional<double> progress, std::string_view said) {
    if (progress) {
        log_line(std::format("@status {:.3f} {}", *progress, said));
    } else {
        log_line(std::format("@status - {}", said));
    }
}

void TrtLogger::log(Severity severity, const char *message) noexcept {
    if (severity > Severity::kWARNING) return;
    try {
        log_line(std::format("TensorRT（{}）{}", static_cast<int>(severity), message ? message : ""));
    } catch (...) {
        // ログを書けなくても推論は止めない。
    }
}

} // namespace zwtrt

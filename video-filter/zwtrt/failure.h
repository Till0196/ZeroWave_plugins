// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

#pragma once

#include <stdexcept>

namespace zwtrt {

// 本体へ返す理由（UTF-8）を持つ失敗。応答のループが受け止め、状態 1 の応答にする。
class Failure : public std::runtime_error {
public:
    using std::runtime_error::runtime_error;
};

} // namespace zwtrt

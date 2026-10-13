// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

#pragma once

// Windows だけ（d3d12 と shm のハンドル）。

#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>

#include <memory>

namespace zwtrt {

struct HandleCloser {
    void operator()(HANDLE handle) const noexcept {
        if (handle != nullptr && handle != INVALID_HANDLE_VALUE) CloseHandle(handle);
    }
};

// 手放すと閉じる Win32 のハンドル。
using UniqueHandle = std::unique_ptr<void, HandleCloser>;

} // namespace zwtrt

// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

// CUDA のリソース（ストリーム・イベント・メモリ）と、本体の共有バッファ・共有フェンスの取り込み
// （d3d12 の方式は NT ハンドル、vulkan の方式は OPAQUE_FD の fd）。

#pragma once

#include "protocol.h"

#include <cuda_runtime_api.h>

#include <cstddef>
#include <span>
#include <cstdint>
#include <memory>
#include <string_view>
#include <type_traits>

namespace zwtrt::cuda {

// 失敗なら CUDA のエラーの説明をログに書き、reason で Failure を投げる。
void check(cudaError_t result, std::string_view reason);

struct StreamDestroyer {
    void operator()(cudaStream_t stream) const noexcept { cudaStreamDestroy(stream); }
};
struct EventDestroyer {
    void operator()(cudaEvent_t event) const noexcept { cudaEventDestroy(event); }
};
struct DeviceFree {
    void operator()(void *pointer) const noexcept { cudaFree(pointer); }
};
struct ExternalMemoryDestroyer {
    void operator()(cudaExternalMemory_t memory) const noexcept { cudaDestroyExternalMemory(memory); }
};
struct ExternalSemaphoreDestroyer {
    void operator()(cudaExternalSemaphore_t semaphore) const noexcept { cudaDestroyExternalSemaphore(semaphore); }
};

using Stream = std::unique_ptr<std::remove_pointer_t<cudaStream_t>, StreamDestroyer>;
using Event = std::unique_ptr<std::remove_pointer_t<cudaEvent_t>, EventDestroyer>;
using DeviceFloats = std::unique_ptr<float, DeviceFree>;

Stream make_stream();
Event make_event();
DeviceFloats allocate_floats(size_t count);

// 本体のレンダリングする GPU と同じ CUDA の GPU を選ぶ。見つかれば真（d3d12 は LUID 8 バイト、
// vulkan は deviceUUID 16 バイト）。
bool select_device(protocol::Way way, std::span<const std::byte> id);
// どれでもよいとき（shm の方式）：0 番。
void select_any_device();

// 取り込んだ共有バッファ。CUDA から見たアドレスは、取り込みを破棄する前に返す。
class SharedBuffer {
public:
#ifdef _WIN32
    // handle は取り込んだあと閉じてよい（CUDA が自分で参照を持つ）。
    static SharedBuffer import_d3d12(HANDLE handle, uint64_t allocation_size, size_t mapped_bytes);
#else
    // fd は取り込めたら CUDA の物になる（失敗なら閉じる）。
    static SharedBuffer import_fd(int fd, uint64_t allocation_size, size_t mapped_bytes);
#endif

    float *data() const { return mapped_.get(); }

private:
    std::unique_ptr<std::remove_pointer_t<cudaExternalMemory_t>, ExternalMemoryDestroyer> memory_;
    DeviceFloats mapped_;
};

// 取り込んだ共有フェンス。
class SharedFence {
public:
#ifdef _WIN32
    static SharedFence import_d3d12(HANDLE handle);
#else
    static SharedFence import_timeline_fd(int fd);
#endif

    void wait(uint64_t value, cudaStream_t stream, std::string_view reason) const;
    void signal(uint64_t value, cudaStream_t stream, std::string_view reason) const;

private:
    std::unique_ptr<std::remove_pointer_t<cudaExternalSemaphore_t>, ExternalSemaphoreDestroyer> semaphore_;
};

// プレーンの並び：base から pitch 要素ごとに行、plane_stride 要素ごとにプレーン。
struct Planes {
    float *base;
    size_t pitch;
    size_t plane_stride;
};

// メインメモリを CUDA から速くコピーできるよう固定する（固定できなければそのまま。手放すと外す）。
class Pinned {
public:
    Pinned(void *data, size_t size);
    ~Pinned();
    Pinned(const Pinned &) = delete;
    Pinned &operator=(const Pinned &) = delete;

private:
    void *data_ = nullptr;
};

// RGB の 3 プレーンを、左上から width×height だけコピーする（パディングした所には触らない）。
// どちらも GPU のメモリでもメインメモリでもよい（cudaMemcpyDefault）。
void copy_rgb(Planes to, Planes from, size_t width, size_t height, cudaStream_t stream, std::string_view reason);

} // namespace zwtrt::cuda

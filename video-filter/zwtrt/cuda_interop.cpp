// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

#include "cuda_interop.h"

#include "failure.h"
#include "report.h"

#include <cstring>
#include <format>
#include <string>

#ifndef _WIN32
#include <unistd.h>
#endif

namespace zwtrt::cuda {

namespace {

constexpr size_t kRgbPlanes = 3;

} // namespace

void check(cudaError_t result, std::string_view reason) {
    if (result == cudaSuccess) return;
    log_line(std::format("{}（{}）", reason, cudaGetErrorName(result)));
    throw Failure(std::string(reason));
}

Stream make_stream() {
    cudaStream_t stream = nullptr;
    check(cudaStreamCreateWithFlags(&stream, cudaStreamNonBlocking), "CUDA のストリームを作れない");
    return Stream(stream);
}

Event make_event() {
    cudaEvent_t event = nullptr;
    check(cudaEventCreate(&event), "CUDA のイベントを作れない");
    return Event(event);
}

DeviceFloats allocate_floats(size_t count) {
    void *pointer = nullptr;
    check(cudaMalloc(&pointer, count * sizeof(float)), "CUDA のメモリが足りない");
    return DeviceFloats(static_cast<float *>(pointer));
}

bool select_device(protocol::Way way, std::span<const std::byte> id) {
    int count = 0;
    if (cudaGetDeviceCount(&count) != cudaSuccess || count == 0) throw Failure("CUDA の GPU が無い");
    for (int device = 0; device < count; device++) {
        cudaDeviceProp properties{};
        if (cudaGetDeviceProperties(&properties, device) != cudaSuccess) continue;
        const bool same = way == protocol::Way::D3d12
                              ? id.size() >= sizeof(properties.luid) &&
                                    std::memcmp(properties.luid, id.data(), sizeof(properties.luid)) == 0
                              : id.size() >= sizeof(properties.uuid.bytes) &&
                                    std::memcmp(properties.uuid.bytes, id.data(), sizeof(properties.uuid.bytes)) == 0;
        if (!same) continue;
        check(cudaSetDevice(device), "CUDA の GPU を選べない");
        log_line(std::format("GPU {} {}", device, properties.name));
        return true;
    }
    return false;
}

void select_any_device() {
    cudaDeviceProp properties{};
    check(cudaGetDeviceProperties(&properties, 0), "CUDA の GPU が無い");
    check(cudaSetDevice(0), "CUDA の GPU を選べない");
    log_line(std::format("GPU 0 {}（メインメモリを通す）", properties.name));
}

namespace {

void map_buffer(cudaExternalMemoryHandleDesc &desc, size_t mapped_bytes,
                        std::unique_ptr<std::remove_pointer_t<cudaExternalMemory_t>, ExternalMemoryDestroyer> &memory,
                        DeviceFloats &mapped) {
    cudaExternalMemory_t raw = nullptr;
    check(cudaImportExternalMemory(&raw, &desc), "共有バッファを CUDA に取り込めない");
    memory.reset(raw);
    cudaExternalMemoryBufferDesc mapping{};
    mapping.size = mapped_bytes;
    void *pointer = nullptr;
    check(cudaExternalMemoryGetMappedBuffer(&pointer, raw, &mapping), "取り込んだバッファのアドレスが取れない");
    mapped.reset(static_cast<float *>(pointer));
}

} // namespace

#ifdef _WIN32

SharedBuffer SharedBuffer::import_d3d12(HANDLE handle, uint64_t allocation_size, size_t mapped_bytes) {
    SharedBuffer buffer;
    cudaExternalMemoryHandleDesc desc{};
    desc.type = cudaExternalMemoryHandleTypeD3D12Resource;
    desc.handle.win32.handle = handle;
    desc.size = allocation_size;
    desc.flags = cudaExternalMemoryDedicated;
    map_buffer(desc, mapped_bytes, buffer.memory_, buffer.mapped_);
    return buffer;
}

SharedFence SharedFence::import_d3d12(HANDLE handle) {
    SharedFence fence;
    cudaExternalSemaphoreHandleDesc desc{};
    desc.type = cudaExternalSemaphoreHandleTypeD3D12Fence;
    desc.handle.win32.handle = handle;
    cudaExternalSemaphore_t semaphore = nullptr;
    check(cudaImportExternalSemaphore(&semaphore, &desc), "共有フェンスを CUDA に取り込めない");
    fence.semaphore_.reset(semaphore);
    return fence;
}

#else

SharedBuffer SharedBuffer::import_fd(int fd, uint64_t allocation_size, size_t mapped_bytes) {
    SharedBuffer buffer;
    cudaExternalMemoryHandleDesc desc{};
    desc.type = cudaExternalMemoryHandleTypeOpaqueFd;
    desc.handle.fd = fd;
    desc.size = allocation_size;
    desc.flags = cudaExternalMemoryDedicated;
    try {
        map_buffer(desc, mapped_bytes, buffer.memory_, buffer.mapped_);
    } catch (...) {
        if (!buffer.memory_) close(fd);
        throw;
    }
    return buffer;
}

SharedFence SharedFence::import_timeline_fd(int fd) {
    SharedFence fence;
    cudaExternalSemaphoreHandleDesc desc{};
    desc.type = cudaExternalSemaphoreHandleTypeTimelineSemaphoreFd;
    desc.handle.fd = fd;
    cudaExternalSemaphore_t semaphore = nullptr;
    if (cudaImportExternalSemaphore(&semaphore, &desc) != cudaSuccess) {
        close(fd);
        throw Failure("タイムラインセマフォを CUDA に取り込めない");
    }
    fence.semaphore_.reset(semaphore);
    return fence;
}

#endif

Pinned::Pinned(void *data, size_t size) {
    if (cudaHostRegister(data, size, cudaHostRegisterDefault) == cudaSuccess) {
        data_ = data;
    } else {
        cudaGetLastError();
        log_line("メインメモリを CUDA に固定できない（遅いままコピーする）");
    }
}

Pinned::~Pinned() {
    if (data_ != nullptr) cudaHostUnregister(data_);
}

void SharedFence::wait(uint64_t value, cudaStream_t stream, std::string_view reason) const {
    cudaExternalSemaphoreWaitParams params{};
    params.params.fence.value = value;
    auto *semaphore = semaphore_.get();
    check(cudaWaitExternalSemaphoresAsync(&semaphore, &params, 1, stream), reason);
}

void SharedFence::signal(uint64_t value, cudaStream_t stream, std::string_view reason) const {
    cudaExternalSemaphoreSignalParams params{};
    params.params.fence.value = value;
    auto *semaphore = semaphore_.get();
    check(cudaSignalExternalSemaphoresAsync(&semaphore, &params, 1, stream), reason);
}

void copy_rgb(Planes to, Planes from, size_t width, size_t height, cudaStream_t stream, std::string_view reason) {
    for (size_t channel = 0; channel < kRgbPlanes; channel++) {
        check(cudaMemcpy2DAsync(to.base + channel * to.plane_stride, to.pitch * sizeof(float),
                                from.base + channel * from.plane_stride, from.pitch * sizeof(float), width * sizeof(float),
                                height, cudaMemcpyDefault, stream),
              reason);
    }
}

} // namespace zwtrt::cuda

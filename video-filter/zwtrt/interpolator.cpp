// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

#include "interpolator.h"

#include "failure.h"
#include "report.h"

#include <algorithm>
#include <format>

#ifdef _WIN32
#include "win32.h"
#endif

namespace zwtrt {

namespace {

// これより小さい画像は補間しない。
constexpr uint32_t kMinimumSide = 32;
constexpr size_t kRgb = 3;

const char *way_name(protocol::Way way) {
    switch (way) {
    case protocol::Way::D3d12: return "d3d12";
    case protocol::Way::Vulkan: return "vulkan";
    case protocol::Way::Metal: return "metal";
    case protocol::Way::Shm: return "shm";
    }
    return "?";
}

// 本体の優先順に、使える方式の最初の物（GPU もここで選ぶ）。
protocol::Way choose_way(const protocol::OpenRequest &request) {
    if (request.width < kMinimumSide || request.height < kMinimumSide) throw Failure("画像が小さすぎる");
    if (!request.offers_format(protocol::Format::F32)) throw Failure("f32 の画像を出さない本体");
    for (const auto &offer : request.offers) {
        const auto way = static_cast<protocol::Way>(offer.way);
        switch (way) {
#ifdef _WIN32
        case protocol::Way::D3d12:
#else
        case protocol::Way::Vulkan:
#endif
            // レンダリングする GPU が CUDA に見えれば GPU のまま受け渡す。
            if (cuda::select_device(way, offer.id)) return way;
            log_line(std::format("{} の方式：レンダリングする GPU が CUDA に見えない", way_name(way)));
            break;
        case protocol::Way::Shm:
            cuda::select_any_device();
            return way;
        default:
            break;
        }
    }
    throw Failure("使える受け渡しの方式が無い（レンダリングする GPU が CUDA に見えない）");
}

RifeEngine open_engine(nvinfer1::ILogger &logger, const std::string &onnx, const std::string &engines,
                       const protocol::OpenRequest &request) {
    const Extent size = padded({static_cast<int>(request.width), static_cast<int>(request.height)});
    return RifeEngine::prepare(logger, onnx, engine_path(engines, onnx, size), size);
}

} // namespace

Interpolator::Interpolator(nvinfer1::ILogger &logger, const std::string &onnx, const std::string &engines,
                           const protocol::OpenRequest &request)
    : way_(choose_way(request)),
      jobs_(std::clamp<uint32_t>(request.jobs, 1, kMostJobs)),
      picture_{static_cast<int>(request.width), static_cast<int>(request.height)},
      padded_(padded(picture_)),
      engine_(open_engine(logger, onnx, engines, request)) {
    log_line(std::format("受け渡しの方式は {}、同時に {} つ", way_name(way_), jobs_));
    cuda::check(cudaGetDevice(&device_), "CUDA の GPU が分からない");
    const size_t area = padded_.area();
    const size_t outputs = engine_.output_count();
    if (outputs < kRgb * area) throw Failure("出力の形が RIFE と違う");
    input_ = cuda::allocate_floats(kRifePlanes * area);
    output_ = cuda::allocate_floats(outputs);
    cuda::check(cudaMemset(input_.get(), 0, kRifePlanes * area * sizeof(float)), "CUDA のメモリを 0 にできない");
    auto &context = engine_.context();
    if (!context.setTensorAddress(engine_.input_name().c_str(), input_.get()) ||
        !context.setTensorAddress(engine_.output_name().c_str(), output_.get())) {
        throw Failure("入出力のアドレスを渡せない");
    }
    staging_.resize(area);
    fill_grid();
    stream_ = cuda::make_stream();
    inputs_.resize(protocol::inputs_of(jobs_));
    middles_.resize(jobs_);
    timings_.resize(jobs_);
    for (auto &timing : timings_) {
        timing.begin = cuda::make_event();
        timing.end = cuda::make_event();
    }
}

Interpolator::~Interpolator() {
    if (replier_.joinable()) {
        {
            const std::lock_guard lock(mutex_);
            stopping_ = true;
        }
        wake_.notify_all();
        replier_.join();
    }
    // ストリームに入れたジョブが完了してから、取り込んだ物とエンジンを壊す。
    if (stream_) cudaStreamSynchronize(stream_.get());
}

float *Interpolator::Picture::data() const {
    if (mapping) return reinterpret_cast<float *>(mapping->data());
    return shared->data();
}

void Interpolator::upload_plane(InputPlane plane, std::string_view reason) {
    const size_t area = padded_.area();
    cuda::check(cudaMemcpy(input_.get() + plane * area, staging_.data(), area * sizeof(float), cudaMemcpyHostToDevice),
                reason);
}

void Interpolator::fill_grid() {
    // グリッドと倍率の 4 プレーンは大きさで決まるので一度だけ。
    const auto width = static_cast<size_t>(padded_.width);
    const auto height = static_cast<size_t>(padded_.height);
    const float step_x = 2.0f / static_cast<float>(padded_.width - 1);
    const float step_y = 2.0f / static_cast<float>(padded_.height - 1);
    const auto fill = [&](InputPlane plane, auto value_at) {
        for (size_t y = 0; y < height; y++) {
            for (size_t x = 0; x < width; x++) staging_[y * width + x] = value_at(x, y);
        }
        upload_plane(plane, "グリッドのプレーンを書けない");
    };
    fill(kGridX, [&](size_t x, size_t) { return static_cast<float>(x) * step_x - 1.0f; });
    fill(kGridY, [&](size_t, size_t y) { return static_cast<float>(y) * step_y - 1.0f; });
    fill(kScaleX, [&](size_t, size_t) { return step_x; });
    fill(kScaleY, [&](size_t, size_t) { return step_y; });
}

void Interpolator::share(const protocol::ShareRequest &request, protocol::Channel &channel) {
    // OS のハンドルは先に所有権を取る（失敗してもリークしない）。
#ifdef _WIN32
    const auto handle = reinterpret_cast<HANDLE>(request.handle);
    if (handle == nullptr) throw Failure("SHARE にハンドルが無い");
#else
    const int fd = channel.take_fd();
#endif
    (void)channel;
    switch (request.what) {
    case protocol::What::Input:
    case protocol::What::Middle: {
        auto &slots = request.what == protocol::What::Input ? inputs_ : middles_;
        if (request.index >= slots.size()) throw Failure("SHARE のスロットの番号が大きい");
        auto &slot = slots[request.index];
        const auto width = static_cast<uint32_t>(picture_.width);
        const auto height = static_cast<uint32_t>(picture_.height);
        if (request.stride % sizeof(float) != 0 || request.stride < width * sizeof(float)) {
            throw Failure("画像の行の幅が合わない");
        }
        const size_t bytes = kRgb * height * static_cast<size_t>(request.stride);
        slot.stride = request.stride;
        if (way_ == protocol::Way::Shm) {
            slot.mapping.emplace(protocol::Mapping::map(
#ifdef _WIN32
                handle,
#else
                fd,
#endif
                request.size));
            if (slot.mapping->size() < bytes) throw Failure("共有メモリが小さい");
            slot.pinned = std::make_unique<cuda::Pinned>(slot.mapping->data(), bytes);
            return;
        }
#ifdef _WIN32
        // 取り込んだ物は CUDA が自分で参照を持つので、複製してもらったハンドルは閉じる。
        const UniqueHandle owned(handle);
        slot.shared = cuda::SharedBuffer::import_d3d12(handle, request.size, bytes);
#else
        slot.shared = cuda::SharedBuffer::import_fd(fd, request.size, bytes);
#endif
        return;
    }
    case protocol::What::Ready:
    case protocol::What::Done: {
        const size_t at = request.what == protocol::What::Ready ? 0 : 1;
#ifdef _WIN32
        const UniqueHandle owned(handle);
        fences_[at] = cuda::SharedFence::import_d3d12(handle);
#else
        fences_[at] = cuda::SharedFence::import_timeline_fd(fd);
#endif
        return;
    }
    }
    throw Failure("SHARE の種類が分からない");
}

void Interpolator::collect_times() {
    for (auto &timing : timings_) {
        if (!timing.pending || cudaEventQuery(timing.end.get()) != cudaSuccess) continue;
        float ms = -1.0f;
        if (cudaEventElapsedTime(&ms, timing.begin.get(), timing.end.get()) == cudaSuccess) last_ms_ = ms;
        timing.pending = false;
    }
    cudaGetLastError();  // 済んでいないイベントの問い合わせは失敗を残す
}

void Interpolator::reply_loop(protocol::Channel *channel) {
    cudaSetDevice(device_);
    for (;;) {
        std::pair<uint64_t, size_t> job;
        {
            std::unique_lock lock(mutex_);
            wake_.wait(lock, [&] { return stopping_ || !waiting_.empty(); });
            if (waiting_.empty()) return;
            job = waiting_.front();
            waiting_.pop_front();
        }
        auto &timing = timings_[job.second];
        if (cudaEventSynchronize(timing.end.get()) != cudaSuccess) {
            channel->fail("推論が終わらない");
            continue;
        }
        protocol::RunReply reply{job.first};
        cudaEventElapsedTime(&reply.gpu_ms, timing.begin.get(), timing.end.get());
        channel->reply(reply.encode());
    }
}

std::optional<protocol::RunReply> Interpolator::run(const protocol::RunRequest &request, protocol::Channel &channel) {
    const bool fenced = protocol::sync_of(way_) == protocol::Sync::Fence;
    if (request.previous >= inputs_.size() || request.current >= inputs_.size() || request.middle >= middles_.size()) {
        throw Failure("RUN のスロットの番号が大きい");
    }
    const auto &previous = inputs_[request.previous];
    const auto &current = inputs_[request.current];
    const auto &middle = middles_[request.middle];
    const bool shared = previous.ready() && current.ready() && middle.ready() &&
                        (!fenced || std::ranges::all_of(fences_, [](const auto &f) { return f.has_value(); }));
    if (!shared) throw Failure("SHARE が足りない");
    cudaStream_t stream = stream_.get();
    auto &timing = timings_[request.middle];

    // 時刻のプレーンは位相が替わったときだけ（ストリームに入れた推論が読み終えてから書く）。
    if (request.phase != phase_) {
        cuda::check(cudaStreamSynchronize(stream), "推論が終わらない");
        std::ranges::fill(staging_, request.phase);
        upload_plane(kTime, "時刻のプレーンを書けない");
        phase_ = request.phase;
    }

    const size_t padded_area = padded_.area();
    const auto width = static_cast<size_t>(picture_.width);
    const auto height = static_cast<size_t>(picture_.height);
    const size_t padded_width = static_cast<size_t>(padded_.width);
    // 本体の画像：行は行の幅ごと、プレーンは高さ × 行の幅ごと。
    const auto shared_planes = [&](const Picture &picture) {
        const size_t pitch = picture.stride / sizeof(float);
        return cuda::Planes{picture.data(), pitch, pitch * height};
    };
    const auto input = [&](InputPlane first) {
        return cuda::Planes{input_.get() + first * padded_area, padded_width, padded_area};
    };

    // 一本のストリームに順に入れるので、前のジョブが RIFE の入出力を使い終えてから読み書きし、
    // done も値の順に上がる。
    // 前にこのスロットでしたジョブの時間を、イベントを使い回す前に読む。
    if (fenced) collect_times();
    if (fenced) fences_[0]->wait(request.value, stream, "ready を待てない");
    cuda::check(cudaEventRecord(timing.begin.get(), stream), "推論をストリームに入れられない");
    cuda::copy_rgb(input(kPreviousRgb), shared_planes(previous), width, height, stream, "画像を並べられない");
    cuda::copy_rgb(input(kCurrentRgb), shared_planes(current), width, height, stream, "画像を並べられない");
    if (!engine_.context().enqueueV3(stream)) throw Failure("推論をストリームに入れられない");
    cuda::copy_rgb(shared_planes(middle), cuda::Planes{output_.get(), padded_width, padded_area}, width, height,
                   stream, "中間フレームをコピーできない");
    cuda::check(cudaEventRecord(timing.end.get(), stream), "推論をストリームに入れられない");
    if (fenced) {
        fences_[1]->signal(request.value, stream, "done を進められない");
        timing.pending = true;
        return protocol::RunReply{request.value, last_ms_};
    }
    // 完了をメッセージで返す方式：書き終えたら応答を書くスレッドが返す（応答が done の代わり）。
    if (!replier_.joinable()) replier_ = std::thread(&Interpolator::reply_loop, this, &channel);
    {
        const std::lock_guard lock(mutex_);
        waiting_.emplace_back(request.value, static_cast<size_t>(request.middle));
    }
    wake_.notify_one();
    return std::nullopt;
}

} // namespace zwtrt

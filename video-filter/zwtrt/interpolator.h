// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

// 一つの大きさの補間：本体から受けた前・今を RIFE の入力へ並べ、出力を中間フレームへコピーする。
// 受け渡しの方式は d3d12（Windows）・vulkan（Linux）の GPU の共有と、どこでも動く shm（メインメモリ）。
// ジョブは一本のストリームに順に入れる（同時に二つ抱えると、本体が次を用意して渡す間も GPU が動き続ける）。

#pragma once

#include "cuda_interop.h"
#include "engine.h"
#include "protocol.h"

#include <array>
#include <condition_variable>
#include <deque>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <vector>

namespace zwtrt {

class Interpolator {
public:
    // OPEN：方式と形式を選んで GPU を選び、エンジンを読む（無いか合わなければ作る）。
    Interpolator(nvinfer1::ILogger &logger, const std::string &onnx, const std::string &engines,
                 const protocol::OpenRequest &request);
    ~Interpolator();

    Interpolator(const Interpolator &) = delete;
    Interpolator &operator=(const Interpolator &) = delete;

    // OPEN の応答。
    protocol::OpenReply chosen() const { return {way_, protocol::Format::F32, jobs_}; }

    // SHARE：本体から来た物を取り込む（fd は channel が持っている）。
    void share(const protocol::ShareRequest &request, protocol::Channel &channel);

    // RUN：ストリームに入れて、フェンスの方式はすぐ応答を返す（ready を待って読み、done を値の順に進める）。
    // shm の方式は書き終えたら応答を書くスレッドが channel へ返す（ここは nullopt）。
    std::optional<protocol::RunReply> run(const protocol::RunRequest &request, protocol::Channel &channel);

private:
    // RIFE の入力のプレーンの並び（vs-mlrt の形）。
    enum InputPlane : size_t {
        kPreviousRgb = 0,
        kCurrentRgb = 3,
        kTime = 6,
        kGridX = 7,
        kGridY = 8,
        kScaleX = 9,
        kScaleY = 10,
    };
    static constexpr size_t kFenceSlots = 2;
    static constexpr uint32_t kMostJobs = 2;

    // 本体の画像のスロット（GPU の共有か、マップしたメインメモリ）。
    struct Picture {
        std::optional<cuda::SharedBuffer> shared;     // GPU の方式
        std::optional<protocol::Mapping> mapping;     // shm の方式
        std::unique_ptr<cuda::Pinned> pinned;
        uint32_t stride = 0;  // 行の幅（バイト）

        bool ready() const { return shared.has_value() || mapping.has_value(); }
        float *data() const;
    };
    // 中間フレームのスロットごとのジョブの時間。
    struct Timing {
        cuda::Event begin;
        cuda::Event end;
        bool pending = false;  // ストリームに入れて、まだ時間を読んでいない
    };

    void fill_grid();
    void upload_plane(InputPlane plane, std::string_view reason);
    // 完了したジョブの時間を読む（フェンスの方式の応答に）。
    void collect_times();
    // shm の方式：書き終えたジョブの応答を返す。
    void reply_loop(protocol::Channel *channel);

    protocol::Way way_;
    uint32_t jobs_;
    int device_ = 0;
    Extent picture_;
    Extent padded_;
    std::vector<float> staging_;  // プレーンを一枚ずつ CUDA へ送るためのメインメモリ
    float phase_ = -1.0f;         // 時刻のプレーンに書いてある位相
    float last_ms_ = -1.0f;       // 分かっている一番新しい完了したジョブの時間

    // 応答を書くスレッド（shm の方式）。
    std::thread replier_;
    std::mutex mutex_;
    std::condition_variable wake_;
    std::deque<std::pair<uint64_t, size_t>> waiting_;  // 値、中間フレームのスロット
    bool stopping_ = false;

    // メンバーの並びが破棄の順（後ろから）。エンジンから、ストリームは最後。
    cuda::Stream stream_;
    std::vector<Timing> timings_;  // 中間フレームのスロットごと
    cuda::DeviceFloats input_;
    cuda::DeviceFloats output_;
    std::array<std::optional<cuda::SharedFence>, kFenceSlots> fences_;  // ready・done
    std::vector<Picture> inputs_;   // 入力のスロット
    std::vector<Picture> middles_;  // 中間フレームのスロット
    RifeEngine engine_;
};

} // namespace zwtrt

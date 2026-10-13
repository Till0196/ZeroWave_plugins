// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

// 本体（ZeroWave）の「外部プロセスの補間の実装」のプロトコルの読み書き。プロトコルの全文は
// zerowave の sdk/src/video/process.md（Rust の zwsdk::video::process）で、ここはそれに合わせる。
// TensorRT にも CUDA にも寄らないので、別の外部プロセス（CoreML の実装など）は protocol.h と
// protocol.cpp をそのままコピーして使える（C++20、Windows・Linux・macOS）。
//
//   本体 → プロセス：{op u32, 長さ u32, 中身}、プロセス → 本体：{状態 i32, 長さ u32, 中身}
//   （little endian）。状態が 0 でなければ中身は理由の UTF-8。
//   Windows は標準入力・標準出力のパイプ、Unix は標準入力と標準出力を兼ねる一本の unix ソケットで、
//   fd は SHARE のメッセージに SCM_RIGHTS で付く（Channel が受けて保持し、take_fd で渡す）。
//   ジョブは同時にいくつか抱えてよい（数は OPEN で決める）。応答はどのスレッドから書いてもよい。

#pragma once

#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#endif

#include <cstddef>
#include <cstdint>
#include <deque>
#include <memory>
#include <mutex>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace zwtrt::protocol {

using Bytes = std::vector<std::byte>;

enum class Op : uint32_t {
    Open = 1,
    Share = 2,
    Run = 3,
};

enum class Status : int32_t {
    Ok = 0,
    Failed = 1,
};

// 受け渡しの方式。
enum class Way : uint32_t {
    D3d12 = 1,   // 中身：アダプターの LUID 8 バイト
    Vulkan = 2,  // 中身：deviceUUID 16 バイト、driverUUID 16 バイト
    Metal = 3,   // 中身：registryID u64、mach のサービス名（残り）
    Shm = 4,     // 中身：無し
};

// フェンスの方式（d3d12・vulkan）は RUN をストリームに入れたらすぐ応答し、完了をメッセージで返す方式（metal・shm）は書き終えてから応答する。
enum class Sync { Fence, Notify };
Sync sync_of(Way way);

enum class Format : uint32_t {
    F32 = 1,
    F16 = 2,
};

// SHARE の何か。入力のスロット（同時の数 + 1 個）と中間フレームのスロット（同時の数）、フェンス（フェンスの方式だけ）。
enum class What : uint32_t {
    Input = 0,
    Middle = 1,
    Ready = 2,
    Done = 3,
};

// 同時の数が jobs のときの入力のスロットの数。
inline uint32_t inputs_of(uint32_t jobs) { return jobs + 1; }

// metal の方式で IOSurface のポートを運ぶメッセージの msgh_id。
inline uint32_t port_id(What what, uint32_t index) { return static_cast<uint32_t>(what) * 256 + index; }

// 受けたメッセージ。op は知らない値も来うるので、そのまま持つ。
struct Message {
    uint32_t op = 0;
    Bytes body;
};

struct Offer {
    uint32_t way = 0;  // 知らない値も来うる
    Bytes id;
};

// OPEN {幅, 高さ, 同時の数, 方式の数, 方式[{方式, 長さ, 中身}], 形の数, 形[]} → {方式 u32, 形 u32, 同時の数 u32}
struct OpenRequest {
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t jobs = 1;           // 本体が出せる同時の数
    std::vector<Offer> offers;   // 本体の優先順
    std::vector<uint32_t> formats;

    static OpenRequest parse(std::span<const std::byte> body);

    // その方式を出していれば中身。
    const Offer *find(Way way) const;
    bool offers_format(Format format) const;
};

struct OpenReply {
    Way way = Way::Shm;
    Format format = Format::F32;
    uint32_t jobs = 1;  // 同時に処理できる数（1〜本体が示した数）

    Bytes encode() const;
};

// metal の方式の中身（registryID と mach のサービス名）。
struct MetalId {
    uint64_t registry_id = 0;
    std::string service;

    static MetalId parse(std::span<const std::byte> id);
};

// SHARE {何か u32, 番号 u32, 行の幅 u32, 大きさ u64, ハンドル u64} → {}
struct ShareRequest {
    What what = What::Input;
    uint32_t index = 0;   // スロットの番号。フェンスは 0
    uint32_t stride = 0;  // 画像の行の幅（バイト）。フェンスは 0
    uint64_t size = 0;    // 割り当ての大きさ
    uint64_t handle = 0;  // Windows：こちらへ複製したハンドル。Unix は 0（fd が付く）

    static ShareRequest parse(std::span<const std::byte> body);
};

// RUN {値 u64, 位相 f32, 前 u32, 今 u32, 間 u32} → {値 u64, GPU の時間 f32（ミリ秒、分からなければ負）}
// 前・今は入力のスロット、間は中間フレームのスロットの番号。
struct RunRequest {
    uint64_t value = 0;
    float phase = 0.0f;
    uint32_t previous = 0;
    uint32_t current = 0;
    uint32_t middle = 0;

    static RunRequest parse(std::span<const std::byte> body);
};

// フェンスの方式は分かっている一番新しい完了したジョブの時間、完了をメッセージで返す方式はこのジョブの時間。
struct RunReply {
    uint64_t value = 0;
    float gpu_ms = -1.0f;

    Bytes encode() const;
};

// 画像の並び：プレーン c（0 赤、1 緑、2 青）の y 行目は (c × 高さ + y) × 行の幅 バイトの所から。
inline size_t plane_offset(uint32_t channel, uint32_t row, uint32_t height, uint32_t stride) {
    return (static_cast<size_t>(channel) * height + row) * stride;
}

// メッセージのチャネル（標準入力で受け、標準出力で応答する）。
class Channel {
public:
    // 標準入出力をメッセージのチャネルにする。ほかの所が printf しても混ざらないよう、
    // チャネルを先に取ってから標準出力を標準エラーへ向け直す。
    static Channel take_stdio();

    Channel(Channel &&other) noexcept;
    Channel &operator=(Channel &&) = delete;
    Channel(const Channel &) = delete;
    ~Channel();

    // メッセージを一つ受ける。相手が閉じたら nullopt。受けるのは一つのスレッドから。
    std::optional<Message> receive();

    // 応答を一つ（どのスレッドからでも。一つずつ書く）。
    void reply(std::span<const std::byte> body = {});
    void fail(std::string_view reason);

#ifndef _WIN32
    // メッセージに付いてきた fd を一つ（来た順）。所有権は呼ぶ側。無ければ例外。
    int take_fd();
#endif
#ifdef __APPLE__
    // metal の方式：OPEN の応答の前に、本体が送ってくるポートを受ける受信ポートを作る。
    void check_in(const std::string &service);
    // SHARE の前に本体が送った mach のメッセージから、送る権利を一つ（msgh_id が port_id）。
    uint32_t take_port(What what, uint32_t index);
#endif

private:
#ifdef _WIN32
    Channel(HANDLE in, HANDLE out) : in_(in), out_(out) {}
    HANDLE in_;   // 標準入力（所有しない）
    HANDLE out_;  // 標準出力の複製（所有する）
#else
    Channel(int in, int out) : in_(in), out_(out) {}
    int in_;   // 標準入力（unix ソケット）
    int out_;  // 標準出力の複製（所有する）
    std::deque<int> fds_;  // 付いてきた fd
#endif
#ifdef __APPLE__
    uint32_t port_ = 0;  // 受信ポート（mach_port_t）
#endif
    std::unique_ptr<std::mutex> writing_ = std::make_unique<std::mutex>();  // 応答を一つずつ書く

    bool read_exact(std::span<std::byte> into);
    void write_all(std::span<const std::byte> data);
    void send(Status status, std::span<const std::byte> body);
};

// shm の方式の共有メモリをマップした物。Windows はハンドル、Unix は fd の所有権を取る。
class Mapping {
public:
#ifdef _WIN32
    static Mapping map(HANDLE handle, uint64_t size);
#else
    static Mapping map(int fd, uint64_t size);
#endif
    Mapping(Mapping &&other) noexcept;
    Mapping &operator=(Mapping &&) = delete;
    Mapping(const Mapping &) = delete;
    ~Mapping();

    std::byte *data() const { return data_; }
    uint64_t size() const { return size_; }

private:
    Mapping(std::byte *data, uint64_t size) : data_(data), size_(size) {}
    std::byte *data_ = nullptr;
    uint64_t size_ = 0;
};

} // namespace zwtrt::protocol

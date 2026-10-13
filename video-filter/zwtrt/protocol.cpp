// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

#include "protocol.h"

#include <algorithm>
#include <array>
#include <bit>
#include <cstdio>
#include <cstring>
#include <limits>
#include <stdexcept>
#include <type_traits>
#include <utility>

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#else
#include <csignal>
#include <sys/mman.h>
#include <sys/socket.h>
#include <unistd.h>
#endif
#ifdef __APPLE__
#include <mach/mach.h>
#include <servers/bootstrap.h>
#endif

namespace zwtrt::protocol {

static_assert(std::endian::native == std::endian::little, "メッセージは little endian のまま読み書きする");

namespace {

using Error = std::runtime_error;

// 中身を先頭から順に読む。足りなければ「what が短い」という例外を投げる。
class Reader {
public:
    Reader(std::span<const std::byte> bytes, const char *what) : bytes_(bytes), what_(what) {}

    template <class T>
        requires std::is_trivially_copyable_v<T>
    T take() {
        T value;
        std::memcpy(&value, take_bytes(sizeof(T)).data(), sizeof(T));
        return value;
    }

    std::span<const std::byte> take_bytes(size_t count) {
        if (bytes_.size() - at_ < count) throw Error(std::string(what_) + " が短い");
        const auto taken = bytes_.subspan(at_, count);
        at_ += count;
        return taken;
    }

    std::span<const std::byte> rest() const { return bytes_.subspan(at_); }

private:
    std::span<const std::byte> bytes_;
    const char *what_;
    size_t at_ = 0;
};

template <class T>
void append(Bytes &bytes, const T &value) {
    const auto raw = std::as_bytes(std::span(&value, 1));
    bytes.insert(bytes.end(), raw.begin(), raw.end());
}

constexpr size_t kHeadSize = sizeof(uint32_t) * 2;

} // namespace

Sync sync_of(Way way) { return way == Way::D3d12 || way == Way::Vulkan ? Sync::Fence : Sync::Notify; }

OpenRequest OpenRequest::parse(std::span<const std::byte> body) {
    Reader reader(body, "OPEN");
    OpenRequest request;
    request.width = reader.take<uint32_t>();
    request.height = reader.take<uint32_t>();
    request.jobs = reader.take<uint32_t>();
    const auto offers = reader.take<uint32_t>();
    for (uint32_t i = 0; i < offers; i++) {
        Offer offer;
        offer.way = reader.take<uint32_t>();
        const auto id = reader.take_bytes(reader.take<uint32_t>());
        offer.id.assign(id.begin(), id.end());
        request.offers.push_back(std::move(offer));
    }
    const auto formats = reader.take<uint32_t>();
    for (uint32_t i = 0; i < formats; i++) request.formats.push_back(reader.take<uint32_t>());
    return request;
}

const Offer *OpenRequest::find(Way way) const {
    const auto found = std::ranges::find(offers, static_cast<uint32_t>(way), &Offer::way);
    return found == offers.end() ? nullptr : &*found;
}

bool OpenRequest::offers_format(Format format) const {
    return std::ranges::find(formats, static_cast<uint32_t>(format)) != formats.end();
}

Bytes OpenReply::encode() const {
    Bytes bytes;
    append(bytes, static_cast<uint32_t>(way));
    append(bytes, static_cast<uint32_t>(format));
    append(bytes, jobs);
    return bytes;
}

MetalId MetalId::parse(std::span<const std::byte> id) {
    Reader reader(id, "metal の方式の中身");
    MetalId parsed;
    parsed.registry_id = reader.take<uint64_t>();
    const auto rest = reader.rest();
    parsed.service.assign(reinterpret_cast<const char *>(rest.data()), rest.size());
    return parsed;
}

ShareRequest ShareRequest::parse(std::span<const std::byte> body) {
    Reader reader(body, "SHARE");
    ShareRequest request;
    request.what = static_cast<What>(reader.take<uint32_t>());
    request.index = reader.take<uint32_t>();
    request.stride = reader.take<uint32_t>();
    request.size = reader.take<uint64_t>();
    request.handle = reader.take<uint64_t>();
    return request;
}

RunRequest RunRequest::parse(std::span<const std::byte> body) {
    Reader reader(body, "RUN");
    RunRequest request;
    request.value = reader.take<uint64_t>();
    request.phase = reader.take<float>();
    request.previous = reader.take<uint32_t>();
    request.current = reader.take<uint32_t>();
    request.middle = reader.take<uint32_t>();
    return request;
}

Bytes RunReply::encode() const {
    Bytes bytes;
    append(bytes, value);
    append(bytes, gpu_ms);
    return bytes;
}

std::optional<Message> Channel::receive() {
    std::array<std::byte, kHeadSize> head;
    if (!read_exact(head)) return std::nullopt;
    Reader reader(head, "メッセージのヘッダー");
    Message message;
    message.op = reader.take<uint32_t>();
    message.body.resize(reader.take<uint32_t>());
    if (!read_exact(message.body)) return std::nullopt;
    return message;
}

void Channel::send(Status status, std::span<const std::byte> body) {
    // ヘッダーと中身を一度に書く。
    Bytes message;
    message.reserve(kHeadSize + body.size());
    append(message, static_cast<int32_t>(status));
    append(message, static_cast<uint32_t>(body.size()));
    message.insert(message.end(), body.begin(), body.end());
    const std::lock_guard lock(*writing_);
    write_all(message);
}

void Channel::reply(std::span<const std::byte> body) { send(Status::Ok, body); }

void Channel::fail(std::string_view reason) { send(Status::Failed, std::as_bytes(std::span(reason))); }

#ifdef _WIN32

Channel Channel::take_stdio() {
    HANDLE in = GetStdHandle(STD_INPUT_HANDLE);
    // 複製を持つ（_dup2 は CRT の 1 番が持っていた元のハンドルを閉じる）。
    HANDLE out = nullptr;
    DuplicateHandle(GetCurrentProcess(), GetStdHandle(STD_OUTPUT_HANDLE), GetCurrentProcess(), &out, 0, FALSE,
                    DUPLICATE_SAME_ACCESS);
    SetStdHandle(STD_OUTPUT_HANDLE, GetStdHandle(STD_ERROR_HANDLE));
    _dup2(_fileno(stderr), _fileno(stdout));
    return Channel(in, out);
}

Channel::Channel(Channel &&other) noexcept
    : in_(other.in_), out_(std::exchange(other.out_, nullptr)), writing_(std::move(other.writing_)) {}

Channel::~Channel() {
    if (out_ != nullptr) CloseHandle(out_);
}

bool Channel::read_exact(std::span<std::byte> into) {
    while (!into.empty()) {
        DWORD got = 0;
        const auto want = static_cast<DWORD>(std::min<size_t>(into.size(), std::numeric_limits<DWORD>::max()));
        if (!ReadFile(in_, into.data(), want, &got, nullptr) || got == 0) return false;
        into = into.subspan(got);
    }
    return true;
}

void Channel::write_all(std::span<const std::byte> data) {
    while (!data.empty()) {
        DWORD put = 0;
        const auto want = static_cast<DWORD>(std::min<size_t>(data.size(), std::numeric_limits<DWORD>::max()));
        if (!WriteFile(out_, data.data(), want, &put, nullptr) || put == 0) return;
        data = data.subspan(put);
    }
}

Mapping Mapping::map(HANDLE handle, uint64_t size) {
    void *view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, static_cast<SIZE_T>(size));
    // マップした物が参照を持つので、ハンドルは閉じてよい。
    CloseHandle(handle);
    if (view == nullptr) throw Error("共有メモリをマップできない");
    return Mapping(static_cast<std::byte *>(view), size);
}

Mapping::~Mapping() {
    if (data_ != nullptr) UnmapViewOfFile(data_);
}

#else

Channel Channel::take_stdio() {
    // 本体が先に閉じても、書いたところで落ちない。
    std::signal(SIGPIPE, SIG_IGN);
    const int out = dup(STDOUT_FILENO);
    std::fflush(stdout);
    dup2(STDERR_FILENO, STDOUT_FILENO);
    return Channel(STDIN_FILENO, out);
}

Channel::Channel(Channel &&other) noexcept
    : in_(other.in_), out_(std::exchange(other.out_, -1)), fds_(std::move(other.fds_)),
      writing_(std::move(other.writing_)) {
#ifdef __APPLE__
    port_ = std::exchange(other.port_, 0);
#endif
}

Channel::~Channel() {
    if (out_ >= 0) close(out_);
    for (int fd : fds_) close(fd);
}

bool Channel::read_exact(std::span<std::byte> into) {
    // いつも recvmsg で読み、付いてきた fd を保持する（read だと捨てられる）。
    while (!into.empty()) {
        iovec iov{into.data(), into.size()};
        alignas(cmsghdr) char control[CMSG_SPACE(sizeof(int) * 4)];
        msghdr header{};
        header.msg_iov = &iov;
        header.msg_iovlen = 1;
        header.msg_control = control;
        header.msg_controllen = sizeof(control);
        const ssize_t got = recvmsg(in_, &header, 0);
        if (got <= 0) return false;
        for (cmsghdr *c = CMSG_FIRSTHDR(&header); c != nullptr; c = CMSG_NXTHDR(&header, c)) {
            if (c->cmsg_level != SOL_SOCKET || c->cmsg_type != SCM_RIGHTS) continue;
            const size_t count = (c->cmsg_len - CMSG_LEN(0)) / sizeof(int);
            for (size_t i = 0; i < count; i++) {
                int fd;
                std::memcpy(&fd, CMSG_DATA(c) + i * sizeof(int), sizeof(int));
                fds_.push_back(fd);
            }
        }
        into = into.subspan(static_cast<size_t>(got));
    }
    return true;
}

void Channel::write_all(std::span<const std::byte> data) {
    while (!data.empty()) {
        const ssize_t put = write(out_, data.data(), data.size());
        if (put <= 0) return;
        data = data.subspan(static_cast<size_t>(put));
    }
}

int Channel::take_fd() {
    if (fds_.empty()) throw Error("SHARE に fd が付いていない");
    const int fd = fds_.front();
    fds_.pop_front();
    return fd;
}

Mapping Mapping::map(int fd, uint64_t size) {
    void *view = mmap(nullptr, static_cast<size_t>(size), PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    // マップした物が参照を持つので、fd は閉じてよい。
    close(fd);
    if (view == MAP_FAILED) throw Error("共有メモリをマップできない");
    return Mapping(static_cast<std::byte *>(view), size);
}

Mapping::~Mapping() {
    if (data_ != nullptr) munmap(data_, static_cast<size_t>(size_));
}

#endif

Mapping::Mapping(Mapping &&other) noexcept
    : data_(std::exchange(other.data_, nullptr)), size_(std::exchange(other.size_, 0)) {}

#ifdef __APPLE__

void Channel::check_in(const std::string &service) {
    mach_port_t port = MACH_PORT_NULL;
    if (bootstrap_check_in(bootstrap_port, service.c_str(), &port) != KERN_SUCCESS) {
        throw Error("bootstrap_check_in ができない：" + service);
    }
    port_ = port;
}

uint32_t Channel::take_port(What what, uint32_t index) {
    struct {
        mach_msg_header_t header;
        mach_msg_body_t body;
        mach_msg_port_descriptor_t port;
        mach_msg_trailer_t trailer;
    } message{};
    const kern_return_t got = mach_msg(&message.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(message), port_,
                                       5000, MACH_PORT_NULL);
    if (got != KERN_SUCCESS) throw Error("IOSurface のポートが来ない");
    if (message.header.msgh_id != static_cast<mach_msg_id_t>(port_id(what, index)) ||
        message.body.msgh_descriptor_count != 1) {
        throw Error("IOSurface のポートのメッセージが違う");
    }
    return message.port.name;
}

#endif

} // namespace zwtrt::protocol

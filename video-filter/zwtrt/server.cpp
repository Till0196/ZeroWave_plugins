// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

#include "server.h"

#include "failure.h"

#include <exception>
#include <utility>

namespace zwtrt {

Server::Server(nvinfer1::ILogger &logger, std::string onnx, std::string engines, protocol::Channel channel)
    : logger_(logger), onnx_(std::move(onnx)), engines_(std::move(engines)), channel_(std::move(channel)) {}

void Server::serve() {
    while (auto message = channel_.receive()) {
        try {
            handle(*message);
        } catch (const std::exception &error) {
            channel_.fail(error.what());
        }
    }
}

void Server::handle(const protocol::Message &message) {
    switch (static_cast<protocol::Op>(message.op)) {
    case protocol::Op::Open:
        return open(protocol::OpenRequest::parse(message.body));
    case protocol::Op::Share:
        return share(protocol::ShareRequest::parse(message.body));
    case protocol::Op::Run:
        return run(protocol::RunRequest::parse(message.body));
    }
    throw Failure("不明なメッセージ");
}

void Server::open(const protocol::OpenRequest &request) {
    // 前の物を先に手放す（GPU のメモリを空けてから作る）。
    interpolator_.reset();
    interpolator_ = std::make_unique<Interpolator>(logger_, onnx_, engines_, request);
    channel_.reply(interpolator_->chosen().encode());
}

void Server::share(const protocol::ShareRequest &request) {
    if (!interpolator_) throw Failure("SHARE が OPEN より先");
    interpolator_->share(request, channel_);
    channel_.reply();
}

void Server::run(const protocol::RunRequest &request) {
    if (!interpolator_) throw Failure("RUN が早い（OPEN の前）か短い");
    // 完了をメッセージで返す方式の応答は、書き終えたら補間の側が返す。
    if (const auto reply = interpolator_->run(request, channel_)) channel_.reply(reply->encode());
}

} // namespace zwtrt

// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

// 応答のループ：メッセージを一つずつ受け、Interpolator に渡して応答する。

#pragma once

#include "interpolator.h"
#include "protocol.h"

#include <memory>
#include <string>

namespace zwtrt {

class Server {
public:
    Server(nvinfer1::ILogger &logger, std::string onnx, std::string engines, protocol::Channel channel);

    // 相手が閉じるまでメッセージをやり取りする。失敗は理由を返して続ける。
    void serve();

private:
    void handle(const protocol::Message &message);
    void open(const protocol::OpenRequest &request);
    void share(const protocol::ShareRequest &request);
    void run(const protocol::RunRequest &request);

    nvinfer1::ILogger &logger_;
    std::string onnx_;
    std::string engines_;  // エンジンのフォルダー
    protocol::Channel channel_;
    std::unique_ptr<Interpolator> interpolator_;  // OPEN が完了するまで空
};

} // namespace zwtrt

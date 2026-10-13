// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0
//
// zwtrt：RIFE を TensorRT 10 と CUDA で動かす、本体の「外部プロセスの補間の実装」
// （zerowave の core/src/output/video/host/process.rs）。video-filter の「RIFE（TensorRT）」が
// プラグインの設定（rife_program・rife_dir・rife_onnx・rife_tensorrt・rife_cuda）からこれを起動させる。本体には C++ も
// TensorRT も入らず、こちらが落ちても巻き込まない。
//
//   zwtrt serve <ONNX> <エンジンのフォルダー>
//       本体のメッセージを標準入力で受けて標準出力で応答する（protocol.h、プロトコルは zerowave の
//       sdk/src/video/process.md）。受け渡しの方式は d3d12（Windows）・vulkan（Linux）の GPU の
//       共有か、どこでも動く shm（メインメモリ）で、本体が出す物から選ぶ。
//       OPEN で大きさを聞いたらフォルダーのエンジンを読み、無いか合わなければ作り直す（数分。
//       本体はその間、補間せずに待つ）。進み具合は標準エラーの `@status` の行で通知する（report.h）。
//       画像は本体の共有バッファ（かメインメモリ）で来るので、CUDA のコピーで RIFE の入力へ並べ、
//       出力を中間フレームへコピーする（interpolator.h）。
//   zwtrt build <ONNX> <エンジン> <幅> <高さ> [プレーンの数=11] [fp16=1]
//       エンジンだけ作って保存する（前もって作っておくとき）。
//
// ビルドは Windows が build.cmd（MSVC。TensorRT の C++ の仮想関数テーブルは MSVC の ABI で並ぶ）、
// Linux が build.sh（g++）。動かすには TensorRT と cudart のライブラリーのフォルダーを PATH か
// LD_LIBRARY_PATH に（Windows は設定の rife_tensorrt・rife_cuda）。

#include "engine.h"
#include "failure.h"
#include "protocol.h"
#include "report.h"
#include "server.h"

#include <charconv>
#include <exception>
#include <format>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace {

constexpr int kUsage = 2;
constexpr std::string_view kUsageText = "使い方：zwtrt serve <ONNX> <エンジンのフォルダー>\n"
                                        "       zwtrt build <ONNX> <エンジン> <幅> <高さ> [プレーンの数=11] [fp16=1]";

// atoi と同じく、読めなければ 0。
int to_int(std::string_view text) {
    int value = 0;
    std::from_chars(text.data(), text.data() + text.size(), value);
    return value;
}

int serve(zwtrt::TrtLogger &logger, std::span<const std::string_view> args) {
    zwtrt::Server server(logger, std::string(args[0]), std::string(args[1]), zwtrt::protocol::Channel::take_stdio());
    server.serve();
    return 0;
}

int build(zwtrt::TrtLogger &logger, std::span<const std::string_view> args) {
    const std::string onnx(args[0]);
    const std::string engine(args[1]);
    // 大きさは serve と同じく 32 の倍数にパディングする。
    const zwtrt::Extent size = zwtrt::padded({to_int(args[2]), to_int(args[3])});
    zwtrt::BuildOptions options;
    if (args.size() > 4) options.planes = to_int(args[4]);
    if (args.size() > 5) options.fp16 = to_int(args[5]) != 0;
    const auto blob = zwtrt::build_engine(logger, onnx, size, options);
    if (!zwtrt::write_file(engine, blob)) throw zwtrt::Failure("保存できない：" + engine);
    zwtrt::log_line(std::format("保存した：{}（{} バイト）", engine, blob.size()));
    return 0;
}

} // namespace

int main(int argc, char **argv) {
    const std::vector<std::string_view> args(argv + 1, argv + argc);
    const auto rest = std::span(args).subspan(args.empty() ? 0 : 1);
    zwtrt::TrtLogger logger;
    try {
        if (args.size() >= 3 && args[0] == "serve") return serve(logger, rest);
        if (args.size() >= 5 && args[0] == "build") return build(logger, rest);
    } catch (const std::exception &error) {
        zwtrt::log_line(error.what());
        return 1;
    }
    zwtrt::log_line(kUsageText);
    return kUsage;
}

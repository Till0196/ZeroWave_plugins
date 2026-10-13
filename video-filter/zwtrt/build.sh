#!/bin/sh
# Linux の zwtrt を g++（C++20）でビルドする：build.sh [TensorRT のフォルダー] [CUDA のフォルダー=/usr/local/cuda]
# TensorRT が /usr の下（NVIDIA の TensorRT のコンテナーなど）ならフォルダーは要らない。
# 動かすには TensorRT と cudart のライブラリーを LD_LIBRARY_PATH に。
set -e
cd "$(dirname "$0")"
CUDA=${2:-/usr/local/cuda}
MORE=""
if [ -n "$1" ]; then MORE="-isystem $1/include -L $1/lib"; fi
g++ -std=c++20 -O2 -Wall -Wextra $MORE -isystem "$CUDA/include" \
    -o zwtrt main.cpp server.cpp interpolator.cpp engine.cpp cuda_interop.cpp protocol.cpp report.cpp \
    -L "$CUDA/lib64" -lnvinfer -lnvonnxparser -lcudart

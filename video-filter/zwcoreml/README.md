# zwcoreml

Mac で RIFE のフレーム補間を動かすプログラムです。
video-filter の「RIFE（TensorRT・CoreML）」（`interpolate/@rife.wgsl`）を Mac で選ぶと、ZeroWave がこのプログラムを起動します。
RIFE の計算には CoreML を使い、できるだけ Neural Engine（ANE）で動かします。
Windows・Linux では、同じ設定で `zwtrt` を使います。

macOS 13 以降で動きます。
Neural Engine を使えるのは Apple シリコンの Mac だけで、Intel の Mac では GPU で動きます。

## 使い方

1. RIFE のモデルを CoreML の形式に変換します（次の「モデルを作る」）。
2. `zwcoreml` の実行ファイルを、モデルと同じフォルダーに置きます。
   リリースの zip から取り出した物は公証していないので、`xattr -d com.apple.quarantine zwcoreml` で隔離の印を外します。
3. video-filter の設定の `rife_program` に `zwcoreml` を、`rife_dir` にモデルのフォルダーを指定します。
4. プリセットに「RIFE（TensorRT・CoreML）」を加えます。

## モデルを作る

モデルは同梱していません。
[vs-rife](https://github.com/HolyWu/vs-rife)（HolyWu、MIT）が配布している RIFE（[Practical-RIFE](https://github.com/hzwer/Practical-RIFE)、hzwer、MIT）の重みを、`convert.py` で CoreML のモデル（mlpackage）に変換します。

```sh
git clone --depth 1 https://github.com/HolyWu/vs-rife.git
curl -LO https://github.com/HolyWu/vs-rife/releases/download/model/flownet_v4.6.pkl
python3.12 -m venv venv && ./venv/bin/pip install coremltools "torch==2.7.0" numpy
./venv/bin/python convert.py --vsrife vs-rife --model 4.6 --pkl flownet_v4.6.pkl \
    --out <モデルのフォルダー> --scale 0.5 --size 1440x1080 1920x1080
```

- `--model` は `4.6`・`4.22.lite`・`4.25.lite` から選べます。
- `--scale` は動きを計算するときの縮小率で、`1`・`0.5`・`0.25` から選べます。小さいほど軽くなります。
- `--size` には、見る放送の映像の大きさ（1440x1080、1920x1080、SD なら 720x480 など）を並べます。Neural Engine は大きさの決まったモデルしか扱えないため、大きさごとに `rife-v4.6-s0.5-1440x1080.mlpackage` のようなモデルができます。
- 映像の大きさに合うモデルが無いときは、「モデルが無い」と通知して補間しません。
- モデルは、初めて読み込むときに CoreML の実行用の形式（`.mlmodelc`）に変換して、同じフォルダーに保存します。変換には 1〜2 秒かかり、その間は進み具合を表示します。

## 設定

video-filter の設定で指定します。

| 項目 | 内容 |
|---|---|
| `rife_program` | `zwcoreml` の実行ファイル |
| `rife_dir` | モデルのフォルダー。空のときは `zwcoreml` と同じフォルダー |
| `rife_coreml` | 使うモデルの名前（既定は `rife-v4.6-s0.5`）。後ろに `:gpu`・`:ane`・`:cpu` を付けると、計算に使うデバイスを指定できます（付けなければ CoreML が選びます） |
| `rife_way` | フレームの受け渡し方式（`metal`・`shm`）。空のときは自動で選びます |

## ビルド

zerowave_plugins の workspace には入っていないので、このフォルダーでビルドします。

Mac で組みます。

```sh
cargo build --release
```

できあがった `target/release/zwcoreml` を使います。

ZeroWave を使わずに処理時間を測れます。

```sh
zwcoreml bench <モデルのフォルダー> rife-v4.6-s0.5 1440 1080 300 [all|gpu|ane] [同時に処理する数 1|2]
```

## 仕組み

- 動きの計算（IFNet）は CoreML で、Neural Engine で動かします。モデルは大きさを決めた fp16 の mlprogram です。
- 元の大きさへの仕上げ（動きを拡大し、前後のフレームを動きに合わせてずらして混ぜる）は、Metal で行い、ZeroWave の IOSurface に直接書き込みます。Metal のシェーダーは `src/finish.metal` で、実行時にコンパイルします。
- フレームの受け渡しは、`metal`（ZeroWave の IOSurface をそのまま使う）か、`shm`（共有メモリからコピーする）のどちらかです。
- `metal` の方式で macOS 14 以降なら、二つのフレームを同時に処理して、受け取り・推論・仕上げを重ねます。
- 元の IFNet は、段階ごとに元の大きさで前後のフレームをずらしますが、そうすると CoreML が Neural Engine を使わず CPU で動いてしまいます。このモデルでは、縮小した大きさでずらすように変えています。元の IFNet（fp32）との差は、0〜255 の平均で 0.05（等倍）〜0.15（1/2）ほどです（1440x1080、v4.6）。
- ZeroWave とのやり取りの決まりは、ZeroWave の `sdk/src/video/process.md` にあります。

## 処理時間

Apple M2 で、1 フレームの推論と仕上げにかかる時間です。

| モデル | 1440x1080 | 1920x1080 |
|---|---|---|
| v4.6・1/4 | 10.7 ms | 11.5 ms |
| v4.6・1/2 | 12.2 ms（GPU だけなら 21.5 ms） | 13.8 ms |
| v4.6・等倍 | 30.6 ms | — |
| v4.22.lite・1/2 | 24.1 ms | — |
| v4.25.lite・1/2 | 37.9 ms | — |

このうち仕上げは 2.4〜2.9 ms です。
lite のモデルは特徴を元の大きさで計算するので、v4.6 より遅くなります。

二つのフレームを同時に処理すると（v4.6・1/2、1440x1080）、1 フレームあたりの間隔は 12.5 ms から 9.3 ms に縮みます。
三つにしても 9.3 ms のままです（Neural Engine が使い切られるため）。

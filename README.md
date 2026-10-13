# zerowave_plugins

[ZeroWave](https://github.com/Till0196/ZeroWave) のプラグイン集です。
ZeroWave と同じフォルダーにある `plugins` フォルダーに入れると、起動時に読み込まれます。
プラグインは ZeroWave の SDK（`sdk`）を使って書きます。Rust なら `zwsdk::plug`、TypeScript なら `sdk/ts` です。

プラグインの入れ方は二通りあります。

- wasm ファイルを一つ置く。Rust でも TypeScript でも、ビルドすると wasm ファイル一つになります。
- フォルダーを置く。フォルダーの中に `manifest.json` と、そこで指定した wasm ファイルを入れます。

| プラグイン | 種類 | 内容 |
| --- | --- | --- |
| [tsmf](tsmf/README.md) | stream | トランスモジュレーションの多重化フレーム（TSMF）から、選んだサービスのストリームを取り出します。 |
| [pipe](pipe/README.md) | stream | 受信したストリームを、設定で選んだ外部のプログラムに通します。 |
| [video-filter](video-filter/README.md) | video・ui | 映像フィルターです。mpv のユーザーシェーダーや WGSL のシェーダーを番組ごとに使い分けられます。フレーム補間もできます。 |
| [databroadcast](databroadcast/README.md) | layer | データ放送（BML、HTML5）を表示します。 |
| [clock-comments](clock-comments/README.md) | layer | 現在時刻を五秒ごとにニコニコ風のコメントで流します。TypeScript で書いたプラグインの例です。 |

ビルドの方法と試し方は、各プラグインの README を見てください。

## layer プラグイン

layer プラグインは、画面に一枚の絵を重ねるプラグインです。
ZeroWave の画面は、映像・データ放送・字幕・OSD のレイヤーを重ねてできています。
layer プラグインは、そのどれかと置き換える（`at`）か、どれかの上や下に重ねて（`above`・`below`）表示されます。
プラグインは必要な入力（`wants`）を指定しておくと、それを `Message` として受け取れます。指定できるのは streams・captions・keys・clock・area・broadcast・popup です。
Rust では `Layer` trait を、TypeScript では `onNew`・`onMessage`・`onFrame`・`send` を使って書きます。

## テスト

```sh
cargo nextest run --workspace
```

## ライセンス

Apache-2.0 です（`LICENSE`、`NOTICE`）。
ただし `video-filter/examples/shaders/deinterlace/tgmc.wgsl` は NNEDI3 の重みを含むため、LGPL-3.0-or-later です（条文は `LICENSES` にあります）。

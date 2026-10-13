# databroadcast

データ放送を表示するプラグインです。
BML（`bml`）と、HTML5（`web`、Blitz と boa を使用）に対応しています。
ZeroWave のデータ放送のレイヤーで表示します。
`bml` と `web` はネイティブでもビルドでき、テストと例はネイティブで実行します。

## ビルド

```sh
rustup target add wasm32-wasip1
cargo build --profile plugin --target wasm32-wasip1 -p zerowave-databroadcast
```

できあがった `target/wasm32-wasip1/plugin/zerowave_databroadcast.wasm` を、`databroadcast.wasm` という名前で ZeroWave の `plugins` フォルダーにコピーします。

ほかのプラグインと違い、wasm32-wasip1 向けにビルドします（Blitz が使う web-time が、wasm32-unknown-unknown では wasm-bindgen を必要とするため）。
`plugin` プロファイルは、LTO でサイズを小さくし、シンボル名も削除します。
opt-level は 3 のままにしています。"s" にすると 4K のレンダリングが三倍以上遅くなります。

## 試す

録画したファイルで、データ放送の画面を画像に書き出せます。

```sh
cargo run -p zerowave-databroadcast --release --example dataplugin -- databroadcast.wasm 録画.ts サービス番号 out.png
ZW_NATIVE=1 cargo run -p zerowave-databroadcast --release --example dataplugin -- x 録画.ts サービス番号   # wasm を使わずネイティブで実行する
```

テストは次のコマンドで実行します。

```sh
cargo nextest run -p zerowave-databroadcast -p bml -p web
```

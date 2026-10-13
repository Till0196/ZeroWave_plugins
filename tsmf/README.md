# tsmf

トランスモジュレーションの多重化フレーム（TSMF）から、選んだサービスのストリームを取り出すプラグインです。
Rust のライブラリ（`zerowave-tsmf`）としても使えます。

## ビルド

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown -p zerowave-tsmf
```

できあがった `target/wasm32-unknown-unknown/release/zerowave_tsmf.wasm` を、ZeroWave の `plugins` フォルダーにコピーします。

## 試す

録画したキャリアから、一つのサービスだけを取り出せます。

```sh
cargo run -p zerowave-tsmf --example extract -- --service 0x0400 carrier.ts out.ts
```

テストは次のコマンドで実行します。
先に wasm をビルドしておくと、ZeroWave にプラグインとして読み込ませるテストも実行されます。

```sh
cargo nextest run -p zerowave-tsmf
```

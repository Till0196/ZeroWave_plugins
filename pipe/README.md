# pipe

受信したストリームを、設定で選んだ外部のプログラムに通すプラグインです。
プログラムは TS と MMT/TLV で別々に選べます。
ストリームは、プログラムに通す必要があるときだけ通します。
プログラムが止まったときは、自動で起動し直します。
tsmf と一緒に使うときは、tsmf で取り出した後のストリームが渡されます。

## ビルド

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown -p zerowave-pipe
```

できあがった `target/wasm32-unknown-unknown/release/zerowave_pipe.wasm` を、`pipe.wasm` という名前で ZeroWave の `plugins` フォルダーにコピーします。

## 試す

```sh
cargo nextest run -p zerowave-pipe
```

先に wasm をビルドしておくと、ZeroWave にプラグインとして読み込ませるテストも実行されます。

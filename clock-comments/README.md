# clock-comments

現在時刻を五秒ごとに、ニコニコ風のコメントで画面に流すプラグインです。
コメントの描画には [niconicomments](https://github.com/xpadev-net/niconicomments) を使っています。
TypeScript で書いたプラグインの例でもあります。

## ビルド

ZeroWave のリポジトリを、このリポジトリと同じフォルダーに `zerowave` という名前で置いておきます。

```sh
npm install
npm run build
```

`dist/clock-comments.wasm` ができます。
ZeroWave の `plugins` フォルダーにコピーするか、次のコマンドでビルドとコピーをまとめて行います。

```sh
ZW_PLUGINS=<plugins フォルダー> npm run install:local
```

`npm run build` は、esbuild で一つの JavaScript にまとめ、Javy で wasm に変換し、マニフェストを wasm に書き込みます。
Javy は初回のビルドのときに自動でダウンロードされます。

## TypeScript でプラグインを書く

layer プラグインは、画面に一枚の絵を重ねて表示します。
どのレイヤーと置き換えるか（`at`）、どのレイヤーの上や下に重ねるか（`above`・`below`）は、`package.json` の `zerowave.layer` で指定します。
画面の大きさは 1920×1080 として描きます。

```ts
import { layer, onFrame, onMessage, send } from "zwsdk";

const ctx = layer.getContext("2d")!;
let last = "";
onMessage((message) => {
  // メッセージは zwsdk::layer::Message の JSON です（{"Key":"Up"}、{"Clock":{…}} など）。
  if (typeof message === "object" && "Key" in message) last = JSON.stringify(message.Key);
});
onFrame(() => {
  ctx.clearRect(0, 0, layer.width, layer.height);
  ctx.font = "bold 64px sans-serif";
  ctx.fillStyle = "white";
  ctx.fillText(`${new Date().toLocaleTimeString()} ${last}`, 100, 100);
});
send({ Log: "起動しました" });
```

`onFrame` は `fps` で指定した回数（指定しなければ 1 秒に 20 回）呼ばれます。
`onMessage` は、`wants` で指定した入力が届いたときに呼ばれます。

プラグインの名前やバージョンなどのマニフェストは、`package.json` に書きます。
ビルドすると wasm の中に書き込まれるので、別のファイルは要りません。

```json
{ "name": "zerowave-clock-comments", "version": "0.2.0",
  "zerowave": { "hooks": ["layer"],
                "layer": { "above": "captions", "wants": ["clock", "area"], "fps": 60 } } }
```

ブラウザーの API は、次のものが使えます。

- canvas 2D（`getContext("2d")`、`document.createElement("canvas")`、`OffscreenCanvas`。`drawImage` で描けるのは canvas だけです）
- `setTimeout`・`setInterval`、`requestAnimationFrame`、`performance.now()`
- `Date`（日本標準時）
- `console.log`（ZeroWave のログに出力されます）

これ以外の DOM（`document.body` など）は中身が空で、`fetch` は使えません。

JavaScript は、QuickJS を wasm にした [Javy](https://github.com/bytecodealliance/javy) の上で動きます。
描画は ZeroWave 側で行うので、プラグインは描画の命令を送るだけです。

作ったプラグインは、ZeroWave のリポジトリで次のように試せます。

```sh
cargo run -p zwsdk --features host --example layer -- <プラグインのフォルダー> 40 out.png
```

// ZeroWave の層: いまの時刻を五秒おきに、ニコニコ風に右から左へ流す。
//
// TypeScript で書き、esbuild で一つにまとめ、javy で wasm にする。レンダリングするのは
// zwsdk の canvas 2D。時刻は Date（本体の時計、日本標準時）。

import NiconiComments, { type FormattedComment } from "@xpadev-net/niconicomments";
import { layer, now } from "zwsdk";

const EVERY_MS = 5000;

const canvas = layer as unknown as HTMLCanvasElement;
const comments: FormattedComment[] = [];
const renderer = new NiconiComments(canvas, comments, {
  format: "formatted",
  keepCA: true,
  scale: 1,
  // 既定はニコニコの画面（1530 px を中央に、両脇 195 px は流れない）。
  // ここは画面の端から端まで。
  config: { commentDrawRange: 1920, commentDrawPadding: 0 },
});

const started = now();
let next = 0;
let id = 1;

function pad(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}

function timeText(): string {
  const t = new Date();
  return `${pad(t.getHours())}:${pad(t.getMinutes())}:${pad(t.getSeconds())}`;
}

/** 経過ミリ秒 → niconicomments の vpos（1/100 秒）。 */
function vposOf(ms: number): number {
  return Math.floor(ms / 10);
}

function frame(): void {
  const elapsed = now() - started;
  if (elapsed >= next) {
    next += EVERY_MS;
    // ニコニコのコメントは vpos の 1 秒前に右端から入り始める。いま右端に
    // 出したいので、1 秒先の vpos で入れる。
    renderer.addComments({
      id: id++,
      vpos: vposOf(elapsed) + 100,
      content: timeText(),
      date: Math.floor(Date.now() / 1000),
      date_usec: 0,
      owner: false,
      premium: false,
      mail: ["big", "white"],
      user_id: 0,
      layer: -1,
      is_my_post: false,
    });
  }
  renderer.drawCanvas(vposOf(elapsed));
  requestAnimationFrame(frame);
}

requestAnimationFrame(frame);

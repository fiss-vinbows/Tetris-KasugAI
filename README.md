# KasugAI(鎹)

![KasugAI のロゴ: 鎹と C-spin](docs/images/kasugai-cspin-brackets-v1.png)

ぷよぷよテトリス2 のテトリス対戦を対象にした、Rust 製の自作テトリスAIです。

全自動・支援ツール [TBA(Tetris Beginner Assistance)](#tba-との関係) の思考エンジンとして使っています。Tetris Bot Protocol(TBP)に対応しています。

## 特徴

- **手作りの評価関数 + 自動調整**: ビーム探索の評価の重みを、自己対戦(SPSA)で調整します
- **C4W(中あけ REN)**: 中央4列をあけて積み、REN を切らさずに発火します
- **Tスピン**: TSD・TST・C-spin の形や、テトリスと TSD のどちらでも撃てる形を評価します
- **開幕の作戦**(`--opening`):
  - TD系テンプレ(テトリス堂の図。TST→TSD→パフェまで続けます)
  - 開幕TSD・種3C4W
  - 開幕パフェ(最初の10ミノで4段を消す。見えていないミノの来方も試して、パフェできる見込みが一番高い置き方を選びます)
- **相手を見た作戦**: 相手の盤面と予告から、C4W で対抗するか早撃ちするかを選びます。TDテンプレの途中で速攻を受けたら、
  TSD の後に C4W へ移ります
- **実機に合わせた対戦シミュレーター**: 壁に止められる手、キー入力・ソフトドロップ・消去の演出にかかる時間、
  実機に合わせたおじゃまの穴の列の変わり方を再現できます

## 強さ

対戦シミュレーターでの Cold Clear 2 との対戦です(2026-11-14〜15、a5eef0b 以降の探索。実機の時間を数える `--timed`、
壁に止められる再現あり)。勝率は引き分けを除いて数えています。

| 開幕 | 打ち方(重み) | CC2 の思考 | 局数 | 勝ち / 負け / 引き分け | 勝率 | 95%信頼区間 |
|---|---|---:|---:|---:|---:|---:|
| 既定 | 標準(`run16`) | 100 ms | 600 | 294 / 303 / 3 | 49.2% | 45.3〜53.2% |
| 既定 | B2B 重視(`b2b`) | 100 ms | 300 | 169 / 130 / 1 | 56.5% | 50.9〜62.0% |
| 種3C4W(`seed3`) | 標準(`run16`) | 100 ms | 1380 | 1065 / 315 / 0 | 77.2% | 74.9〜79.3% |
| 種3C4W(`seed3`) | CC2 専用(`cc2_c4w`) | 100 ms | 600 | 480 / 120 / 0 | **80.0%** | 76.6〜83.0% |
| 種3C4W(`seed3`) | 標準(`run16`) | 300 ms | 540 | 402 / 138 / 0 | 74.4% | 70.6〜77.9% |
| 種3C4W(`seed3`) | CC2 専用(`cc2_c4w`) | 300 ms | 300 | 238 / 62 / 0 | 79.3% | 74.4〜83.5% |

開幕を種3C4W(最初から C4W を組む)にすると、Cold Clear 2 には大きく勝ち越します。勝った局のほとんどは
REN 8 以上を撃っています。Cold Clear 2 の負けの1割ほどは、壁に止められて届かない手を返した「反則」です。

- 既定の開幕(TDテンプレか素の積み)では、標準の重みは互角、B2B 重視の重み(TSD の点を大きくし、相手の C4W には
  迎え撃つ)は勝ち越しています。打ち方は TBA の全自動の設定で選べます(`config/weights/styles.toml`)。
- `cc2_c4w` は run16 から、開幕 seed3 で Cold Clear 2 と戦う局だけで自動調整した重みです(`tune --vs-cc2`)。
  同じミノ列で比べると run16 より勝ちますが、既定の開幕では run16 より下がる向き(43.7% 対 47.3%、300局)なので、
  seed3 で Cold Clear 2 と戦う時の専用です。TBA の打ち方には入れていません。
- 開幕に開幕パフェを混ぜる(`seed3+pco`)と 58.3%、TD・TSD・種3 から選ぶ(`random`)と 54.6% に下がりました(240局ずつ)。

次のコマンドで同じ対戦ができます(重みと開幕を表に合わせて変えます。CC2 の思考は `--think-ms 300`)。

```bash
target/release/tetrisai-arena.exe --games 600 --a-weights config/weights/run16.toml --block-margin 1 --timed --adjudicate
```

```bash
target/release/tetrisai-arena.exe --games 600 --a-weights config/weights/cc2_c4w.toml --a-opening seed3 --block-margin 1 --timed --adjudicate --seed 3001
```

対戦シミュレーターの規則を実機に合わせて直したため(REN 中もせり上がる・おじゃまの穴の列が行ごとに変わる)、以前の
「77.1%」(`run11_cut20`、v0.1.0 の頃の規則)とは比べられません。

シミュレーターの結果で、実機(ぷよテト2)での勝率ではありません。

## 名前の由来

鎹(かすがい)は、両端が尖った「コ」の字型の金属製の釘です。逆向きにするとテトリスの **C-spin** の形によく似ています。C-spin は TST→TSD を組むための技術思想そのもので、このAIにも積極的に採用してもらう意図を込めました。

末尾を大文字にした(Kasugai ではなく **KasugAI**)のは、AI であることを主張するためです。

## 使い方

Rust のツールチェインが必要です。

```bash
cargo build --release -p tetrisai-bot
```

`target/release/tetrisai-bot.exe` ができます。標準入出力で TBP を話すので、TBP に対応したクライアントから起動します。重みは `--config` で渡します。

```bash
target/release/tetrisai-bot.exe --config config/weights/run16.toml --opening no_c4w
```

`--opening` の値は `default`・`random`・`no_c4w`(開幕C4Wを除く)・`td`・`tsd`・`seed3`・`pco`(開幕パフェ)・`ban_c4w`(開幕も中盤も C4W を組まない。相手の C4W には早撃ちで答える)です。

細かく選ぶ時は「`td+tsd+seed3+pco`」のように開幕を `+` でつなぎます(対局ごとに均等に選ぶ。`pco` は組める見込みがあれば先に狙う。何も選ばない時は `plain`)。`no_mid_c4w` を足すと中盤も C4W を使いません(開幕C4W を選んだ局を除く)。`style=名前` を足すと、`--config` と同じフォルダの `styles.toml` で引いた重み(`standard` は run16、`b2b` は B2B 重視)を `--config` の代わりに読みます(例: `--opening td+tsd+no_mid_c4w+style=b2b`)。ほかのオプションは `crates/bot/src/main.rs` の先頭に書いてあります。

TBA で使うプラグインは [Releases](https://github.com/fiss-vinbows/Tetris-KasugAI/releases) にあります。

### 重み

| ファイル | 内容 |
|---|---|
| `config/weights/run16.toml` | 標準(TBA で使っている重み) |
| `config/weights/run16_shape.toml` | run16 + テトリスと TSD の両対応の形を良形にする・パフェの後に C4W に向かわない |
| `config/weights/run17_td.toml` | TDテンプレを前提にした重み(`--opening td` で使う。速攻を受けたら TSD の後に C4W へ) |
| `config/weights/run16_rush.toml` | 相手が C4W を目指したら、C4W で対抗せずに攻撃で潰す(早撃ち) |
| `config/weights/cc2_c4w.toml` | 開幕 `seed3` で Cold Clear 2 と戦う専用(run16 から CC2 相手に自動調整) |

古い重みも、調べた時の記録として残しています。どれが何かは各ファイルの先頭にあります。

## 構成

| 場所 | 内容 |
|---|---|
| `crates/core/` | 盤面(ビットボード)・ミノ・SRS・手の生成・ルール(攻撃量・B2B・REN) |
| `crates/engine/` | 評価関数・ビーム探索・開幕と作戦の選択 |
| `crates/bot/` | TBP 互換の実行ファイル `tetrisai-bot.exe` |
| `crates/arena/` | 対戦シミュレーター `tetrisai-arena.exe`、自動調整 `tune`、C4W の集計 `c4w_audit` |
| `config/rules/ppt2.toml` | ぷよテト2のルールの数値(「暫定」は実機で要確認) |
| `config/weights/` | 評価の重み |
| `tools/` | Cold Clear 2・TBA との突き合わせなどの道具 |
| `input/` | TBA のシミュレーターで作った局面の画像(局面テストの元) |
| `docs/images/` | README のロゴの画像 |

## 開発

```bash
cargo test
```

### 対戦シミュレーター

既定は、このAI 対 Cold Clear 2 です。オプションは `crates/arena/src/main.rs` の先頭に書いてあります。

```bash
cargo build --release
target/release/tetrisai-arena.exe --games 300 --a-weights config/weights/run16.toml --block-margin 1 --timed --adjudicate
```

- `--block-margin 1`: 壁に止められる手を再現します
- `--a-opening` / `--b-opening`: 序盤の作戦(bot の `--opening` と同じ値)
- `--a-time-pct` / `--b-time-pct`: 手の時間の倍率(人の相手の遅さを再現します)
- `--b-incoming-lag`: 予告に気づくまでの遅れ(手の数)
- 環境変数 `ARENA_TRACE=<種>|all`: 1手ずつの様子を標準エラーに出します
- `--timed`: 実機ログに合わせた時間を数え、遅れている側だけが置きます(ソフトドロップや消去の演出の遅さを再現します)
- 重み同士を比べる時は `--b target/release/tetrisai-bot.exe --b-weights <TOML>` を足します。少ない局数では偶然の差が出るので、`--seed` を変えても確かめます

Cold Clear 2 と対戦するには、隣のフォルダに `../Tetris/external/cold-clear-2` が必要です(`--b` で別の場所も指定できます)。

### 評価の重みの自動調整

SPSA で調整します。途中経過は `--out` のフォルダに保存され、続きから再開できます。オプションは `crates/arena/src/bin/tune.rs` の先頭に書いてあります。

```bash
target/release/tune.exe --out records/tune_new --init config/weights/run16.toml --hours 2 --width 128 --games 64 --block-margin 1
```

### C4W の使い方の集計

発火できた割合や、REN を切った手の数を数えます。オプションは `crates/arena/src/bin/c4w_audit.rs` の先頭に書いてあります。

```bash
target/release/c4w_audit.exe --weights config/weights/run16.toml --games 200 --block-margin 1
```

### 突き合わせ

手の生成を Cold Clear 2 と比べます(`../Tetris/external/cold-clear-2` が必要です)。

```bash
cargo run --release --manifest-path tools/crosscheck-cc2/Cargo.toml -- 3000 1 tall
```

SRS の到達判定を TBA と比べます(TBA の venv の Python で実行します)。

```bash
../Tetris/venv/Scripts/python.exe tools/crosscheck_tba.py 400 1
```

TBA のクライアントのテストを bot で実行します(TBA のコードは書き換えません)。

```bash
../Tetris/venv/Scripts/python.exe tools/tba_harness.py test
```

TBA の自己対戦を Cold Clear 2 と bot で並べて比べます。

```bash
../Tetris/venv/Scripts/python.exe tools/tba_harness.py selfplay --seeds 5 --max-moves 100 --think-seconds 0.3
```

手の生成の速さを測ります。

```bash
cargo run --release --example bench_movegen
```

## TBA との関係

TBA はぷよテト2の画面とメモリを読んで操作する、別のプロジェクトです。このリポジトリの隣のフォルダ(`../Tetris`)に置く前提で、道具の一部が参照しています。TBA は `config/engine.json` で指定した bot と重みを使います。bot を作り直す時は、TBA が古い exe を使っていないか確かめてください。古い exe は新しい重みの項目を読めずにエラーになります。

## ライセンス

[MIT License](LICENSE) です。

ぷよぷよテトリス2 とは関係のない、個人の非公式プロジェクトです。「ぷよぷよテトリス」は株式会社セガの商標です。
Tetris® は The Tetris Company の商標です。オンライン対戦など、ほかの人と対戦する場で自動操作に使わないでください。

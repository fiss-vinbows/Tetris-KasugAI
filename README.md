# KasugAI(鎹)

TBA(Tetris Beginner Assistance、`../Tetris`)の思考エンジン Cold Clear 2 を置き換えるための自作のテトリスAI。
リポジトリ: https://github.com/fiss-vinbows/Tetris-KasugAI(フォルダ名は TetrisAI のまま)
ぷよぷよテトリス2 のテトリス対戦が対象。設計は [docs/DESIGN.md](docs/DESIGN.md)。

## 名前の由来

鎹(かすがい)は、両端が尖った「コ」の字型の金属製の釘。逆向きにするとテトリスの **C-spin** の形によく似ている。
C-spin は TST→TSD を組むための技術思想そのもので、このAIにも積極的に採用してもらう意図を込めている。
末尾を大文字にした(Kasugai ではなく **KasugAI**)のは、AI であることを主張するため。

## 構成

- `crates/core/` — 盤面(ビットボード)・ミノ・SRS・手の生成・ルール(攻撃量・B2B・REN)
- `crates/engine/` — 評価関数とビーム探索
- `crates/bot/` — TBP 互換の実行ファイル `tetrisai-bot.exe`(Cold Clear 2 と差し替えて使う)
- `crates/arena/` — ぷよテト2ルールの対戦シミュレーター `tetrisai-arena.exe`(TBP の bot 同士を対戦させる)
- `config/rules/ppt2.toml` — ぷよテト2のルールの数値(「暫定」は実機で要確認)
- `config/weights/tuned.toml` — 自己対戦で自動調整した評価の重み(最終版)。bot に `--config` で渡す
- `tools/crosscheck-cc2/` — Cold Clear 2 との手の生成の突き合わせ
- `tools/crosscheck_tba.py` — TBA の SRS 到達判定との突き合わせ
- `tools/tba_harness.py` — TBA の Cold Clear 2 クライアントを bot に差し替えて、TBA のテスト・自己対戦を動かす
- `input/` — ユーザーが TBA のシミュレーターで作った局面の画像(局面テストの元)

## 使い方

Rust のツールチェインが必要(TBA の README 参照)。

bot のビルド(`target/release/tetrisai-bot.exe` ができる):

```bash
cargo build --release -p tetrisai-bot
```

テスト:

```bash
cargo test
```

Cold Clear 2 との突き合わせ(隣の `../Tetris/external/cold-clear-2` が必要):

```bash
cargo run --release --manifest-path tools/crosscheck-cc2/Cargo.toml -- 3000 1 tall
```

TBA との突き合わせ(TBA の venv の Python で実行):

```bash
../Tetris/venv/Scripts/python.exe tools/crosscheck_tba.py 400 1
```

TBA のクライアントのテストを bot で実行(TBA のコードは書き換えない):

```bash
../Tetris/venv/Scripts/python.exe tools/tba_harness.py test
```

TBA の自己対戦を Cold Clear 2 と bot で並べて比べる:

```bash
../Tetris/venv/Scripts/python.exe tools/tba_harness.py selfplay --seeds 5 --max-moves 100 --think-seconds 0.3
```

対戦シミュレーター(既定は このAI 対 Cold Clear 2。オプションは crates/arena/src/main.rs の先頭):

```bash
cargo build --release -p tetrisai-bot -p tetrisai-arena
target/release/tetrisai-arena.exe --games 100 --think-ms 100 --record records/result.jsonl
```

評価の重みの自動調整(SPSA。途中経過は records/ に保存され、続きから再開できる。オプションは crates/arena/src/bin/tune.rs の先頭):

```bash
target/release/tune.exe --out records/tune_new --init config/weights/tuned.toml --hours 2 --width 128 --games 64
```

調整した重みの bot と Cold Clear 2 を対戦させる:

```bash
target/release/tetrisai-arena.exe --a-config config/weights/tuned.toml --games 200 --think-ms 100
```

手の生成の速さ:

```bash
cargo run --release --example bench_movegen
```

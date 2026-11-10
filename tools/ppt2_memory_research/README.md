# 全自動のログ集計の道具

TBA(隣の Tetris フォルダ)の全自動モードのデバッグログ(`../Tetris/debug_logs/`)を読んで集計する道具。どれもログを読むだけ。
日本語を出すので `-X utf8` を付ける。

## 全自動のログ集計(TBA のデバッグログ)

| ファイル | 内容 |
|---|---|
| `log_misplacements.py` | 画面との食い違いごとに、直前の手・予定にだけあるマス・画面にだけあるマスを並べる |
| `log_preinput_effect.py` | 先行入力を使った手と使わない手の、ミノが出てからの時間を比べる |
| `log_move_time_trend.py` | 一手の時間の内訳(待ち・思考・入力・出現待ち)が対局の進み方でどう変わるか |
| `log_clear_moves.py` | ラインを消した手の割合と一手の時間 |
| `log_stalls.py` | 手の数ごとに、待った秒数・考え直し・食い違いの回数 |
| `log_piece_order.py` | 各局のミノの順番を組み立てる(`pco_probe` の `PCO_SEQS` で、実機と同じ順番の開幕パフェを試す) |
| `log_boards.py` | 置いた手ごとの盤面(TBA の `log_boards`)から、C4W の形(おじゃま・井戸の底の残り・中あけ・出現位置の列)を数える |

使い方: `python -X utf8 <道具> debug_logs/debug_log_YYYYMMDD_HHMMSS.txt [debug_log_..._2p.txt]`

置いた位置の確認(手ごと)の集計は、ログの「置いた位置の確認」「これまで 合っていた N手・無かった M手」を grep するのが早い。

"""TBA(隣の Tetris フォルダ)の Cold Clear 2 クライアントを、このAIの bot に差し替えて動かす。

TBA のコードは書き換えず、`src.engine.cold_clear_client.COLD_CLEAR_EXE` だけをこのプロセスの中で差し替える。
TBA はいつもどおり `--config config/cold_clear_beginner.json` を渡してくるので、bot もその重みで動く。

使い方(TBA の venv の Python で実行する。先に `cargo build --release -p tetrisai-bot` が必要):
    ../Tetris/venv/Scripts/python.exe tools/tba_harness.py test
        TBA の Cold Clear 2 クライアントのテスト(統合テストを含む)を bot で実行する
    ../Tetris/venv/Scripts/python.exe tools/tba_harness.py selfplay [--seeds 5] [--max-moves 100]
            [--think-seconds 0.3] [--garbage-chance 0.0]
        TBA の自己対戦(tools/cold_clear_self_play.py)を CC2 と bot の両方で回して並べる
"""

from __future__ import annotations

import argparse
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TBA = ROOT.parent / "Tetris"
BOT_EXE = ROOT / "target" / "release" / "tetrisai-bot.exe"
sys.path.insert(0, str(TBA))

from src.engine import cold_clear_client  # noqa: E402

CC2_EXE = cold_clear_client.COLD_CLEAR_EXE


def use_engine(exe: Path) -> None:
    if not exe.exists():
        sys.exit(f"実行ファイルが無い: {exe}")
    cold_clear_client.COLD_CLEAR_EXE = exe


def run_tests() -> int:
    use_engine(BOT_EXE)
    # テストモジュールは読み込み時に COLD_CLEAR_EXE を見て統合テストを実行するか決めるので、差し替えた後で読む
    sys.path.insert(0, str(TBA / "tests"))
    import test_cold_clear_client  # noqa: E402

    suite = unittest.defaultTestLoader.loadTestsFromModule(test_cold_clear_client)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


def run_selfplay(args: argparse.Namespace) -> int:
    sys.path.insert(0, str(TBA / "tools"))
    from cold_clear_self_play import run_simulation  # noqa: E402

    rows = []
    for name, exe in (("Cold Clear 2", CC2_EXE), ("TetrisAI", BOT_EXE)):
        use_engine(exe)
        client = cold_clear_client.ColdClearClient()
        try:
            results = [
                run_simulation(client, seed, args.max_moves, args.think_seconds, args.garbage_chance)
                for seed in range(1, args.seeds + 1)
            ]
        finally:
            client.close()
        n = len(results)
        moves = sum(r["moves_survived"] for r in results)
        rows.append(
            (
                name,
                sum(r["game_over"] for r in results),
                moves / n,
                sum(r["total_lines"] for r in results) / n,
                sum(r["clear_counts"][4] for r in results) / n,
                sum(r["perfect_clears"] for r in results) / n,
                sum(r["avg_holes_per_move"] * r["moves_survived"] for r in results) / max(moves, 1),
                sum(r["max_height_seen"] for r in results) / n,
            )
        )
    print(
        f"\n{args.seeds}局 × 最大{args.max_moves}手、1手 {args.think_seconds}秒、"
        f"おじゃまの確率 {args.garbage_chance}(1局あたりの平均)"
    )
    print(f"{'エンジン':<14}{'負け':>4}{'手数':>8}{'消去行':>8}{'テトリス':>8}{'パフェ':>7}{'平均の穴':>8}{'最高段':>7}")
    for name, over, mv, lines, tetris, pc, holes, height in rows:
        print(f"{name:<14}{over:>4}{mv:>8.1f}{lines:>8.1f}{tetris:>8.1f}{pc:>7.2f}{holes:>8.2f}{height:>7.1f}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["test", "selfplay"])
    parser.add_argument("--seeds", type=int, default=5)
    parser.add_argument("--max-moves", type=int, default=100)
    parser.add_argument("--think-seconds", type=float, default=0.3)
    parser.add_argument("--garbage-chance", type=float, default=0.0)
    args = parser.parse_args()
    return run_tests() if args.mode == "test" else run_selfplay(args)


if __name__ == "__main__":
    sys.exit(main())

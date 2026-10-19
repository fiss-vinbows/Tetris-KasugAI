"""TBA(隣の Tetris フォルダ)の SRS 到達判定と、tetrisai-core の手の生成を突き合わせる。

使い方(TBA の venv の Python で実行する):
    ../Tetris/venv/Scripts/python.exe tools/crosscheck_tba.py [盤面数] [乱数の種]

確かめること:
1. 届いて固定できる4マスの集合が一致する(TBA の lock_positions の plain)
   - TBA はぷよテト2と同じ出現位置(22行の最上段)・1マスずつのソフトドロップで探す
2. こちらでTスピンになる配置は、TBA でも「回転で接地した配置」(lock_positions の spin)に含まれる
3. ソフトドロップの区間数(難度 ◎○△)が TBA の find_path_min_soft と一致する
   - こちらは「ソフトドロップを接地前に止める回数」を区間数より優先して減らす。TBA は止める回数を
     数えないので、TBA の最短手順が途中で止めている場合は、こちらの区間数のほうが多くなる。
     この場合は「設計どおりの違い」として数え、TBA の手順が止めていないのに食い違うものだけを不一致とする
"""

from __future__ import annotations

import random
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
TBA = ROOT.parent / "Tetris"
sys.path.insert(0, str(TBA))

from src.education.rules import ROWS, SPAWN_COL, SPAWN_ROW, GameState  # noqa: E402
from src.engine.srs_reach import find_path_min_soft, lock_positions  # noqa: E402

PIECES = "IOTLJSZ"
MAX_HEIGHT = 18


def random_board(rng: random.Random) -> list[str]:
    """下から積んだような凸凹に、穴と張り出しを混ぜた盤面(上の行から順)。"""
    while True:
        heights = [rng.randint(0, 12) for _ in range(10)]
        grid = [[y < heights[x] for x in range(10)] for y in range(MAX_HEIGHT)]
        for _ in range(rng.randint(0, 25)):
            x, y = rng.randrange(10), rng.randrange(MAX_HEIGHT)
            if y <= heights[x] + 2:
                grid[y][x] = not grid[y][x]
        # そろった行は消しておく
        grid = [row for row in grid if not all(row)]
        top = max((y + 1 for y, row in enumerate(grid) if any(row)), default=0)
        if top <= MAX_HEIGHT:
            return ["".join("#" if c else "." for c in grid[y]) for y in reversed(range(top))] or ["." * 10]


def to_tba(rows: list[str]) -> frozenset:
    """上から順の行を、TBA の22行座標(row 0 が最上段の非表示行)の占有マスにする。"""
    cells = set()
    for i, row in enumerate(rows):
        r = ROWS - len(rows) + i
        for c, ch in enumerate(row):
            if ch != ".":
                cells.add((r, c))
    return frozenset(cells)


def ours_to_tba(xy: str) -> tuple:
    """`x,y x,y ...`(y は下から)を TBA の (row, col) の組にする。"""
    cells = []
    for pair in xy.split():
        x, y = map(int, pair.split(","))
        cells.append((ROWS - 1 - y, x))
    return tuple(sorted(cells))


_OPS = {"←": "move_left", "→": "move_right", "左回転": "rotate_ccw", "右回転": "rotate_cw", "↓": "soft_drop"}


def mid_air_stops(state: GameState, piece: str, path: tuple[str, ...]) -> int:
    """TBA の操作手順を再生し、ソフトドロップを接地前にやめた回数を数える。"""
    sim = GameState.new(1)
    sim.board = [row[:] for row in state.board]
    sim.current, sim.orient, sim.row, sim.col = piece, 0, SPAWN_ROW, SPAWN_COL
    stops = 0
    for i, step in enumerate(path[:-1]):  # 最後はハードドロップ
        label, _, count = step.partition("×")
        for _ in range(int(count or 1)):
            assert getattr(sim, _OPS[label])(), path
        if label == "↓" and not sim.is_grounded():
            stops += 1  # 次の操作(ハードドロップ以外)を空中で行う
    return stops


def main() -> int:
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 300
    seed = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    rng = random.Random(seed)
    problems = [(piece, random_board(rng)) for _ in range(n) for piece in PIECES]

    exe = subprocess.run(
        ["cargo", "run", "--release", "-q", "--example", "dump_moves"],
        cwd=ROOT,
        input="\n".join(f"{p} {'/'.join(rows)}" for p, rows in problems) + "\n",
        capture_output=True,
        text=True,
        check=True,
    )
    lines = exe.stdout.splitlines()
    assert len(lines) == len(problems)

    set_ng = spin_ng = 0
    sections_checked = sections_ng = designed = 0
    for (piece, rows), line in zip(problems, lines):
        moves = []
        for item in filter(None, line.split("|")):
            cells, spin, stops, sections = item.split(";")
            moves.append((ours_to_tba(cells), spin, int(stops), int(sections)))
        board = to_tba(rows)
        plain, spin_set = lock_positions(board, piece)
        ours_cells = {m[0] for m in moves}
        if ours_cells != set(plain):
            set_ng += 1
            if set_ng <= 3:
                print(f"[到達集合の不一致] {piece}")
                print("  " + "\n  ".join(rows))
                print("  こちらだけ:", sorted(ours_cells - set(plain))[:5])
                print("  TBAだけ:  ", sorted(set(plain) - ours_cells)[:5])
        for cells, spin, _stops, _sections in moves:
            if spin != "None" and cells not in spin_set:
                spin_ng += 1
        # 区間数: 止める回数 0 の配置から数個を選んで比べる(TBA の探索は遅いため)
        sample = [m for m in moves if m[2] == 0]
        for cells, spin, _stops, sections in rng.sample(sample, min(3, len(sample))):
            state = GameState.new(1)
            for r, c in board:
                state.board[r][c] = "X"
            found = find_path_min_soft(state, piece, cells, spin_entry=(spin != "None"))
            sections_checked += 1
            if found is not None and found[1] < sections and mid_air_stops(state, piece, found[0]) > 0:
                designed += 1
                continue
            if found is None or found[1] != sections:
                sections_ng += 1
                if sections_ng <= 3:
                    print(f"[区間数の不一致] {piece} {cells} spin={spin} こちら={sections} TBA={found}")
                    print("  " + "\n  ".join(rows))

    print(f"盤面 {n} 個 × 7種 = {len(problems)} 問")
    print(f"到達集合の不一致: {set_ng} 問")
    print(f"Tスピンなのに TBA で回転接地にならない配置: {spin_ng} 個")
    print(f"区間数: {sections_checked} 個を比較、不一致 {sections_ng} 個"
          f"(ほかに設計どおりの違い {designed} 個: TBA の手順が途中で止めていて区間数が少ない)")
    return 0 if set_ng == spin_ng == sections_ng == 0 else 1


if __name__ == "__main__":
    sys.exit(main())

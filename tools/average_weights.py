"""自動調整のログ(log.csv)から、指定した範囲の反復の重みを平均して TOML に書き出す。

SPSA は1回ごとの勝敗のぶれで重みが行ったり来たりするので、ある範囲の平均を取るとぶれが打ち消し合って
安定した重みになる(Polyak 平均)。調整していない項目は、同じフォルダの latest.toml の値をそのまま使う。

使い方:
    ../Tetris/venv/Scripts/python.exe tools/average_weights.py records/tune_run2 400 800 records/tune_run2/avg_400_800.toml
"""

from __future__ import annotations

import csv
import re
import sys
import tomllib
from pathlib import Path


def to_toml(weights: dict) -> str:
    """平らな辞書(値は数・真偽・数の配列)を TOML にする。"""
    lines = []
    for key, value in weights.items():
        if isinstance(value, bool):
            text = "true" if value else "false"
        elif isinstance(value, list):
            text = "[" + ", ".join(repr(float(v)) for v in value) + "]"
        else:
            text = repr(value)
        lines.append(f"{key} = {text}")
    return "\n".join(lines) + "\n"


def main() -> int:
    folder, start, end, out = Path(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3]), Path(sys.argv[4])
    with open(folder / "log.csv", encoding="utf-8") as f:
        rows = list(csv.reader(f))
    names = rows[0][5:]
    picked = [r for r in rows[1:] if start < int(r[0]) <= end]
    if not picked:
        sys.exit(f"反復 {start}〜{end} のログが無い")
    means = [sum(float(r[5 + i]) for r in picked) / len(picked) for i in range(len(names))]

    weights = tomllib.loads((folder / "latest.toml").read_text(encoding="utf-8"))
    for name, value in zip(names, means):
        m = re.fullmatch(r"(\w+)\[(\d+)\]", name)
        if m:
            weights[m.group(1)][int(m.group(2))] = value
        else:
            weights[name] = value
    out.write_text(to_toml(weights), encoding="utf-8")
    print(f"反復 {int(picked[0][0])}〜{int(picked[-1][0])}({len(picked)} 回)の平均を {out} に書いた")
    return 0


if __name__ == "__main__":
    sys.exit(main())

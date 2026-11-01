"""TBA(隣の Tetris フォルダ)の TD系テンプレ6種の図を、KasugAI が読む JSON に書き出す。

【2026-11-01・ユーザー】「いきなりRENを組むのは対策されると弱いので、序盤はTDテンプレか素の状態をランダムで選ぶ」。
テンプレの図は TBA の src/engine/opener_data_td.py(テトリス堂の図。左右反転の図は TBA の openers.py が作る)を
そのまま使う。図の読み取り(ミノごとのマス・Tスピンで入れる T・既に置いてあるブロック)も TBA の openers.py に任せる。

書き出すもの(crates/engine/data/td_templates.json。engine に埋め込む):
    [{"name": テンプレ名, "forms": [{"section": 節, "existing": [[x, y], ...],
                                     "items": [{"piece": "T", "cells": [[x, y], ...], "last": bool}, ...]}]}]
座標は KasugAI の盤面と同じ(x は左から、y は下から 0)。行が消えた後に置くミノ(TBA の gap_rows)は、
序盤が「テンプレを撃つまで」(最初にラインを消すまで)なので書き出さない。

使い方(TBA の venv の Python で実行する):
    ../Tetris/venv/Scripts/python.exe tools/export_td_templates.py
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TBA = ROOT.parent / "Tetris"
OUT = ROOT / "crates" / "engine" / "data" / "td_templates.json"
sys.path.insert(0, str(TBA))

from src.engine.openers import BOARD_ROWS, TD_EXTRA_TEMPLATES  # noqa: E402


def to_xy(cells) -> list[list[int]]:
    """TBA の (行, 列)(行 0 が一番上)を KasugAI の [x, y](y 0 が一番下)にする。"""
    return sorted([col, BOARD_ROWS - 1 - row] for row, col in cells)


def main() -> None:
    templates = []
    for template in TD_EXTRA_TEMPLATES:
        forms = []
        for form in template.forms:
            items = [
                {"piece": item.piece, "cells": to_xy(item.cells), "last": item.last}
                for item in form.items
                if not item.gap_rows
            ]
            if items:
                forms.append({"section": form.section, "existing": to_xy(form.existing), "items": items})
        templates.append({"name": template.name_ja, "forms": forms})
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(templates, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8")
    for t in templates:
        first = sum(1 for f in t["forms"] if not f["existing"])
        print(f"{t['name']}: 図 {len(t['forms'])}(1巡目 {first})")
    print(f"書き出した: {OUT}")


if __name__ == "__main__":
    main()

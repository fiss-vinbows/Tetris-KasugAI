# 全自動の実機ログの「盤面(メモリ)」の行(置いた手ごと。TBA の log_boards、2026-11-10)から、C4W の形を手ごとに数える。
# 使い方: python -X utf8 tools/ppt2_memory_research/log_boards.py <debug_log_..._2p.txt> [局の番号]
# 出力: 手・高さ・下のおじゃまの行数・井戸の底の残りの段数(中あけ4列に1〜3マスある行)・中あけの行数・出現位置の列(中央4列)の高さ
import re
import sys

CENTER = range(3, 7)


def classify(rows_top_down):
    rows = list(reversed(rows_top_down))  # 下から
    garbage = 0
    for r in rows:
        if r.count('#') == 9:
            garbage += 1
        else:
            break
    residue = well = 0
    i = garbage
    while i < len(rows):
        r = rows[i]
        walls = all(r[c] == '#' for c in (0, 1, 2, 7, 8, 9))
        center = sum(r[c] == '#' for c in CENTER)
        if walls and 1 <= center <= 3:
            residue += 1
        elif walls and center == 0:
            break
        else:
            break
        i += 1
    while i < len(rows):
        r = rows[i]
        if all(r[c] == '#' for c in (0, 1, 2, 7, 8, 9)) and all(r[c] == '.' for c in CENTER):
            well += 1
            i += 1
        else:
            break
    spawn = max((h + 1 for h, r in enumerate(rows) if any(r[c] == '#' for c in CENTER)), default=0)
    return garbage, residue, well, spawn


def main():
    path = sys.argv[1]
    only = int(sys.argv[2]) if len(sys.argv) > 2 else None
    game = 0
    for line in open(path, encoding='utf-8', errors='replace'):
        if '対局開始(' in line:
            game += 1
            if only is None or only == game:
                print(f'== {game}局目')
            continue
        if only is not None and only != game:
            continue
        m = re.search(r'盤面\((\S+)\) (\d+)手目 高さ(\d+) (\S*)', line)
        if m:
            rows = m[4].split('/') if m[4] else []
            g, res, well, spawn = classify(rows)
            print(f'  {m[2]:>4}手 高さ{m[3]:>2} おじゃま{g:>2} 底の残り{res} 中あけ{well:>2} 出現位置の列{spawn:>2} ({m[1]})')
        elif '予告(メモリ)' in line or 'せり上がった' in line or '決着(' in line:
            print('       ' + line.strip('- \n').replace('[全自動] 先読み: ', '')[:70])


if __name__ == '__main__':
    main()

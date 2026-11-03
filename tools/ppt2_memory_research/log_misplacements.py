"""食い違い(考え直し)ごとに、直前の手と、予定にだけある(置いたはずなのに無い)マス・画面にだけあるマスを並べる。"""
import re, sys
for path in sys.argv[1:]:
    lines = open(path, encoding="utf-8").read().splitlines()
    moves = 0
    events = []
    for i, line in enumerate(lines):
        if re.search(r"先読み: \d+手目 ", line):
            moves += 1
        if "画面の盤面が予定と違う" in line:
            only_screen = int(re.search(r"画面にだけある (\d+)マス", line).group(1))
            only_plan = int(re.search(r"予定にだけある (\d+)マス", line).group(1))
            last = next((l for l in reversed(lines[:i]) if "[全自動] 入力:" in l), "")
            last = last.split("入力: ")[-1][:70] if last else "?"
            events.append((moves, only_plan, only_screen, last))
    print(f"== {path}: {moves} 手、食い違い {len(events)} 回")
    for e in events:
        print(f"  {e[0]:>4}手目のあと: 予定にだけある {e[1]:>2}マス / 画面にだけある {e[2]:>3}マス / 直前の手 {e[3]}")

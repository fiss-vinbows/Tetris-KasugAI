import re, sys
for path in sys.argv[1:]:
    lines = open(path, encoding="utf-8").read().splitlines()
    moves = 0
    seg = {}
    reasons = {}
    for line in lines:
        if re.search(r"先読み: \d+手目 ", line):
            moves += 1
            continue
        b = moves // 30
        d = seg.setdefault(b, {"待ち(秒)": 0, "考え直し": 0, "食い違い": 0, "読めない演出": 0})
        if "考え始められない" in line:
            d["待ち(秒)"] += 1
            r = line.split("考え始められない: ")[-1].split("(")[0].strip(" -")
            reasons[r] = reasons.get(r, 0) + 1
        if "画面の盤面が予定と違う" in line or "考え直す" in line:
            d["考え直し"] += 1
        if "1回目の食い違い" in line:
            d["食い違い"] += 1
        if "board_data_trustworthyタイムアウト" in line:
            d["読めない演出"] += 1
    print(f"== {path}: {moves} 手")
    for b in sorted(seg):
        print(f"  {b*30+1:>4}〜{b*30+30:>4}手の間: {seg[b]}")
    print("  待っていた理由:", sorted(reasons.items(), key=lambda x: -x[1])[:6])

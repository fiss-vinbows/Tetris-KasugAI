import re, sys, statistics as st
pat = re.compile(r"先読み: (\d+)手目 (\S+) \(待ち・思考 (\d+)ms 入力 (\d+)ms 出現待ち (\d+)ms 1手 (\d+)ms\)")
for path in sys.argv[1:]:
    lines = open(path, encoding="utf-8").read().splitlines()
    rows = []  # (move_text, think, input, spawn, total, after_clear, pre)
    prev_clear = False
    pre = None
    pending_pre = None  # 先行入力の記録は、消した手(その手の行より前)に書かれる。使うのは次の手
    mismatch_after_pre = 0
    last_was_pre = False
    for line in lines:
        if "先行入力 " in line:
            pending_pre = line.split("先行入力 ")[1].split("(")[0]
        if "画面の盤面が予定と違う" in line and last_was_pre:
            mismatch_after_pre += 1
        m = pat.search(line)
        if m:
            piece = m.group(2)
            think, inp, spawn, total = map(int, m.groups()[2:])
            rows.append((piece, think, inp, spawn, total, prev_clear, pre))
            last_was_pre = pre is not None
            prev_clear = spawn >= 400
            pre, pending_pre = pending_pre, None
    def show(name, sel):
        if not sel:
            print(f"  {name}: なし")
            return
        med = lambda j: round(st.median(r[j] for r in sel))
        print(f"  {name} {len(sel)}手: 待ち・思考 {med(1)} 入力 {med(2)} 1手(出現待ちを除く) {round(st.median(r[1] + r[2] for r in sel))}ms(中央値)")
    print(f"== {path}")
    show("消した手の次で先行入力あり", [r for r in rows if r[5] and r[6]])
    show("  うち HOLD", [r for r in rows if r[5] and r[6] == "hold"])
    show("  うち 回転", [r for r in rows if r[5] and r[6] and r[6] != "hold"])
    show("消した手の次で先行入力なし", [r for r in rows if r[5] and not r[6]])
    show("  うち HOLD を使う手", [r for r in rows if r[5] and not r[6] and "(HOLD)" in r[0]])
    show("消さなかった手の次", [r for r in rows if not r[5]])
    print("  先行入力の手のすぐ後に画面と食い違った回数:", mismatch_after_pre)

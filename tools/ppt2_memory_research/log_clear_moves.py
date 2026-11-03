import re, sys, statistics as st
pat = re.compile(r"先読み: (\d+)手目 .*?\(待ち・思考 (\d+)ms 入力 (\d+)ms 出現待ち (\d+)ms 1手 (\d+)ms\)")
for path in sys.argv[1:]:
    rows = [tuple(map(int, m.groups())) for m in pat.finditer(open(path, encoding="utf-8").read())]
    clear = [r for r in rows if r[3] >= 400]
    plain = [r for r in rows if r[3] < 400]
    print(f"== {path}")
    print(f"  ラインを消した手(出現待ち0.4秒以上) {len(clear)}手: 1手の平均 {st.mean(r[4] for r in clear):.0f}ms / 消さない手 {len(plain)}手: {st.mean(r[4] for r in plain):.0f}ms")
    k = 30
    for i in range(0, len(rows), k):
        seg = rows[i:i + k]
        c = sum(1 for r in seg if r[3] >= 400)
        print(f"  {i+1:>4}〜{i+len(seg):>4}手: 消した手 {c:>2}/{len(seg)}  1手の平均 {st.mean(r[4] for r in seg):.0f}ms")

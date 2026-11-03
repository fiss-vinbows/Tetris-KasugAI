import re, sys, statistics as st
pat = re.compile(r"先読み: (\d+)手目 .*?\(待ち・思考 (\d+)ms 入力 (\d+)ms 出現待ち (\d+)ms 1手 (\d+)ms\)(?: 裏の照合 (\d+)回)?")
for path in sys.argv[1:]:
    rows = [tuple(int(x or 0) for x in m.groups()) for m in pat.finditer(open(path, encoding="utf-8").read())]
    print(f"== {path}: {len(rows)} 手(先読みの手番の通し番号は考え直すたびに戻る)")
    n = len(rows)
    if not n:
        continue
    k = max(1, n // 8)
    for i in range(0, n, k):
        seg = rows[i:i + k]
        f = lambda j: round(st.median(r[j] for r in seg))
        print(f"  {i + 1:>4}〜{i + len(seg):>4}手: 待ち・思考 {f(1):>4} 入力 {f(2):>4} 出現待ち {f(3):>4} 1手 {f(4):>5}ms(中央値) 平均1手 {round(st.mean(r[4] for r in seg)):>5}")

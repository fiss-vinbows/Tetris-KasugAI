# 実機ログから各局のミノの順番(引いた順)を組み立てる(2026-11-09)。
# 使い方: PCO_SEQS=$(python -X utf8 tools/ppt2_memory_research/log_piece_order.py <ログ>) cargo run --release -p tetrisai-engine --example pco_probe -- 0 30 60
# 出力はカンマ区切り。各局の最初の約14手のミノ。! が付いた局は組み立てが矛盾している
import re, sys
games = []
for line in open(sys.argv[1], encoding='utf-8', errors='replace'):
    if '対局開始(' in line:
        games.append({'first': None, 'moves': []})
    elif not games:
        continue
    elif games[-1]['first'] is None and 'る(操作=' in line:
        m = re.search(r'操作=(\w) HOLD=(\w+) NEXT=(\w+)', line)
        games[-1]['first'] = (m[1], m[2], m[3])
    elif re.search(r'先読み: \d+手目 ', line):
        m = re.search(r'(\d+)手目 (\w)(\(HOLD\))?', line)
        games[-1]['moves'].append((m[2], bool(m[3])))
out = []
for g in games:
    cur, hold0, nxt = g['first']
    seq = {i: p for i, p in enumerate(cur + nxt)}
    pos, hold, bad = 0, None, False
    def put(i, p):
        global bad
        if seq.get(i, p) != p:
            bad = True
        seq[i] = p
    for label, used in g['moves'][:14]:
        if not used:
            put(pos, label); pos += 1
        elif hold is None:
            hold = pos; pos += 1; put(pos, label); pos += 1
        else:
            put(hold, label); hold = pos; pos += 1
    s = ''
    for i in range(max(seq) + 1):
        if i not in seq:
            break
        s += seq[i]
    out.append(s + ('!' if bad else ''))
print(','.join(out))

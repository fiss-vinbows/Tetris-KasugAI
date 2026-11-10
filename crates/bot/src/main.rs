//! TBP(Tetris Bot Protocol)互換の実行ファイル。Cold Clear 2(`cold-clear-2.exe`)と差し替えて使う。
//!
//! 標準入力で1行1メッセージの JSON を受け取り、標準出力へ1行1メッセージで返す。
//! - 起動直後に `info` を送る。`rules` には `ready` を返す
//! - `start` で局面を受け取ると、別スレッドで探索を始める。探索は幅を広げながら繰り返し、
//!   そのたびに最善手を更新する(操作している間の時間をそのまま思考時間に使える)
//! - `suggest` には、その時点の最善手を待たずに返す(まだ無ければ `moves` は空)
//! - `play`(指した手)と `new_piece`(新しく見えたミノ)で局面を進める
//!
//! TBP の拡張(Cold Clear 2 フォークと同じ): `suggestion` に読み筋 `plan`(1手目から最大5手)を付ける。
//! 独自の拡張: `suggestion` に `difficulty`(`moves` と同じ順の、操作の難しさ)を付ける。
//! 独自の拡張: `suggestion` に `aim`(狙い筋の文言。改行で2行まで。TBA の情報パネルの「狙い」に出す)を付ける。
//! 独自の拡張: `incoming` メッセージ(`{"type": "incoming", "garbage": [{"lines": 4, "delay": 1}, ...]}`)で
//! 自分への予告を受け取る。`start` にも同じ形の `incoming` を付けられる。Cold Clear 2 は知らないメッセージを
//! 無視するので、送る側は両方に同じものを送ってよい。
//! 独自の拡張: `battle` メッセージで、予告に加えて自分の攻撃ゲージと相手の情報を受け取る:
//! `{"type": "battle", "incoming": [{"lines": 4, "delay": 1}], "gauge": 0,
//!   "opponent": {"board": [[null, "G", ...], ...], "incoming": 2, "gauge": 0, "back_to_back": false}}`
//! (`board` は `start` と同じ形。`opponent` が無ければ相手を見ずに読む)
//!
//! 起動引数:
//! - `--config <ファイル>`: 評価の重み。`.json` は Cold Clear 2 の設定(`freestyle_weights`)、
//!   `.toml` はこのAIの重み。TBA は `config/cold_clear_beginner.json` を渡してくる。
//!   読めない・形式が違う・知らない項目がある重みは、理由を標準エラーに書いて終了する(終了コード 2)。
//!   以前は既定の重みで動き続けたため、別の重みで動いていることに気づけなかった
//! - `--max-width <数>`: 探索の幅の上限(既定 1024)
//! - `--no-opening`: 序盤の作戦(TD系テンプレか素の状態を対局ごとにランダムで選ぶ。設計書 8.10)を使わない
//! - `--speculate`: 見えているミノの先を1手予測して読む(計算が6〜7割増える。同じ幅では勝率57%で
//!   はっきりした差が出なかったので既定では使わない。設計書 8.4)

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use serde_json::{json, Value};
use tetrisai_core::{Board, Piece, Placement, PlayerState, Rules};
use tetrisai_engine::{
    search, IncomingGarbage, OpeningMode, OpeningTracker, Opponent, Root, SearchOptions, SearchResult, StrategyTracker, Weights,
};

const MAX_CANDIDATES: usize = 5;

struct Best {
    result: SearchResult,
    /// この局面で評価した局面の数の合計(探索を繰り返した分を含む)
    nodes: u64,
    nps: f64,
    width: usize,
    /// 狙い筋の文言(TBP の拡張 `aim`。TBA の情報パネルの「狙い」に出す)
    aim: String,
    /// C4W で続けて消せる回数の見込み(今続いている分を含む。TBP の拡張 `c4w_ren` には `ren_count` で REN 数にして送る。2026-11-05)
    c4w_ren: u32,
    /// Downstack(掘り)で続けて消せる回数の見込み(送る時は `ren_count` で REN 数にする)と、見えているミノをすべて使って消し切れたか(2026-11-07)
    downstack_ren: (u32, bool),
}

#[derive(Default)]
struct Shared {
    /// 局面が変わるたびに増やす番号。探索スレッドはこれで古い探索をやめる
    generation: u64,
    /// 探索スレッドが最後に取りかかった番号
    worked: u64,
    root: Option<Root>,
    /// 操作ミノがまだ無い `start`(最初の `new_piece` を待つ)
    pending_start: bool,
    best: Option<Best>,
}

struct Bot {
    shared: Mutex<Shared>,
    wake: Condvar,
    generation: AtomicU64,
}

impl Bot {
    /// 局面を変えたあとに呼ぶ。探索スレッドに知らせる。
    fn bump(&self, shared: &mut Shared) {
        shared.generation += 1;
        self.generation.store(shared.generation, Ordering::SeqCst);
        self.wake.notify_all();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let weights = load_weights(arg_value(&args, "--config")).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let max_width: usize = arg_value(&args, "--max-width").and_then(|s| s.parse().ok()).unwrap_or(1024);
    let speculate = args.iter().any(|a| a == "--speculate");
    let rules = Rules::ppt2();
    // 【2026-11-01・ユーザー】序盤は TD系テンプレか素の状態をランダムで選ぶ(空の盤面を受け取るたびに選び直す)
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    // 【2026-11-05・ユーザー】全自動の設定で開幕に目指すものを選ぶ(--opening default|random|td|tsd|seed3|pco)
    // 【2026-11-09】pco: 開幕パフェ(見込みが低ければ TDテンプレか開幕TSD。C4W の相手が REN を始める前に10行届ける)
    let mode = match arg_value(&args, "--opening") {
        Some(s) => OpeningMode::parse(s).unwrap_or_else(|| {
            eprintln!("--opening の値が分かりません: {s}");
            std::process::exit(1);
        }),
        None => OpeningMode::default(),
    };
    let mut opening = (!args.iter().any(|a| a == "--no-opening")).then(|| {
        OpeningTracker::with_mode(seed, mode)
            .with_after_pc(weights.after_pc_seed, weights.after_pc_no_rival)
            .with_attacked_well(weights.td_attacked_well, weights.td_attacked_clears)
    });
    let mut bag_counter = BagCounter::default();
    let observe = |opening: &mut Option<OpeningTracker>, root: &mut Root| {
        if let Some(tracker) = opening.as_mut() {
            root.opening = tracker.observe(root, &rules);
            root.opening_well = tracker.well_now();
        }
    };
    // 【2026-11-03・ユーザー】作戦層: 相手の盤面(battle で受け取る。全自動だけ)を見て、C4W で対抗するか早撃ちするかを選ぶ
    let (c4w_response, pivot_opening_rows) = (weights.c4w_response, weights.pivot_opening_rows);
    let mut strategy = StrategyTracker::default();

    let bot = Arc::new(Bot { shared: Mutex::new(Shared::default()), wake: Condvar::new(), generation: AtomicU64::new(0) });
    {
        let bot = bot.clone();
        let rules = rules.clone();
        std::thread::spawn(move || worker(&bot, &weights, &rules, max_width, speculate));
    }

    let stdout = io::stdout();
    let send = |msg: Value| {
        let mut out = stdout.lock();
        writeln!(out, "{msg}").ok();
        out.flush().ok();
    };
    send(json!({
        "type": "info",
        "name": "KasugAI",
        "version": env!("CARGO_PKG_VERSION"),
        "author": "fiss-vinbows",
        "features": [],
    }));

    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("JSON を読めない: {e}: {line}");
                continue;
            }
        };
        match msg["type"].as_str().unwrap_or("") {
            "rules" => send(json!({"type": "ready"})),
            "start" => match parse_start(&msg) {
                Some(mut root) => {
                    // TBA は袋の残り(randomizer.bag_state)を送らないので、見えたミノの数から数える(2026-11-09)
                    if msg["randomizer"]["bag_state"].is_null() {
                        root.bag = bag_counter.on_start(&root);
                    }
                    observe(&mut opening, &mut root);
                    root.strategy = strategy.current();
                    // テンプレでパフェまで撃った後は「C4W で対抗」を使わない(2026-11-09)
                    // 【2026-11-10】速攻の相手(テンプレの途中で攻撃された)には、TSD の後 C4W に移る
                    if opening.as_ref().is_some_and(|t| t.force_rival()) && root.strategy == tetrisai_engine::Strategy::Normal {
                        root.strategy = tetrisai_engine::Strategy::C4WRival;
                    }
                    if opening.as_ref().is_some_and(|t| t.suppress_rival()) && root.strategy != tetrisai_engine::Strategy::Normal {
                        root.strategy = tetrisai_engine::Strategy::Normal;
                    }
                    let mut s = bot.shared.lock().unwrap();
                    s.pending_start = root.queue.is_empty();
                    s.root = Some(root);
                    s.best = None;
                    bot.bump(&mut s);
                }
                None => eprintln!("start を読めない: {line}"),
            },
            "new_piece" => match msg["piece"].as_str().and_then(parse_piece) {
                Some(piece) => {
                    bag_counter.seen.push(piece);
                    let mut s = bot.shared.lock().unwrap();
                    if let Some(root) = s.root.as_mut() {
                        root.push_piece(piece);
                        observe(&mut opening, root);
                        s.pending_start = false;
                        // それまでの最善手は同じ局面のものなので残しておく(新しい探索で置き換わる)
                        bot.bump(&mut s);
                    }
                }
                None => eprintln!("new_piece を読めない: {line}"),
            },
            "play" => match serde_json::from_value::<Placement>(msg["move"].clone()) {
                Ok(placement) => {
                    let mut s = bot.shared.lock().unwrap();
                    if let Some(root) = s.root.as_mut() {
                        if !root.play(&placement, &rules) {
                            eprintln!("play の手のミノが操作ミノ・HOLD のどちらでもない: {line}");
                        }
                        observe(&mut opening, root);
                    }
                    s.best = None;
                    bot.bump(&mut s);
                }
                Err(e) => eprintln!("play を読めない: {e}: {line}"),
            },
            "incoming" => {
                let mut s = bot.shared.lock().unwrap();
                if let Some(root) = s.root.as_mut() {
                    root.incoming = parse_incoming(&msg["garbage"]);
                    // それまでの最善手は同じ盤面のものなので残しておく(新しい探索で置き換わる)
                    bot.bump(&mut s);
                }
            }
            "battle" => {
                let mut s = bot.shared.lock().unwrap();
                if let Some(root) = s.root.as_mut() {
                    root.incoming = parse_incoming(&msg["incoming"]);
                    root.gauge = msg["gauge"].as_u64().unwrap_or(0) as u32;
                    root.opponent = parse_opponent(&msg["opponent"]);
                    root.strategy = strategy
                        .observe(&root.state.board, root.opponent.as_ref().map(|o| &o.board))
                        .forced(c4w_response);
                    // 【2026-11-10】速攻の相手(テンプレの途中で攻撃された)には、TSD の後 C4W に移る
                    if opening.as_ref().is_some_and(|t| t.force_rival()) && root.strategy == tetrisai_engine::Strategy::Normal {
                        root.strategy = tetrisai_engine::Strategy::C4WRival;
                    }
                    if opening.as_ref().is_some_and(|t| t.suppress_rival()) && root.strategy != tetrisai_engine::Strategy::Normal {
                        root.strategy = tetrisai_engine::Strategy::Normal;
                    }
                    // 【2026-11-07・ユーザー「相手の C4W になすすべなく負ける」】相手の C4W が育ってきたら序盤を切り上げて対抗する
                    if let (Some(tracker), Some(o)) = (opening.as_mut(), root.opponent.as_ref()) {
                        if tracker.pivot_if_opponent_c4w(&o.board, pivot_opening_rows) {
                            root.opening = tetrisai_engine::Opening::None;
                        }
                    }
                    bot.bump(&mut s);
                }
            }
            "suggest" => {
                let s = bot.shared.lock().unwrap();
                send(suggestion(s.best.as_ref()));
            }
            "stop" => {
                let mut s = bot.shared.lock().unwrap();
                s.root = None;
                s.best = None;
                bot.bump(&mut s);
            }
            "quit" => break,
            _ => {} // 知らないメッセージは無視する(TBP の約束)
        }
    }
}

/// 探索スレッド: 局面が変わるたびに、幅を広げながら探索を繰り返す。
fn worker(bot: &Bot, weights: &Weights, rules: &Rules, max_width: usize, speculate: bool) {
    loop {
        let (root, generation) = {
            let mut s = bot.shared.lock().unwrap();
            while s.worked == s.generation || s.root.is_none() || s.pending_start {
                s.worked = s.generation;
                s = bot.wake.wait(s).unwrap();
            }
            s.worked = s.generation;
            (s.root.clone().unwrap(), s.generation)
        };
        let cancelled = || bot.generation.load(Ordering::SeqCst) != generation;
        let started = Instant::now();
        let mut nodes = 0u64;
        // 最初の手を遅らせないように、一番狭い探索の後で数える
        let mut downstack_ren = (0, false);
        let mut width = 8;
        while width <= max_width {
            let options = SearchOptions { width, max_depth: 6, speculate };
            let Some(result) = search(&root, &options, weights, rules, &cancelled) else { break };
            nodes += result.nodes;
            let nps = nodes as f64 / started.elapsed().as_secs_f64().max(1e-9);
            let mut s = bot.shared.lock().unwrap();
            if s.generation != generation {
                break;
            }
            let aim = tetrisai_engine::aim::describe(&root, &result.plan, rules);
            let rows = tetrisai_engine::shape::ren_rows(&root.state.board);
            let c4w_ren = if rows > 0 { root.state.consecutive_clears + rows } else { 0 };
            s.best = Some(Best { result, nodes, nps, width, aim, c4w_ren, downstack_ren });
            drop(s);
            if width == 8 {
                downstack_ren = tetrisai_engine::aim::downstack_ren(&root, rules);
            }
            width *= 2;
        }
    }
}

fn suggestion(best: Option<&Best>) -> Value {
    let Some(best) = best else {
        return json!({"type": "suggestion", "moves": [], "plan": [], "move_info": {"nodes": 0, "nps": 0.0, "extra": ""}});
    };
    let r = &best.result;
    let candidates: Vec<_> = r.candidates.iter().take(MAX_CANDIDATES).collect();
    json!({
        "type": "suggestion",
        "moves": candidates.iter().map(|c| c.mv.placement).collect::<Vec<_>>(),
        "plan": r.plan.iter().map(|m| m.placement).collect::<Vec<_>>(),
        "move_info": {
            "nodes": best.nodes,
            "nps": best.nps,
            "extra": format!("width={} depth={}", best.width, r.depth),
        },
        // 【2026-11-01・ユーザー】狙い筋(TBA の情報パネルの「狙い」)。改行で2行まで
        "aim": best.aim,
        // 【2026-11-08・ユーザー「REN 見込みが1つずれている。最初は0から」】続けて消す回数ではなく REN 数(最初の消去は 0REN)を送る
        "c4w_ren": ren_count(best.c4w_ren),
        "downstack_ren": ren_count(best.downstack_ren.0),
        "downstack_ren_open": best.downstack_ren.1,
        "difficulty": candidates.iter().map(|c| json!({
            "soft_drop_sections": c.mv.soft_drop_sections,
            "mid_air_stops": c.mv.mid_air_stops,
        })).collect::<Vec<_>>(),
    })
}

fn parse_piece(s: &str) -> Option<Piece> {
    let mut chars = s.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Piece::from_char(c)
}

/// TBP の `start` を局面にする。盤面は y=0(最下段)から順の行の配列。
/// TBP の盤面(y=0(最下段)から順の行の配列)を読む。
fn parse_board(v: &Value) -> Option<Board> {
    let mut board = Board::new();
    for (y, row) in v.as_array()?.iter().enumerate().take(tetrisai_core::board::HEIGHT) {
        for (x, cell) in row.as_array()?.iter().enumerate().take(tetrisai_core::board::WIDTH) {
            if !cell.is_null() {
                board.cols[x] |= 1 << y;
            }
        }
    }
    // ライン消去の演出中に読み取った盤面などで、そろった行が残っていることがある
    board.clear_full_lines();
    Some(board)
}

/// `battle` の `opponent` を読む。無ければ None。
fn parse_opponent(v: &Value) -> Option<Opponent> {
    Some(Opponent {
        board: parse_board(&v["board"])?,
        incoming: v["incoming"].as_u64().unwrap_or(0) as u32,
        gauge: v["gauge"].as_u64().unwrap_or(0) as u32,
        back_to_back: v["back_to_back"].as_bool().unwrap_or(false),
    })
}

/// 対局の始めから見えたミノの並びで、袋(7種1巡)の残りを数える(2026-11-09)。
/// TBA は TBP の randomizer.bag_state を送らないので、袋の残りが分からず、開幕パフェの見込みや先読みが不正確になっていた。
#[derive(Default)]
struct BagCounter {
    /// 対局の始めから見えたミノ(NEXT に出た順)
    seen: Vec<Piece>,
}

impl BagCounter {
    /// start を受け取った時。対局の始め(空の盤面・HOLD なし・NEXT がすべて違うミノ)なら数え直す。
    /// 返り値は NEXT の最後のミノの後で今の袋に残っているミノ(分からなければ空)。
    fn on_start(&mut self, root: &Root) -> Vec<Piece> {
        let fresh_bag = {
            let first: Vec<Piece> = root.queue.iter().take(7).copied().collect();
            (1..first.len()).all(|i| !first[..i].contains(&first[i]))
        };
        if root.state.board.is_empty() && root.hold.is_none() && fresh_bag {
            self.seen = root.queue.clone();
        } else {
            self.catch_up(&root.queue);
        }
        self.bag()
    }

    /// 対局の途中の start(TBA の考え直し)で、new_piece で受け取っていないミノが NEXT に見えていれば足す(2026-11-09)。
    /// 見えたミノの末尾と NEXT の頭が3個以上重なる所で合わせる(合わなければそのまま)。
    fn catch_up(&mut self, queue: &[Piece]) {
        let n = self.seen.len();
        for m in (3..=queue.len().min(n)).rev() {
            if self.seen[n - m..] == queue[..m] {
                self.seen.extend_from_slice(&queue[m..]);
                return;
            }
        }
    }

    fn bag(&self) -> Vec<Piece> {
        let k = self.seen.len() % 7;
        let recent = &self.seen[self.seen.len() - k..];
        // 今の袋で見えたミノに重なりがあれば、数え方が合っていない
        if (1..recent.len()).any(|i| recent[..i].contains(&recent[i])) {
            return Vec::new();
        }
        if k == 0 {
            return Vec::new();
        }
        Piece::ALL.iter().copied().filter(|p| !recent.contains(p)).collect()
    }
}

fn parse_start(msg: &Value) -> Option<Root> {
    let board = parse_board(&msg["board"])?;
    let queue = msg["queue"].as_array()?.iter().map(|p| p.as_str().and_then(parse_piece)).collect::<Option<Vec<_>>>()?;
    let hold = match &msg["hold"] {
        Value::Null => None,
        v => Some(v.as_str().and_then(parse_piece)?),
    };
    let combo = msg["combo"].as_u64().unwrap_or(0) as u32;
    let back_to_back = msg["back_to_back"].as_bool().unwrap_or(false);
    let state = PlayerState { board, back_to_back, consecutive_clears: combo };
    // 7種1巡の残り(TBP の randomizer.bag_state)。無ければ空(7種どれでも来うる)
    let bag = msg["randomizer"]["bag_state"]
        .as_array()
        .map(|a| a.iter().filter_map(|p| p.as_str().and_then(parse_piece)).collect())
        .unwrap_or_default();
    Some(Root { state, hold, queue, incoming: parse_incoming(&msg["incoming"]), bag, ..Default::default() })
}

/// 予告の配列(`[{"lines": 4, "delay": 1}, ...]`)を読む。無い・読めない要素は無視する。
fn parse_incoming(v: &Value) -> Vec<IncomingGarbage> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|g| {
                    let lines = g["lines"].as_u64()? as u32;
                    let delay = g["delay"].as_u64().unwrap_or(0) as u32;
                    Some(IncomingGarbage { lines, delay })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 続けて消す回数を REN 数にする(1回目の消去は 0REN、2回続けて 1REN)。
fn ren_count(clears: u32) -> u32 {
    clears.saturating_sub(1)
}

fn arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

/// 評価の重みを読む。指定が無ければ既定値。
///
/// 【2026-11-01・ユーザーの指示】読めない・形式が違う・知らない項目がある重みはエラーにする(呼び出し側で終了する)。
/// 以前は既定値で動き続けたため、TBA から見ると別の重みで動いていることに気づけなかった。
fn load_weights(path: Option<&str>) -> Result<Weights, String> {
    let Some(path) = path else { return Ok(Weights::default()) };
    let text = std::fs::read_to_string(path).map_err(|e| format!("重みのファイルを読めません: {path}: {e}"))?;
    let parsed = if Path::new(path).extension().is_some_and(|e| e == "json") {
        serde_json::from_str::<Value>(&text).map_err(|e| e.to_string()).and_then(|v| {
            // Cold Clear 2 の設定ファイルなら freestyle_weights の中が重み
            let w = if v.get("freestyle_weights").is_some() { v["freestyle_weights"].clone() } else { v };
            serde_json::from_value::<Weights>(w).map_err(|e| e.to_string())
        })
    } else {
        toml::from_str::<Weights>(&text).map_err(|e| e.to_string())
    };
    parsed.map_err(|e| {
        format!("重みのファイルの形式が違うか、この実行ファイルが知らない項目があります(古い実行ファイルなら作り直してください): {path}: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 最初の消去は0REN() {
        assert_eq!(ren_count(0), 0, "消せない");
        assert_eq!(ren_count(1), 0, "1回だけ消す");
        assert_eq!(ren_count(2), 1);
        assert_eq!(ren_count(8), 7, "8行の C4W を撃ち切ると 7REN");
    }

    #[test]
    fn startを読める() {
        let mut rows = vec![vec![Value::Null; 10]; 40];
        rows[0][0] = json!("G");
        let msg = json!({"type": "start", "board": rows, "queue": ["T", "I"], "hold": "O", "combo": 2, "back_to_back": true});
        let root = parse_start(&msg).unwrap();
        assert!(root.state.board.occupied((0, 0)));
        assert_eq!(root.queue, vec![Piece::T, Piece::I]);
        assert_eq!(root.hold, Some(Piece::O));
        assert_eq!(root.state.consecutive_clears, 2);
        assert!(root.state.back_to_back);
    }

    #[test]
    fn battleの相手の情報を読める() {
        let mut rows = vec![vec![Value::Null; 10]; 40];
        rows[0][3] = json!("G");
        let opp = parse_opponent(&json!({"board": rows, "incoming": 2, "gauge": 3, "back_to_back": true})).unwrap();
        assert!(opp.board.occupied((3, 0)));
        assert_eq!((opp.incoming, opp.gauge, opp.back_to_back), (2, 3, true));
        assert!(parse_opponent(&Value::Null).is_none());
    }

    #[test]
    fn holdを使う手でキューが進む() {
        let rules = Rules::ppt2();
        let mut root = Root { queue: vec![Piece::T, Piece::I, Piece::O], ..Default::default() };
        let placement: Placement =
            serde_json::from_value(json!({"location": {"type": "I", "orientation": "north", "x": 4, "y": 0}, "spin": "none"}))
                .unwrap();
        assert!(root.play(&placement, &rules));
        assert_eq!(root.hold, Some(Piece::T));
        assert_eq!(root.queue, vec![Piece::O]);
    }

    #[test]
    fn cc2の設定ファイルを読める() {
        let dir = std::env::temp_dir().join("tetrisai-bot-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cc2.json");
        std::fs::write(&path, r#"{"freestyle_weights": {"holes": -3.0}, "freestyle_exploitation": 0.69}"#).unwrap();
        let w = load_weights(path.to_str()).unwrap();
        assert_eq!(w.holes, -3.0);
        assert_eq!(w.height, Weights::default().height);
    }

    #[test]
    fn 重みに問題があれば既定値で動かさずエラーにする() {
        let dir = std::env::temp_dir().join("tetrisai-bot-test");
        std::fs::create_dir_all(&dir).unwrap();
        // 知らない項目(古い実行ファイル・書き間違い)
        let unknown = dir.join("unknown.toml");
        std::fs::write(&unknown, "holes = -1.0\ncenter_well_rowz = 3.0\n").unwrap();
        let err = load_weights(unknown.to_str()).unwrap_err();
        assert!(err.contains("center_well_rowz"), "{err}");
        // 形式が違う・ファイルが無い
        let broken = dir.join("broken.toml");
        std::fs::write(&broken, "holes = \n").unwrap();
        assert!(load_weights(broken.to_str()).is_err());
        assert!(load_weights(dir.join("no-such.toml").to_str()).is_err());
        // 指定が無ければ既定値
        assert_eq!(load_weights(None).unwrap(), Weights::default());
    }

    #[test]
    fn 袋の残りを見えたミノの数から数える() {
        use Piece::*;
        let mut c = BagCounter::default();
        // 対局の始め: NEXT に6種見えている → 残りは見えていない1種
        let root = Root { queue: vec![I, O, T, L, J, S], ..Default::default() };
        assert_eq!(c.on_start(&root), vec![Z]);
        // 7種目と次の袋の1つ目が見えた → 次の袋の残りは6種
        c.seen.push(Z);
        c.seen.push(T);
        assert_eq!(c.bag(), vec![I, O, L, J, S, Z]);
        // 対局の途中の start(おじゃまで盤面が変わった)では数え直さない
        let mid = Root { state: PlayerState { board: Board::from_rows(&["#########."]), ..Default::default() }, queue: vec![O, T, L, J, S, Z], ..Default::default() };
        assert_eq!(c.on_start(&mid), vec![I, O, L, J, S, Z]);
        // 途中の start で、new_piece で受け取っていない I・O が NEXT に見えていれば足す
        let later = Root { state: mid.state.clone(), queue: vec![S, Z, T, I, O], ..Default::default() };
        assert_eq!(c.on_start(&later), vec![L, J, S, Z]);
        assert_eq!(c.seen.len(), 10);
    }
}

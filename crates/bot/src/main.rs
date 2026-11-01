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
use tetrisai_engine::{search, IncomingGarbage, OpeningTracker, Opponent, Root, SearchOptions, SearchResult, Weights};

const PLAN_LEN: usize = 5;
const MAX_CANDIDATES: usize = 5;

struct Best {
    result: SearchResult,
    /// この局面で評価した局面の数の合計(探索を繰り返した分を含む)
    nodes: u64,
    nps: f64,
    width: usize,
    /// 狙い筋の文言(TBP の拡張 `aim`。TBA の情報パネルの「狙い」に出す)
    aim: String,
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
    let mut opening = (!args.iter().any(|a| a == "--no-opening")).then(|| OpeningTracker::new(seed));
    let mut observe = |root: &mut Root| {
        if let Some(tracker) = opening.as_mut() {
            root.opening = tracker.observe(root, &rules);
        }
    };

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
                    observe(&mut root);
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
                    let mut s = bot.shared.lock().unwrap();
                    if let Some(root) = s.root.as_mut() {
                        root.push_piece(piece);
                        observe(root);
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
                        observe(root);
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
            s.best = Some(Best { result, nodes, nps, width, aim });
            drop(s);
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
        "plan": r.plan.iter().take(PLAN_LEN).map(|m| m.placement).collect::<Vec<_>>(),
        "move_info": {
            "nodes": best.nodes,
            "nps": best.nps,
            "extra": format!("width={} depth={}", best.width, r.depth),
        },
        // 【2026-11-01・ユーザー】狙い筋(TBA の情報パネルの「狙い」)。改行で2行まで
        "aim": best.aim,
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
}

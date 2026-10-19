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
//!
//! 起動引数:
//! - `--config <ファイル>`: 評価の重み。`.json` は Cold Clear 2 の設定(`freestyle_weights`)、
//!   `.toml` はこのAIの重み。TBA は `config/cold_clear_beginner.json` を渡してくる
//! - `--max-width <数>`: 探索の幅の上限(既定 1024)

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use serde_json::{json, Value};
use tetrisai_core::{Board, Piece, Placement, PlayerState, Rules};
use tetrisai_engine::{search, Root, SearchOptions, SearchResult, Weights};

const PLAN_LEN: usize = 5;
const MAX_CANDIDATES: usize = 5;

struct Best {
    result: SearchResult,
    /// この局面で評価した局面の数の合計(探索を繰り返した分を含む)
    nodes: u64,
    nps: f64,
    width: usize,
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
    let weights = load_weights(arg_value(&args, "--config"));
    let max_width: usize = arg_value(&args, "--max-width").and_then(|s| s.parse().ok()).unwrap_or(1024);
    let rules = Rules::ppt2();

    let bot = Arc::new(Bot { shared: Mutex::new(Shared::default()), wake: Condvar::new(), generation: AtomicU64::new(0) });
    {
        let bot = bot.clone();
        let rules = rules.clone();
        std::thread::spawn(move || worker(&bot, &weights, &rules, max_width));
    }

    let stdout = io::stdout();
    let send = |msg: Value| {
        let mut out = stdout.lock();
        writeln!(out, "{msg}").ok();
        out.flush().ok();
    };
    send(json!({
        "type": "info",
        "name": "TetrisAI",
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
                Some(root) => {
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
                        root.queue.push(piece);
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
                    }
                    s.best = None;
                    bot.bump(&mut s);
                }
                Err(e) => eprintln!("play を読めない: {e}: {line}"),
            },
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
fn worker(bot: &Bot, weights: &Weights, rules: &Rules, max_width: usize) {
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
            let options = SearchOptions { width, max_depth: 6 };
            let Some(result) = search(&root, &options, weights, rules, &cancelled) else { break };
            nodes += result.nodes;
            let nps = nodes as f64 / started.elapsed().as_secs_f64().max(1e-9);
            let mut s = bot.shared.lock().unwrap();
            if s.generation != generation {
                break;
            }
            s.best = Some(Best { result, nodes, nps, width });
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
fn parse_start(msg: &Value) -> Option<Root> {
    let mut board = Board::new();
    for (y, row) in msg["board"].as_array()?.iter().enumerate().take(tetrisai_core::board::HEIGHT) {
        for (x, cell) in row.as_array()?.iter().enumerate().take(tetrisai_core::board::WIDTH) {
            if !cell.is_null() {
                board.cols[x] |= 1 << y;
            }
        }
    }
    // ライン消去の演出中に読み取った盤面などで、そろった行が残っていることがある
    board.clear_full_lines();
    let queue = msg["queue"].as_array()?.iter().map(|p| p.as_str().and_then(parse_piece)).collect::<Option<Vec<_>>>()?;
    let hold = match &msg["hold"] {
        Value::Null => None,
        v => Some(v.as_str().and_then(parse_piece)?),
    };
    let combo = msg["combo"].as_u64().unwrap_or(0) as u32;
    let back_to_back = msg["back_to_back"].as_bool().unwrap_or(false);
    let state = PlayerState { board, back_to_back, consecutive_clears: combo };
    Some(Root { state, hold, queue })
}

fn arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

/// 評価の重みを読む。読めなければ既定値を使う(TBA から起動されたときに止まらないように)。
fn load_weights(path: Option<&str>) -> Weights {
    let Some(path) = path else { return Weights::default() };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("重みのファイルを読めないので既定値を使う: {path}: {e}");
            return Weights::default();
        }
    };
    let parsed = if Path::new(path).extension().is_some_and(|e| e == "json") {
        serde_json::from_str::<Value>(&text).map_err(|e| e.to_string()).and_then(|v| {
            // Cold Clear 2 の設定ファイルなら freestyle_weights の中が重み
            let w = if v.get("freestyle_weights").is_some() { v["freestyle_weights"].clone() } else { v };
            serde_json::from_value::<Weights>(w).map_err(|e| e.to_string())
        })
    } else {
        toml::from_str::<Weights>(&text).map_err(|e| e.to_string())
    };
    parsed.unwrap_or_else(|e| {
        eprintln!("重みのファイルの形式が違うので既定値を使う: {path}: {e}");
        Weights::default()
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
    fn holdを使う手でキューが進む() {
        let rules = Rules::ppt2();
        let mut root = Root { state: PlayerState::default(), hold: None, queue: vec![Piece::T, Piece::I, Piece::O] };
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
        let w = load_weights(path.to_str());
        assert_eq!(w.holes, -3.0);
        assert_eq!(w.height, Weights::default().height);
    }
}

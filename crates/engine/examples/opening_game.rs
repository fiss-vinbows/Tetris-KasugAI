//! 1人で40手ずつ指し、序盤の作戦(OpeningTracker が選ぶ)がどう進んだかを表示する(調べるための道具)。
//! テンプレを選んだ対局では、序盤が終わった手と、最初に消した手(テンプレを撃ったか)を見る。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example opening_game -- [重みの TOML] [対局数]

use tetrisai_core::{Piece, PlayerState, Rules, Spin};
use tetrisai_engine::{search, shape, Opening, OpeningTracker, Root, SearchOptions, Weights};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().unwrap_or_else(|| "config/weights/ren_tuned_run8.toml".to_string());
    let games: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(12);
    let weights: Weights = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let rules = Rules::ppt2();
    let mut shots = 0;
    let mut templates = 0;
    for game in 1..=games {
        let mut rng = game.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let mut bag: Vec<Piece> = Vec::new();
        let mut next_piece = || {
            if bag.is_empty() {
                bag = Piece::ALL.to_vec();
            }
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            bag.remove((rng % bag.len() as u64) as usize)
        };
        let mut root = Root { state: PlayerState::default(), queue: (0..6).map(|_| next_piece()).collect(), ..Default::default() };
        let mut tracker = OpeningTracker::new(game * 7919);
        let mut log = String::new();
        let mut chosen = String::new();
        let mut ended = None;
        let mut first_clear = None;
        let mut max_well = 0;
        for n in 0..40 {
            root.opening = tracker.observe(&root, &rules);
            if n == 0 {
                chosen = match &root.opening {
                    Opening::Template(t) => format!("テンプレ {}", t.name),
                    Opening::Plain => "素".to_string(),
                    Opening::Tsd => "開幕TSD".to_string(),
                    Opening::None => "なし".to_string(),
                };
            }
            if ended.is_none() && !root.opening.is_active() {
                ended = Some(n);
                if std::env::var("OPENING_DEBUG").is_ok() && chosen.starts_with("テンプレ") {
                    let board = root.state.board.to_rows().join("\n    ");
                    println!("  {n} 手目で序盤が終わった盤面(次 {:?}、HOLD {:?}):\n    {board}", root.queue, root.hold);
                }
            }
            let options = SearchOptions { width: 256, max_depth: 6, speculate: false };
            let Some(r) = search(&root, &options, &weights, &rules, &|| false) else { break };
            let Some(c) = r.candidates.first() else { break };
            let mut st = root.state;
            let res = st.apply(&c.mv.placement, &rules);
            if res.lines > 0 && first_clear.is_none() {
                first_clear = Some((n, res.lines, res.spin));
            }
            root.play(&c.mv.placement, &rules);
            root.push_piece(next_piece());
            max_well = max_well.max(shape::center_well_rows(&root.state.board));
            log.push_str(&format!("{:?}", c.mv.placement.location.piece));
        }
        if chosen.starts_with("テンプレ") {
            templates += 1;
            if matches!(first_clear, Some((_, 2..=3, Spin::Full))) {
                shots += 1;
            }
        }
        println!(
            "{game:>3}: {chosen}、序盤の終わり {ended:?} 手目、最初に消した手 {first_clear:?}、中あけの最大 {max_well} 行\n     {log}"
        );
    }
    println!("テンプレを選んだ {templates} 局のうち、最初に消した手が TSD/TST: {shots} 局");
}

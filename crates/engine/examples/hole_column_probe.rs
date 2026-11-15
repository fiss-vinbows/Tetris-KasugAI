//! C4W の壁の下のおじゃまの穴の列(2列目)を、REN を撃つ前にほかの列より高く積んでいるかを調べる道具
//! (2026-11-15・ユーザーの局面)。40局ずつ、予告なしで最大12手打ち、最初に消す手の直前の盤面を見る。
//! PROBE_DEBUG=1 で1局ずつの結果と、積み過ぎた盤面を出す。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example hole_column_probe -- [重みの TOML]

use tetrisai_core::{Board, Piece, PlayerState, Rules};
use tetrisai_engine::{search, shape, Root, SearchOptions, Weights};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "config/weights/run16.toml".to_string());
    let weights: Weights = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let rules = Rules::ppt2();
    // ユーザーの局面: 中あけ9行の下に、穴が中央(3列目)・左(2列目)・左(1列目)のおじゃま
    let mut user: Vec<&str> = vec!["###....###"; 9];
    user.extend(["###.######", "##.#######", "#.########"]);
    // 比べる局面: 一番上のおじゃまの穴が左(2列目)
    let mut side: Vec<&str> = vec!["###....###"; 9];
    side.extend(["##.#######", "##.#######"]);
    for (name, rows) in [("ユーザーの局面", user), ("一番上の穴が左", side)] {
        let board = Board::from_rows(&rows);
        println!("== {name}: 穴の列の積み過ぎ {}", shape::garbage_hole_overstack(&board));
        let mut worst = Vec::new();
        for game in 1..=40u64 {
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
            let first: Vec<Piece> = if game == 1 { "SILZJT".chars().map(|c| Piece::from_char(c).unwrap()).collect() } else { (0..6).map(|_| next_piece()).collect() };
            let mut root = Root { state: PlayerState::new(board), queue: first, ..Default::default() };
            // REN を撃ち始める直前(最初に消す手の前)の、2列目の積み過ぎ
            let mut at_fire = None;
            for _ in 0..12 {
                let options = SearchOptions { width: 256, max_depth: 6, speculate: false };
                let Some(r) = search(&root, &options, &weights, &rules, &|| false) else { break };
                let Some(c) = r.candidates.first() else { break };
                let before = root.state.board;
                let mut st = root.state;
                if st.apply(&c.mv.placement, &rules).lines > 0 {
                    at_fire = Some(col_over(&before));
                    if col_over(&before) > 0 && std::env::var("PROBE_DEBUG").is_ok() {
                        println!("    {game} 局: 撃つ前の盤面\n      {}", before.to_rows().join("\n      "));
                    }
                    break;
                }
                root.play(&c.mv.placement, &rules);
                root.push_piece(next_piece());
            }
            let over = at_fire.unwrap_or(0);
            if std::env::var("PROBE_DEBUG").is_ok() {
                let h: Vec<u32> = (0..10).map(|x| root.state.board.column_height(x)).collect();
                println!("  {game:>2}: 撃った {}、撃つ前の2列目の積み過ぎ {over}、高さ {h:?}", at_fire.is_some());
            }
            worst.push(over);
        }
        println!("  積み過ぎ0の局 {}/{}", worst.iter().filter(|&&v| v == 0).count(), worst.len());
    }
}

/// 2列目(x=2)が、0・1列目の高い方より高い段数
fn col_over(b: &Board) -> u32 {
    b.column_height(2).saturating_sub(b.column_height(0).max(b.column_height(1)))
}

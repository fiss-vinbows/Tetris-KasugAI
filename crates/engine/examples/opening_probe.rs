//! 序盤の作戦(なし・素の状態・テンプレ)ごとに、空の盤面から読み筋を置いて盤面を表示する(調べるための道具)。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example opening_probe -- <重みの TOML>

use tetrisai_core::{Piece, Rules};
use tetrisai_engine::opening::td_templates;
use tetrisai_engine::{search, shape, Opening, Root, SearchOptions, Weights};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "config/weights/ren_tuned_run8.toml".to_string());
    let weights: Weights = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let rules = Rules::ppt2();
    let queues = [
        "LJOSZTI", "TIOLJSZ", "JZITOLS", "SZOITLJ", "IOTLJSZ", "OSZJLIT",
    ];
    for q in queues {
        let queue: Vec<Piece> = q.chars().map(|c| Piece::from_char(c).unwrap()).collect();
        let buildable: Vec<&str> =
            td_templates().iter().filter(|t| t.buildable(&queue[..6], None, &rules)).map(|t| t.name.as_str()).collect();
        let mut openings = vec![("なし", Opening::None), ("素", Opening::Plain)];
        if let Some(t) = td_templates().iter().find(|t| t.buildable(&queue[..6], None, &rules)) {
            openings.push(("テンプレ", Opening::Template(t.clone())));
        }
        println!("== ミノ順 {q}(組めるテンプレ: {buildable:?})");
        for (name, opening) in openings {
            let root = Root { queue: queue.clone(), opening, ..Default::default() };
            let r = search(&root, &SearchOptions { width: 256, max_depth: 6, speculate: false }, &weights, &rules, &|| false)
                .unwrap();
            let mut state = root.state;
            let mut on = Vec::new();
            for m in &r.plan {
                if let Opening::Template(t) = &root.opening {
                    on.push(t.accepts(&state.board, &m.placement.location));
                }
                state.apply(&m.placement, &rules);
            }
            if !on.is_empty() {
                let pieces: String = r.plan.iter().map(|m| format!("{:?}", m.placement.location.piece)).collect();
                println!("  図どおりか: {pieces} {on:?}");
            }
            println!(
                "  {name}: 種{} 中あけ{}行\n    {}",
                shape::center_well_seed(&state.board),
                shape::center_well_rows(&state.board),
                state.board.to_rows().join("\n    ")
            );
        }
    }
}

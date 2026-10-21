//! M5 の局面で、最終版と試験用の重みの読み筋を比べて表示する。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example m5_probe
use tetrisai_core::{Board, Piece, PlayerState, Rules, Spin};
use tetrisai_engine::{search, shape, Root, SearchOptions, Weights};

fn load(p: &str) -> Weights {
    toml::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn main() {
    let rules = Rules::ppt2();
    let cases: Vec<(&str, Board, Option<Piece>, Vec<Piece>)> = vec![
        (
            "空中TSD(1枚目)",
            Board::from_rows(&["..#....##.", ".##....###", "##...#####", "###.######", "###....###", "###....###"]),
            Some(Piece::J),
            vec![Piece::T, Piece::O, Piece::L, Piece::Z, Piece::S, Piece::I],
        ),
        (
            "低い中あけ",
            Board::from_rows(&["###....###", "###....###"]),
            None,
            vec![Piece::L, Piece::J, Piece::O, Piece::S, Piece::Z, Piece::T],
        ),
        (
            "低い偶数(パフェの見込み)",
            Board::from_rows(&["#####.....", "#####....."]),
            None,
            vec![Piece::T, Piece::I, Piece::O, Piece::L, Piece::J, Piece::S],
        ),
    ];
    for (name, board, hold, queue) in cases {
        let root = Root { state: PlayerState::new(board), hold, queue, ..Default::default() };
        for (wname, path) in [("最終版", "config/weights/tuned.toml"), ("試験用", "config/weights/experimental_m5.toml")] {
            let w = load(path);
            let options = SearchOptions { width: 256, max_depth: 6, speculate: false };
            let r = search(&root, &options, &w, &rules, &|| false).unwrap();
            let mut s = root.state;
            let mut steps = Vec::new();
            for m in &r.plan {
                let res = s.apply(&m.placement, &rules);
                let mut step = format!("{:?}", m.placement.location.piece);
                if res.spin != Spin::None {
                    step += "スピン";
                }
                if res.lines > 0 {
                    step += &format!("{}消", res.lines);
                }
                if res.perfect_clear {
                    step += "(パフェ)";
                }
                steps.push(step);
            }
            println!("{name} / {wname}: {}", steps.join(" → "));
            println!("    最後の盤面: 中あけ{}行、高さ{}", shape::center_well_rows(&s.board), s.board.max_height());
        }
    }
}

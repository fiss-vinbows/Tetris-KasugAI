//! C-spin の局面で、最終版と試験用の重みの読み筋を比べて表示する。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example cspin_probe
use tetrisai_core::{Board, Piece, PlayerState, Rules, Spin};
use tetrisai_engine::{search, Root, SearchOptions, Weights};

/// (局面の名前, 盤面の行(上から), HOLD, 操作ミノ + NEXT)
type Case<'a> = (&'a str, Vec<&'a str>, Option<Piece>, Vec<Piece>);

fn load(p: &str) -> Weights {
    toml::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn main() {
    let rules = Rules::ppt2();
    let cases: Vec<Case> = vec![
        (
            "C-spin 完成形",
            vec!["##........", "##..#..#..", "####...###", "####.#####", "####..####", "####.#####", "#####.####"],
            None,
            vec![Piece::T, Piece::T, Piece::O, Piece::I, Piece::L, Piece::S],
        ),
        (
            "屋根なし(J で屋根を作れる)",
            vec!["##........", "####...###", "####.#####", "####..####", "####.#####", "#####.####"],
            None,
            vec![Piece::J, Piece::T, Piece::O, Piece::T, Piece::I, Piece::L],
        ),
    ];
    for (name, rows, hold, queue) in cases {
        let root = Root { state: PlayerState::new(Board::from_rows(&rows)), hold, queue, ..Default::default() };
        for (wname, path) in [("最終版", "config/weights/tuned.toml"), ("試験用", "config/weights/experimental_cspin.toml")] {
            let w = load(path);
            let r = search(&root, &SearchOptions { width: 256, max_depth: 6, speculate: false }, &w, &rules, &|| false).unwrap();
            let mut s = root.state;
            let steps: Vec<String> = r
                .plan
                .iter()
                .map(|m| {
                    let res = s.apply(&m.placement, &rules);
                    let mut t = format!("{:?}", m.placement.location.piece);
                    if res.spin == Spin::Full {
                        t += "スピン";
                    }
                    if res.lines > 0 {
                        t += &format!("{}消", res.lines);
                    }
                    t
                })
                .collect();
            println!("{name} / {wname}: {}", steps.join(" → "));
        }
    }
}

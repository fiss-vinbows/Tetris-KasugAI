//! ユーザーが TBA のシミュレーターで作った局面(input/ の画像)での探索のテスト。

use tetrisai_core::{Board, Piece, PlayerState, Rules, Spin};
use tetrisai_engine::{search, Root, SearchOptions, SearchResult, Weights};

/// input/practice_20261019_214932_696f65.png: おじゃまの穴(1段目4列目・2段目5列目)を使った TST→TSD の例。
/// 操作ミノ I、HOLD T、NEXT S・T・L・T・S。
fn tst_position() -> Root {
    let board = Board::from_rows(&[
        "......ZOO.", // 8段目
        "..L..ZZOO.", // 7段目
        "LLL..ZLLLI", // 6段目
        "JJZ...LOOI", // 5段目
        "JZZSS.JOOI", // 4段目
        "JZSS..JJJI", // 3段目
        "GGGGG.GGGG", // 2段目(おじゃま)
        "GGGG.GGGGG", // 1段目(おじゃま)
    ]);
    Root {
        state: PlayerState::new(board),
        hold: Some(Piece::T),
        queue: vec![Piece::I, Piece::S, Piece::T, Piece::L, Piece::T, Piece::S],
        ..Default::default()
    }
}

fn tuned_weights() -> Weights {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/weights/tuned.toml");
    toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// 読み筋を1手ずつ置き、各手で消えた行数とTスピンの種類を返す。
fn plan_clears(root: &Root, r: &SearchResult) -> Vec<(Piece, Spin, u32)> {
    let rules = Rules::ppt2();
    let mut state = root.state;
    r.plan
        .iter()
        .map(|m| {
            let res = state.apply(&m.placement, &rules);
            (m.placement.location.piece, res.spin, res.lines)
        })
        .collect()
}

/// 手順は状況で変わり一つに決まらない(ユーザー)ので、手順は固定せず「読み筋に TST が入る」ことを確かめる。
#[test]
fn おじゃまの穴を使ったtstを読み筋に入れる() {
    let root = tst_position();
    for (name, weights) in [("既定", Weights::default()), ("最終版", tuned_weights())] {
        let options = SearchOptions { width: 512, max_depth: 6, speculate: false };
        let r = search(&root, &options, &weights, &Rules::ppt2(), &|| false).unwrap();
        let clears = plan_clears(&root, &r);
        println!("{name}: {clears:?}");
        assert!(
            clears.iter().any(|&(p, spin, lines)| p == Piece::T && spin == Spin::Full && lines == 3),
            "{name}の重みで TST が読み筋に入らない: {clears:?}"
        );
    }
}

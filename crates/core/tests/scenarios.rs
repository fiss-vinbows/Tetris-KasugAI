//! ユーザーが TBA のシミュレーターで作った局面(input/ の画像)を使ったテスト。

use tetrisai_core::{generate, Board, Piece, PlayerState, Rules, Spin};

/// input/practice_20261019_214345_5397d0.png: 中あけの上(空中)で TSD を撃つ。
/// ソフトドロップで下ろしてから回転入れで入れ、撃つと中央4列が再び開く。
#[test]
fn 空中tsdはソフトドロップ後の回転入れで入り中あけが再び開く() {
    let board = Board::from_rows(&[
        "..#....##.", // 6段目
        ".##....###", // 5段目
        "##...#####", // 4段目
        "###.######", // 3段目
        "###....###", // 2段目(中あけ)
        "###....###", // 1段目(中あけ)
    ]);
    let rules = Rules::ppt2();
    let moves = generate(&board, Piece::T, rules.spawn());

    // 4段目の左から3〜5列目と、3段目の4列目(下が y=0 なので y は段 − 1)
    let target = [(3, 2), (2, 3), (3, 3), (4, 3)];
    let tsd = moves
        .iter()
        .find(|m| m.placement.location.sorted_cells() == target && m.placement.spin == Spin::Full)
        .expect("空中TSDの入れ方が見つからない");
    assert!(tsd.soft_drop_sections >= 1, "ソフトドロップが必要: {tsd:?}");

    let mut state = PlayerState::new(board);
    let result = state.apply(&tsd.placement, &rules);
    assert_eq!((result.lines, result.spin, result.attack), (2, Spin::Full, 4), "TSD");
    assert_eq!(
        state.board.to_rows(),
        ["..#....##.", ".##....###", "###....###", "###....###"],
        "撃った後は中央4列が開いている"
    );
}

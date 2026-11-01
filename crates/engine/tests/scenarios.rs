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

/// 相殺外しの局面: こちらは低く(5段)TSD の溝があり、予告が2行(すぐせり上がる)来ている。
/// 操作ミノ T、NEXT O・I・L・J。`opponent_height` は相手の盤面の高さ(T溝やテトリスの井戸は無い)。
fn counter_position(opponent_height: usize) -> Root {
    let board = Board::from_rows(&[
        "##........",
        "#...######",
        "##.#######",
        "#########.",
        "#########.",
    ]);
    // 穴の列が行ごとにずれたおじゃまの山(攻撃の準備が無い盤面)
    let rows: Vec<String> = (0..opponent_height)
        .map(|i| (0..10).map(|x| if x == (i * 3) % 10 { '.' } else { '#' }).collect())
        .collect();
    let opponent = Board::from_rows(&rows.iter().map(|r| r.as_str()).collect::<Vec<_>>());
    Root {
        state: PlayerState::new(board),
        queue: vec![Piece::T, Piece::O, Piece::I, Piece::L, Piece::J],
        incoming: vec![tetrisai_engine::IncomingGarbage { lines: 2, delay: 0 }],
        opponent: Some(tetrisai_engine::Opponent { board: opponent, incoming: 0, gauge: 0, back_to_back: false }),
        ..Default::default()
    }
}

fn first_move_clears(root: &Root, weights: &Weights) -> (u32, Vec<(Piece, Spin, u32)>) {
    let options = SearchOptions { width: 256, max_depth: 6, speculate: false };
    let r = search(root, &options, weights, &Rules::ppt2(), &|| false).unwrap();
    let clears = plan_clears(root, &r);
    (clears[0].2, clears)
}

/// 相手が高い(17段、余裕3段)なら、すぐ TSD を撃つと2行が相殺に消えて届くのは2行(とどめに足りない)。
/// いったん受けてから TSD を撃てば4行届いてとどめになるので、すぐには撃たない(相殺を外してカウンターする)。
#[test]
fn 相手が高いときは相殺を外して受けてから撃つ() {
    for (name, weights) in [("既定", Weights::default()), ("最終版", tuned_weights())] {
        let root = counter_position(17);
        let (first_lines, clears) = first_move_clears(&root, &weights);
        println!("{name}(相手17段): {clears:?}");
        assert_eq!(first_lines, 0, "{name}: 1手目で消して相殺している: {clears:?}");
        assert!(
            clears.iter().any(|&(p, spin, lines)| p == Piece::T && spin == Spin::Full && lines == 2),
            "{name}: 読み筋に TSD が無い: {clears:?}"
        );
    }
}

/// 相手が低い(4段)ならとどめの見込みは無いので、すぐ TSD を撃って相殺する。
#[test]
fn 相手が低いときはすぐ撃って相殺する() {
    for (name, weights) in [("既定", Weights::default()), ("最終版", tuned_weights())] {
        let root = counter_position(4);
        let (first_lines, clears) = first_move_clears(&root, &weights);
        println!("{name}(相手4段): {clears:?}");
        assert_eq!(first_lines, 2, "{name}: すぐ TSD で相殺していない: {clears:?}");
    }
}

/// 相手の情報が無ければとどめの見込みも分からないので、相手が低いときと同じくすぐ撃って相殺する
/// (相殺を外す判断が、相手を見て決めたものであることの確認)。
#[test]
fn 相手の情報が無ければすぐ撃って相殺する() {
    for (name, weights) in [("既定", Weights::default()), ("最終版", tuned_weights())] {
        let root = Root { opponent: None, ..counter_position(17) };
        let (first_lines, clears) = first_move_clears(&root, &weights);
        println!("{name}(相手の情報なし): {clears:?}");
        assert_eq!(first_lines, 2, "{name}: すぐ TSD で相殺していない: {clears:?}");
    }
}

// ---------------- M5: 中あけ積み・空中TSD・中盤のパフェ(試験用の重み) ----------------

fn experimental_weights() -> Weights {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/weights/experimental_m5.toml");
    toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn run_plan(root: &Root, weights: &Weights) -> (Vec<(Piece, Spin, u32)>, PlayerState) {
    let options = SearchOptions { width: 256, max_depth: 6, speculate: false };
    let r = search(root, &options, weights, &Rules::ppt2(), &|| false).unwrap();
    let rules = Rules::ppt2();
    let mut state = root.state;
    let steps = r
        .plan
        .iter()
        .map(|m| {
            let res = state.apply(&m.placement, &rules);
            (m.placement.location.piece, res.spin, res.lines)
        })
        .collect();
    (steps, state)
}

/// input/practice_20261019_214345_5397d0.png: 中あけの上で空中TSD を撃ち、その後は左右を積み込んで中あけを保つ。
#[test]
fn 試験用の重みは空中tsdを撃って中あけを保つ() {
    let board = Board::from_rows(&["..#....##.", ".##....###", "##...#####", "###.######", "###....###", "###....###"]);
    let root = Root {
        state: PlayerState::new(board),
        hold: Some(Piece::J),
        queue: vec![Piece::T, Piece::O, Piece::L, Piece::Z, Piece::S, Piece::I],
        ..Default::default()
    };
    let (steps, end) = run_plan(&root, &experimental_weights());
    assert_eq!(steps[0], (Piece::T, Spin::Full, 2), "1手目で空中TSD: {steps:?}");
    assert!(tetrisai_engine::shape::center_well_rows(&end.board) >= 2, "中あけが残っていない: {steps:?}");
}

/// 盤面が低い(5段以下)ので中あけ積みを始め、中央を埋めずに左右を積み上げる。
#[test]
fn 試験用の重みは低い盤面で中あけを積む() {
    let root = Root {
        state: PlayerState::new(Board::from_rows(&["###....###", "###....###"])),
        queue: vec![Piece::L, Piece::J, Piece::O, Piece::S, Piece::Z, Piece::T],
        ..Default::default()
    };
    let (steps, end) = run_plan(&root, &experimental_weights());
    assert!(tetrisai_engine::shape::center_well_rows(&end.board) >= 2, "中あけを埋めた: {steps:?}");
    assert!(steps.iter().all(|&(_, _, lines)| lines == 0), "消さずに積み込む: {steps:?}");
}

/// 埋まりマスが偶数で低い盤面では、見えているミノでパフェを取る。
#[test]
fn 試験用の重みは低い偶数の盤面でパフェを取る() {
    let root = Root {
        state: PlayerState::new(Board::from_rows(&["#####.....", "#####....."])),
        queue: vec![Piece::T, Piece::I, Piece::O, Piece::L, Piece::J, Piece::S],
        ..Default::default()
    };
    let (steps, end) = run_plan(&root, &experimental_weights());
    assert!(end.board.is_empty(), "パフェを取っていない: {steps:?}");
}

// ---------------- 状況で選ぶ中あけ REN(2026-11-01。重み experimental_ren.toml) ----------------
//
// 【2026-11-01・ユーザー】「中あけREN専用のAIという概念自体が設計思想と合わない。状況を見て中あけRENが最善と
// みなす考え方が必要」。中あけ積みの切り替え(何段以下で積む・何段で撃つ)は取り除き、貯めた REN の火力
// (`ren_potential`)を、高さ・予告による危なさと同じ評価の中で比べる。同じ重みのまま、局面によって
// 貯める・撃つ・作らないが変わることを確かめる。

fn ren_weights() -> Weights {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/weights/experimental_ren.toml");
    toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn board_text(board: &Board) -> String {
    board.to_rows().join("
")
}

/// 種3(一番下)の上に、左右を埋めた中あけを `rows` 行積んだ盤面。
fn seed3_well(rows: usize) -> Board {
    let mut text = vec!["###....###"; rows];
    text.push("###.######");
    Board::from_rows(&text)
}

const REN_QUEUE: [Piece; 6] = [Piece::I, Piece::O, Piece::T, Piece::L, Piece::J, Piece::S];

/// 盤面に余裕があり予告も無いうちは、撃たずに種3を残して積む(貯めるほど REN の火力が増える)。
#[test]
fn 余裕があるうちは中あけに貯める() {
    let root = Root { state: PlayerState::new(seed3_well(7)), queue: REN_QUEUE.to_vec(), ..Default::default() };
    let (steps, end) = run_plan(&root, &ren_weights());
    assert!(steps.iter().all(|&(_, _, lines)| lines == 0), "撃ってしまった: {steps:?}");
    assert_eq!(tetrisai_engine::shape::center_well_seed(&end.board), 3, "{}", board_text(&end.board));
}

/// 高く積んで危なくなったら、中あけにミノを入れて REN を撃ち続ける(撃つ高さは決めていない)。
#[test]
fn 高くなったら中あけのrenを撃つ() {
    let root = Root { state: PlayerState::new(seed3_well(17)), queue: REN_QUEUE.to_vec(), ..Default::default() };
    let (steps, _) = run_plan(&root, &ren_weights());
    let ren = steps.iter().take_while(|&&(_, _, lines)| lines > 0).count();
    assert!(ren >= 5, "REN を撃ち続けていない(続いたのは {ren} 手): {steps:?}");
}

/// 貯めている途中でも、大きな予告が来たら REN を撃って相殺する(同じ盤面でも状況で判断が変わる)。
#[test]
fn 予告が来たら中あけのrenで相殺する() {
    let root = Root {
        state: PlayerState::new(seed3_well(7)),
        queue: REN_QUEUE.to_vec(),
        incoming: vec![tetrisai_engine::IncomingGarbage { lines: 10, delay: 0 }],
        ..Default::default()
    };
    let (steps, _) = run_plan(&root, &ren_weights());
    let ren = steps.iter().take_while(|&&(_, _, lines)| lines > 0).count();
    assert!(ren >= 3, "予告を受けても REN を撃たない(続いたのは {ren} 手): {steps:?}");
}

/// 中あけ以外に良い形(おじゃまの穴を使った TST)があれば、そちらを選ぶ(中あけ専用にならない)。
#[test]
fn 良い形があれば中あけにこだわらない() {
    let root = tst_position();
    let options = SearchOptions { width: 512, max_depth: 6, speculate: false };
    let r = search(&root, &options, &ren_weights(), &Rules::ppt2(), &|| false).unwrap();
    let clears = plan_clears(&root, &r);
    assert!(
        clears.iter().any(|&(p, spin, lines)| p == Piece::T && spin == Spin::Full && lines == 3),
        "TST が読み筋に入らない: {clears:?}"
    );
}

// ---------------- C-spin(暫定。試験用の重み) ----------------

fn cspin_weights() -> Weights {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/weights/experimental_cspin.toml");
    toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// C-spin の完成形(examples/cspin_search で見つけた、実際に SRS で入る形)では、TST → すぐ TSD と撃つ。
#[test]
fn 試験用の重みはc_spinでtstからすぐtsdを撃つ() {
    let board = Board::from_rows(&[
        "##........", "##..#..#..", "####...###", "####.#####", "####..####", "####.#####", "#####.####",
    ]);
    let root = Root {
        state: PlayerState::new(board),
        queue: vec![Piece::T, Piece::T, Piece::O, Piece::I, Piece::L, Piece::S],
        ..Default::default()
    };
    let (steps, _) = run_plan(&root, &cspin_weights());
    assert_eq!(steps[0], (Piece::T, Spin::Full, 3), "1手目で TST: {steps:?}");
    assert_eq!(steps[1], (Piece::T, Spin::Full, 2), "2手目で TSD: {steps:?}");
}

/// 屋根の無い C-spin の途中の形では、J で屋根を作って C-spin を組み上げ、TST を撃つ(組み上げる途中から目指す)。
#[test]
fn 試験用の重みは屋根を作ってc_spinを組み上げる() {
    let board = Board::from_rows(&["##........", "####...###", "####.#####", "####..####", "####.#####", "#####.####"]);
    let root = Root {
        state: PlayerState::new(board),
        queue: vec![Piece::J, Piece::T, Piece::O, Piece::T, Piece::I, Piece::L],
        ..Default::default()
    };
    let (steps, _) = run_plan(&root, &cspin_weights());
    assert!(steps.contains(&(Piece::T, Spin::Full, 3)), "TST を撃っていない: {steps:?}");
}

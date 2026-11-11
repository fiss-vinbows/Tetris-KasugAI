//! ユーザーが TBA のシミュレーターで作った局面(input/ の画像)での探索のテスト。

use tetrisai_core::{Board, Piece, PlayerState, Rules, Spin};
use tetrisai_engine::{search, Opening, Root, SearchOptions, SearchResult, Weights};

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

// ---------------- 序盤(2026-11-01。TD系テンプレか素の状態) ----------------
//
// 【2026-11-01・ユーザー】「いきなりRENを組むのは対策されると弱いので、序盤はTDテンプレか素の状態をランダムで選ぶ」。

fn run8_weights() -> Weights {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/weights/ren_tuned_run8.toml");
    toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn pieces(text: &str) -> Vec<Piece> {
    text.chars().map(|c| Piece::from_char(c).unwrap()).collect()
}

/// テンプレを選んだら、5種どれでも、読み筋の手がすべて図どおり(図のマスへ図と同じミノを置く)になる。
/// ミノ順は、見えている7個すべてを図どおりに置けるものを選ぶ。
/// 【2026-11-11】1巡目のソフトドロップの手は図どおりとみなさないので、ソフトドロップなしで組めるミノ順を選ぶ
#[test]
fn テンプレを選ぶと図どおりに組む() {
    use tetrisai_engine::opening::td_templates;
    let rules = Rules::ppt2();
    let queues = ["LJOSZTI", "TIOLJSZ", "JZITOLS", "SZOITLJ", "IOTLJSZ", "OSZJLIT", "JLSZOIT", "ZSLJTOI"];
    for template in td_templates() {
        let queue = queues
            .iter()
            .map(|q| pieces(q))
            .find(|q| template.buildable_hard_drop(q, None, &rules))
            .unwrap_or_else(|| panic!("{} をソフトドロップなしで組めるミノ順が無い", template.name));
        let root = Root { queue: queue.clone(), opening: Opening::Template(template.clone()), ..Default::default() };
        let options = SearchOptions { width: 256, max_depth: 6, speculate: false };
        let r = search(&root, &options, &run8_weights(), &rules, &|| false).unwrap();
        let mut board = root.state.board;
        for m in &r.plan {
            assert!(
                template.accepts(&board, &m.placement.location),
                "{} の図から外れた(ミノ順 {queue:?}): {:?}
{}",
                template.name,
                m.placement,
                board_text(&board)
            );
            board.place(&m.placement.location);
        }
    }
}

/// 中あけ REN を状況で選ぶ重み(run8)でも、素の状態の序盤では中あけを作らない。
/// (序盤の作戦が無いと、このミノ順では空の盤面からすぐ種3の中あけを積み始める)
#[test]
fn 素の状態の序盤では中あけを作らない() {
    for queue in ["SZOITLJ", "JZITOLS", "LJOSZTI"] {
        let root = Root { queue: pieces(queue), opening: Opening::Plain, ..Default::default() };
        let (steps, end) = run_plan(&root, &run8_weights());
        assert_eq!(tetrisai_engine::shape::ren_rows(&end.board), 0, "{queue}: {steps:?}\n{}", board_text(&end.board));
    }
}

// ---------------- 当初の構想(2026-11-03。重み concept.toml。勝率は度外視) ----------------
//
// 【2026-11-03・ユーザー】「シミュレーションの勝率を度外視してもいいので当初の構想を搭載したい」。

fn concept_weights() -> Weights {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/weights/concept.toml");
    toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// 空中TSD: C4W(中あけ REN)の上に S・Z が固まって来て綺麗に積めない時は、中あけの上に渡して空中TSDを組み、撃って中あけを戻す。
/// input/practice_20261019_214345_5397d0.png の局面(M5 の試験用の重みと同じ局面)。
#[test]
fn 構想_空中tsdを撃って中あけを保つ() {
    let board = Board::from_rows(&["..#....##.", ".##....###", "##...#####", "###.######", "###....###", "###....###"]);
    let root = Root {
        state: PlayerState::new(board),
        hold: Some(Piece::J),
        queue: vec![Piece::T, Piece::O, Piece::L, Piece::Z, Piece::S, Piece::I],
        ..Default::default()
    };
    let (steps, _) = run_plan(&root, &concept_weights());
    // HOLD の J を先に置いてから撃つこともある。撃った後は、戻った中あけで REN を始めてよい
    assert!(steps[..2].contains(&(Piece::T, Spin::Full, 2)), "2手目までに空中TSD を撃っていない: {steps:?}");
}

/// 空中TSD を組む: C4W を積む途中で S・Z が続けて来たら、中あけをふさがずに空中TSD の形を作る(中あけの上に渡す)。
/// 盤面は、S・Z・S・Z で空中TSD の形を作れるもの(examples/air_tsd_probe で総当たりして確かめた。左右が平らだと作れない)。
#[test]
fn 構想_szが固まったら空中tsdの形を作る() {
    let board = Board::from_rows(&["##.....###", "###....###", "###....###", "###....###", "###.######"]);
    let root = Root {
        state: PlayerState::new(board),
        queue: vec![Piece::S, Piece::Z, Piece::S, Piece::Z, Piece::T, Piece::O],
        ..Default::default()
    };
    let (steps, end) = run_plan(&root, &concept_weights());
    let air = tetrisai_engine::shape::has_air_tsd(&end.board) > 0
        || steps.iter().any(|&(p, spin, lines)| p == Piece::T && spin == Spin::Full && lines == 2);
    assert!(air, "空中TSD を組んでいない: {steps:?}\n{}", board_text(&end.board));
    assert!(tetrisai_engine::shape::ren_rows(&end.board) >= 2, "中あけをふさいだ: {steps:?}\n{}", board_text(&end.board));
}

/// 火力重視: おじゃまの穴を使った TST(2枚目の画像)を読み筋に入れる。
#[test]
fn 構想_おじゃまの穴を使ったtst() {
    let root = tst_position();
    let options = SearchOptions { width: 512, max_depth: 6, speculate: false };
    let r = search(&root, &options, &concept_weights(), &Rules::ppt2(), &|| false).unwrap();
    let clears = plan_clears(&root, &r);
    assert!(
        clears.iter().any(|&(p, spin, lines)| p == Piece::T && spin == Spin::Full && lines == 3),
        "TST が読み筋に入らない: {clears:?}"
    );
}

/// 火力重視: C-spin の形があれば TST → すぐ TSD と撃つ(TST の後の整地を TSD のソフトドロップだけで済ませる)。
#[test]
fn 構想_c_spinでtstからtsdを撃つ() {
    let board = Board::from_rows(&[
        "##........", "##..#..#..", "####...###", "####.#####", "####..####", "####.#####", "#####.####",
    ]);
    let root = Root {
        state: PlayerState::new(board),
        queue: vec![Piece::T, Piece::T, Piece::O, Piece::I, Piece::L, Piece::S],
        ..Default::default()
    };
    let (steps, _) = run_plan(&root, &concept_weights());
    assert_eq!(steps[0], (Piece::T, Spin::Full, 3), "1手目で TST: {steps:?}");
    // 【2026-11-03】Tスピン以外のソフトドロップを減点してから、間に O を1つ置いてから TSD を撃つこともある
    assert!(steps[1..3].contains(&(Piece::T, Spin::Full, 2)), "TST の後2手以内に TSD: {steps:?}");
}

/// 火力重視: 屋根の無い C-spin の途中の形から、屋根を作って C-spin を組み上げる。
#[test]
fn 構想_屋根を作ってc_spinを組む() {
    let board = Board::from_rows(&["##........", "####...###", "####.#####", "####..####", "####.#####", "#####.####"]);
    let root = Root {
        state: PlayerState::new(board),
        queue: vec![Piece::J, Piece::T, Piece::O, Piece::T, Piece::I, Piece::L],
        ..Default::default()
    };
    let (steps, _) = run_plan(&root, &concept_weights());
    assert!(steps.contains(&(Piece::T, Spin::Full, 3)), "TST を撃っていない: {steps:?}");
}

/// カウンター: 相手が高く、こちらに余裕があれば、相殺を外して受けてから撃つ。相手が低ければすぐ撃って相殺する。
#[test]
fn 構想_相手が高い時だけ相殺を外す() {
    let w = concept_weights();
    let (high, clears) = first_move_clears(&counter_position(17), &w);
    assert_eq!(high, 0, "相手17段: 1手目で消して相殺した: {clears:?}");
    let (low, clears) = first_move_clears(&counter_position(4), &w);
    assert_eq!(low, 2, "相手4段: すぐ TSD で相殺していない: {clears:?}");
    let (none, clears) = first_move_clears(&Root { opponent: None, ..counter_position(17) }, &w);
    assert_eq!(none, 2, "相手の情報なし: すぐ TSD で相殺していない: {clears:?}");
}

/// 相手が大きく育てた C4W(種3 + 中あけ8行)。
fn opponent_c4w() -> tetrisai_engine::Opponent {
    let mut rows = vec!["###....###"; 8];
    rows.push("###.######");
    tetrisai_engine::Opponent { board: Board::from_rows(&rows), incoming: 0, gauge: 0, back_to_back: false }
}

/// 作戦層「早撃ち」: 相手の C4W が大きく育っている時は、同じ形で追いかけず C4W を組まない
/// (作戦が無いと、このミノ順では空の盤面から種3の中あけを積み始める: 素の状態の序盤のテストと同じ)。
#[test]
fn 構想_早撃ちでは自分はc4wを組まない() {
    for queue in ["SZOITLJ", "JZITOLS"] {
        let root = Root {
            queue: pieces(queue),
            opponent: Some(opponent_c4w()),
            strategy: tetrisai_engine::Strategy::Rush,
            ..Default::default()
        };
        let (steps, end) = run_plan(&root, &concept_weights());
        assert_eq!(tetrisai_engine::shape::ren_rows(&end.board), 0, "{queue}: {steps:?}\n{}", board_text(&end.board));
    }
}

/// 作戦層「早撃ち」: Tスピンを撃てるなら、1手目から撃って相手をせり上げる(TSS + 次の2ライン消しを選ぶこともある)。
#[test]
fn 構想_早撃ちではtスピンをすぐ撃つ() {
    let root = Root {
        state: PlayerState::new(Board::from_rows(&["##........", "#...######", "##.#######"])),
        queue: vec![Piece::T, Piece::O, Piece::I, Piece::L, Piece::J],
        opponent: Some(opponent_c4w()),
        strategy: tetrisai_engine::Strategy::Rush,
        ..Default::default()
    };
    let (steps, _) = run_plan(&root, &concept_weights());
    let (piece, spin, lines) = steps[0];
    assert!(piece == Piece::T && spin == Spin::Full && lines >= 1, "Tスピンをすぐ撃っていない: {steps:?}");
}

/// 作戦層「C4W で対抗」: 自分の C4W も育っていれば、撃たずに貯めて対抗する。
#[test]
fn 構想_対抗ではc4wを貯める() {
    let root = Root {
        state: PlayerState::new(seed3_well(7)),
        queue: REN_QUEUE.to_vec(),
        opponent: Some(opponent_c4w()),
        strategy: tetrisai_engine::Strategy::C4WRival,
        ..Default::default()
    };
    let (steps, end) = run_plan(&root, &concept_weights());
    assert!(steps.iter().all(|&(_, _, lines)| lines == 0), "撃ってしまった: {steps:?}");
    assert_eq!(tetrisai_engine::shape::center_well_seed(&end.board), 3, "{}", board_text(&end.board));
}

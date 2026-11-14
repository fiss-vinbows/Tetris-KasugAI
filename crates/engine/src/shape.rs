//! 盤面の形の認識(M5): T溝(空中のものを含む)、中あけの行、中盤のパフェの見込み、TST の溝と C-spin。
//!
//! どれも行ごとの「空きマスの並び」(10ビット)だけで判定するので速い(評価関数の中で毎回呼べる)。

use tetrisai_core::board::WIDTH;
use tetrisai_core::{Board, Piece, PieceLocation, Rotation};

const ROW_FULL: u16 = (1 << WIDTH) - 1;
/// 中あけ4列(左から4〜7列目、x=3〜6)
const CENTER: u16 = 0b0001111000;
/// 中あけの左右(x=0〜2 と x=7〜9)
const SIDES: u16 = ROW_FULL & !CENTER;

/// y 行目の埋まりマス(x ビット目が立っていれば埋まり)。
#[inline]
fn row(board: &Board, y: u32) -> u16 {
    let mut r = 0u16;
    for (x, &c) in board.cols.iter().enumerate() {
        r |= ((c >> y & 1) as u16) << x;
    }
    r
}

/// TSD の溝(南向きの T を入れると2行消える形)の、T の中心 (x, y) の一覧。
///
/// 条件: y 行目の空きがちょうど x−1〜x+1 の3マス、y−1 行目の空きがちょうど x の1マス、
/// T の肩の上 (x−1, y+1) か (x+1, y+1) の少なくとも一方が埋まっている(屋根があるので回転で入れてTスピンになる)。
/// 盤面のどこにあってもよいので、中あけの上に渡したミノの上の溝(空中TSD)も見つかる。
/// 入れ方(屋根の下へ回転で届くか)までは確かめない。
pub fn tsd_slots(board: &Board) -> Vec<(i8, i8)> {
    let top = board.max_height();
    let mut slots = Vec::new();
    let mut below = row(board, 0);
    for y in 1..top {
        let here = row(board, y);
        let empty = !here & ROW_FULL;
        let empty_below = !below & ROW_FULL;
        if empty.count_ones() == 3 && empty_below.count_ones() == 1 {
            let x = empty_below.trailing_zeros() as i8;
            if x >= 1 && x <= WIDTH as i8 - 2 && empty == 0b111u16 << (x - 1) {
                let roof = board.occupied((x - 1, y as i8 + 1)) || board.occupied((x + 1, y as i8 + 1));
                if roof {
                    slots.push((x, y as i8));
                }
            }
        }
        below = here;
    }
    slots
}

/// TST の溝(縦向きの T を入れると3行消える形)の、T の置き場所の一覧。
///
/// 出っ張りが右の T(東向き、中心 (x, y))なら: y−1 行目と y+1 行目の空きがちょうど x の1マス、
/// y 行目の空きがちょうど x と x+1 の2マス。左右反転(西向き、出っ張りが左)も同じ。
/// T の軸の上 (x, y+2) が空いていること(入口)も条件にする。上下の行が x 以外埋まっているので、
/// 斜め4マスがすべて埋まり、回転で入れれば必ず Tスピン(TST)になる。入れ方(回転で届くか)までは確かめない。
/// CC2 譲りの `tslot` は横向きの T しか見ないので、TST の溝は見つけられない。
pub fn tst_slots(board: &Board) -> Vec<PieceLocation> {
    let top = board.max_height();
    let mut slots = Vec::new();
    for y in 1..top {
        let (lo, mid, hi) = (!row(board, y - 1) & ROW_FULL, !row(board, y) & ROW_FULL, !row(board, y + 1) & ROW_FULL);
        if lo.count_ones() != 1 || lo != hi || mid.count_ones() != 2 {
            continue;
        }
        let x = lo.trailing_zeros() as i8;
        if board.occupied((x, y as i8 + 2)) {
            continue; // 入口がふさがっている
        }
        for (dx, rotation) in [(1i8, Rotation::East), (-1, Rotation::West)] {
            let nub = x + dx;
            if (0..WIDTH as i8).contains(&nub) && mid == lo | 1 << nub {
                slots.push(PieceLocation::new(Piece::T, rotation, x, y as i8));
            }
        }
    }
    slots
}

/// C-spin の数: TST の溝に T を入れて3行消した後の盤面に、TSD の溝があるもの(TST → TSD につながる形)。
///
/// 型紙と照らし合わせるのではなく「TST を撃った後に TSD が続くか」で判定するので、左右反転や細かい形の違いも拾える。
pub fn c_spins(board: &Board) -> usize {
    tst_slots(board)
        .iter()
        .filter(|loc| {
            let mut after = *board;
            after.place(loc);
            after.clear_full_lines() == 3 && !tsd_slots(&after).is_empty()
        })
        .count()
}

/// 中あけの行(左右の3列ずつが埋まり、中央4列がすべて空いている行)の数。REN に使える行の目安。
pub fn center_well_rows(board: &Board) -> u32 {
    (0..board.max_height()).filter(|&y| row(board, y) == SIDES).count() as u32
}

/// 盤面の一番下から続く、9マス埋まった行(おじゃまの形の行)の数。せり上がったおじゃまの段数の目安。
pub fn garbage_rows(board: &Board) -> u32 {
    (0..board.max_height()).take_while(|&y| row(board, y).count_ones() == WIDTH as u32 - 1).count() as u32
}

/// 中あけの、一番下の中あけの行から上へ途切れずに続く行の数(壁が埋まりきっていない行で止める)。REN でつながる行の数。
///
/// 【2026-11-14・ユーザー「C4Wを積む際に穴をあけて積み上げたため途中でRENがつながらなくなることがある」】`center_well_rows` は
/// 穴のある壁の行より上の中あけの行も数えるので、穴をふさいで上に積むと点が増え、REN がその行で切れていた。
pub fn center_well_rows_connected(board: &Board) -> u32 {
    let height = board.max_height();
    let Some(start) = (0..height).find(|&y| row(board, y) == SIDES) else { return 0 };
    (start..height).take_while(|&y| row(board, y) == SIDES).count() as u32
}

/// 中あけの種(タネ)のマス数: 中央4列が空いた一番下の行より下にある、中央4列のマスの数。
///
/// 【2026-11-01・ユーザー】中あけ REN は中央4列の底に3マスの種(種3)を置いてから積む。種があると、中あけに
/// ミノを入れるたびに1行ずつ消えて REN が続く。種が無い(種0)と最初の1手で消えず、REN が始まらない。
/// 種は1行に収まらなくてもよい(例: L を寝かせて置くと、下の行に2マス・その上に1マス)。
/// 左右がまだ埋まっていなくても数える(対局の始めに、左右へまたがるミノで種を作るため。例: I を横にして
/// 右の列にかけると中央に3マス)。左右が埋まってからでは、中央のマス数の4で割った余りが変わらないので作れない。
pub fn center_well_seed(board: &Board) -> u32 {
    // 中あけがあれば、そのすぐ下だけを見る(下にせり上がったおじゃまを種に数えない。2026-11-05)
    if let Some(start) = (0..board.max_height()).find(|&y| row(board, y) == SIDES) {
        return seed_below(board, start).0;
    }
    let mut seed = 0;
    for y in 0..=board.max_height() {
        let center = row(board, y) & CENTER;
        if center == 0 {
            break;
        }
        seed += center.count_ones();
    }
    seed
}

/// `start` 行のすぐ下の、左右が埋まり中央が埋まりきっていない行(2行まで)の (中央のマス数, 行数)。
fn seed_below(board: &Board, start: u32) -> (u32, u32) {
    let (mut seed, mut rows) = (0, 0);
    let mut y = start;
    while y > 0 && rows < 2 {
        y -= 1;
        let r = row(board, y);
        let center = r & CENTER;
        if r & SIDES != SIDES || center == CENTER || center == 0 {
            break;
        }
        seed += center.count_ones();
        rows += 1;
    }
    (seed, rows)
}

/// 中あけ REN で続けて消せる行数の見込み。
///
/// 【2026-11-01・ユーザー】中あけ専用の作戦にせず、状況を見て中あけ REN が最善かを判断する。そのために、
/// 中あけに貯めた REN の火力を、TSD やテトリスと同じ物差し(攻撃の行数)で評価する(eval の `ren_potential`)。
///
/// 中央4列にマスがある一番下の行から上へ、左右6列が埋まった行を数える。下の2行までは種(中央の残りのマス、
/// 合わせて6マスまで)があってよく、その上は中央4列がすべて空いた行だけを数える(テトリスの井戸のような、
/// 中央が埋まりかけた行は中あけではない)。中央4列がすべて空いた行が無ければ 0。
///
/// 左右が埋まった中あけでは、中央のマス数を4で割った余りが変わらない(ミノは4マス入り、消える行も中央の
/// 4マスを消す)。余り3(種3)なら、ミノを入れるたびに1行ずつ消えて全部の行で REN が続く。ほかの余りでは
/// 途中で REN が切れやすいので、余り1・2は4分の3、余り0(種0)は半分に見積もる(大まかな目安)。
pub fn ren_rows(board: &Board) -> u32 {
    let height = board.max_height();
    // 【2026-11-05・ユーザー「せり上がると C4W を消化せず通常形に戻す」】中央に穴のあるおじゃまがせり上がると、それを種と数えて
    // 3行目で打ち切り、行数が0になっていた。中央4列がすべて空いた一番下の行から数え、種はそのすぐ下の2行までを見る
    let Some(start) = (0..height).find(|&y| row(board, y) == SIDES) else { return 0 };
    let mut empty_rows = 0;
    for y in start..height {
        if row(board, y) != SIDES {
            break;
        }
        empty_rows += 1;
    }
    let (seed, seed_rows) = seed_below(board, start);
    let rows = seed_rows + empty_rows;
    match seed % 4 {
        3 => rows,
        0 => rows / 2,
        _ => rows * 3 / 4,
    }
}

/// 中あけの下のおじゃまの穴の列を、同じ側のほかの列と中あけの行より高く積んだ段数。
///
/// 【2026-11-08・ユーザー「C4W を積み込んでいる時におじゃまがせり上がったら、穴がある列を高く積まないで。運が良ければ
/// せり上がった部分から REN がつながる」】REN で中あけの行を消し切ると、それより上に積んだブロックが落ちてきて穴をふさぐ。
/// 中あけの行より下で一番上のおじゃま(9マス埋まった行)の穴が左右の列(x=0〜2・7〜9)にある時だけ数える
/// (中央4列の穴は中あけの上から届くので数えない)。中あけの行が無ければ 0。
pub fn garbage_hole_overstack(board: &Board) -> u32 {
    let height = board.max_height();
    let Some(start) = (0..height).find(|&y| row(board, y) == SIDES) else { return 0 };
    let top = (start..height).take_while(|&y| row(board, y) == SIDES).last().map_or(start, |y| y + 1);
    let Some(hole_row) = (0..start).rev().map(|y| row(board, y)).find(|r| r.count_ones() == WIDTH as u32 - 1) else {
        return 0;
    };
    let hole = (!hole_row & ROW_FULL).trailing_zeros() as usize;
    let neighbors: [usize; 2] = match hole {
        0 => [1, 2],
        1 => [0, 2],
        2 => [0, 1],
        7 => [8, 9],
        8 => [7, 9],
        9 => [7, 8],
        _ => return 0,
    };
    let base = neighbors.iter().map(|&x| board.column_height(x)).max().unwrap().max(top);
    board.column_height(hole).saturating_sub(base)
}

/// 中あけの左右の高さの差(左3列の一番高いところと、右3列の一番高いところの差)。
pub fn center_well_side_diff(board: &Board) -> u32 {
    let left = (0..3).map(|x| board.column_height(x)).max().unwrap();
    let right = (7..WIDTH).map(|x| board.column_height(x)).max().unwrap();
    left.abs_diff(right)
}

/// 一番下の中あけの行より上に、TSD の溝があるか(空中TSD)。
pub fn has_air_tsd(board: &Board) -> usize {
    let Some(lowest_well) = (0..board.max_height()).find(|&y| row(board, y) == SIDES) else { return 0 };
    tsd_slots(board).iter().filter(|&&(_, y)| (y as u32) > lowest_well + 1).count()
}

/// テトリスと TSD のどちらでも撃てる形の数(2026-11-09・ユーザー「テトリスとTSDのどちらでも対応できる形を良形として」)。
///
/// 同じ列 x に、下から4行以上のテトリスの井戸(x だけ空いた行が続く)と、その上に T の軸が x にある TSD の溝があり、
/// x 列は井戸の底から上まで空いている(I を縦に落とせばテトリス、T を入れれば TSD)。どちらを先に撃っても、
/// もう片方は残る。例(下から): `#####.####` ×6、`####...###`、`#####.####`、`####...###`、`......####`。
pub fn tetris_tsd_flex(board: &Board) -> usize {
    tsd_slots(board)
        .into_iter()
        .filter(|&(x, y)| {
            let x = x as usize;
            // x 列の一番上のブロックの上が井戸の底(x 列はそこから上まで空いている)
            let bottom = board.column_height(x);
            let well_row = ROW_FULL & !(1u16 << x);
            // 井戸の4行が TSD の軸の行(y−1)以下にあり、どれも x だけ空いている
            bottom + 4 <= y as u32 && (bottom..bottom + 4).all(|r| row(board, r) == well_row)
        })
        .count()
}

/// 中盤のパフェの見込みがある形か。
///
/// パフェは「そろえる行の数 × 10 − 埋まりマス」がミノ4マスの倍数のときだけ取れる。10は偶数なので、
/// 埋まりマスが奇数ならどの高さでも取れない(ユーザーの方針: 奇数なら考えない)。
/// 偶数で、穴が無く、4段以内でそろえられる(4段まで積んで取れる)ときに true。
pub fn pc_possible(board: &Board) -> bool {
    let cells: u32 = board.cols.iter().map(|c| c.count_ones()).sum();
    if cells == 0 || cells % 2 == 1 {
        return false;
    }
    let h = board.max_height();
    if h > 4 {
        return false;
    }
    // 穴(上に埋まりマスがある空きマス)があれば取れない
    let holes: u32 = board.cols.iter().map(|&c| (!c & ((1u64 << (64 - c.leading_zeros())) - 1)).count_ones()).sum();
    if holes > 0 {
        return false;
    }
    (h.max(1)..=4).any(|rows| (rows * 10).checked_sub(cells).is_some_and(|rest| rest % 4 == 0))
}

/// 出現位置の列(中央4列: 3〜6列目)の一番高い高さ。ここが20段に届くとミノが出られず負ける(2026-11-09)。
pub fn spawn_height(board: &Board) -> u32 {
    (3..=6).map(|x| board.column_height(x)).max().unwrap_or(0)
}

/// 倒れるまでの近さ(20 に届くと負け): 出現位置の列の高さか、40段であふれるまでの近さの大きいほう。
/// C4W の盤面は一番高い列が15段でも、中あけの底が低ければまだ遠い(2026-11-09)。
pub fn spawn_danger(board: &Board) -> u32 {
    spawn_height(board).max(board.max_height().saturating_sub(20))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn テトリスとtsdの両対応の形() {
        // 2026-11-09 ユーザーの例(上から)
        let mut rows = vec!["......####", "####...###", "#####.####", "####...###"];
        rows.extend(["#####.####"; 6]);
        let b = Board::from_rows(&rows);
        assert_eq!(tetris_tsd_flex(&b), 2, "y6 と y8 の2つの溝");
        // 井戸が3行しかなければテトリスにならない
        let mut short = vec!["......####", "####...###", "#####.####", "####...###"];
        short.extend(["#####.####"; 3]);
        assert_eq!(tetris_tsd_flex(&Board::from_rows(&short)), 0);
        // 溝の軸と井戸の列がずれていれば数えない
        let mut apart = vec!["......####", "###...####", "####.#####"];
        apart.extend(["#####.####"; 4]);
        assert_eq!(tetris_tsd_flex(&Board::from_rows(&apart)), 0);
    }

    #[test]
    fn tstの溝() {
        // 出っ張りが右の TST の溝(x=4)
        let right = Board::from_rows(&["..........", "####.#####", "####..####", "####.#####"]);
        assert_eq!(tst_slots(&right), vec![PieceLocation::new(Piece::T, Rotation::East, 4, 1)]);
        // 左右反転(出っ張りが左)
        let left = Board::from_rows(&["####.#####", "###..#####", "####.#####"]);
        assert_eq!(tst_slots(&left), vec![PieceLocation::new(Piece::T, Rotation::West, 4, 1)]);
        // 入口(軸の上)がふさがっていれば溝ではない
        let covered = Board::from_rows(&["....#.....", "####.#####", "####..####", "####.#####"]);
        assert!(tst_slots(&covered).is_empty());
        // TST の後に TSD の溝が無いので C-spin ではない
        assert_eq!(c_spins(&right), 0);
    }

    /// examples/cspin_search で見つけた、実際に SRS で TST → TSD が入る C-spin の形(下が1段目)。
    /// 左に屋根(6段目の5列目)・右が開いた「コ」の字の空間に T を入れて TST、消えた後は入口だった5段目が
    /// 1段目の上に落ちて TSD の溝になる。
    pub(crate) fn c_spin_board() -> Board {
        Board::from_rows(&[
            "##........", // 7段目
            "##..#..#..", // 6段目(5列目が屋根)
            "####...###", // 5段目(TST の入口 → TST の後は TSD の上の行)
            "####.#####", // 4段目 ┐
            "####..####", // 3段目 ├ TST の溝(出っ張り右)
            "####.#####", // 2段目 ┘
            "#####.####", // 1段目(TSD の溝の底)
        ])
    }

    #[test]
    fn c_spinの形() {
        let board = c_spin_board();
        assert_eq!(tst_slots(&board), vec![PieceLocation::new(Piece::T, Rotation::East, 4, 2)]);
        assert_eq!(c_spins(&board), 1);
        // 底の空きが TST の出っ張りの反対側(4列目)だと、TST の後に TSD の溝にならない
        let mut wrong = board;
        wrong.cols[5] |= 1;
        wrong.cols[3] &= !1;
        assert_eq!(c_spins(&wrong), 0);
    }

    #[test]
    fn 普通のtsdの溝() {
        let board = Board::from_rows(&["##........", "#...######", "##.#######"]);
        assert_eq!(tsd_slots(&board), vec![(2, 1)]);
    }

    #[test]
    fn 屋根が無ければtスピンにならないので溝ではない() {
        let board = Board::from_rows(&["#...######", "##.#######"]);
        assert!(tsd_slots(&board).is_empty());
    }

    /// input/practice_20261019_214345_5397d0.png: 中あけの上(空中)の TSD の溝。
    fn air_board() -> Board {
        Board::from_rows(&[
            "..#....##.",
            ".##....###",
            "##...#####",
            "###.######",
            "###....###",
            "###....###",
        ])
    }

    #[test]
    fn 空中tsdの溝と中あけの行() {
        let board = air_board();
        assert_eq!(tsd_slots(&board), vec![(3, 3)]);
        assert_eq!(center_well_rows(&board), 2);
        assert_eq!(has_air_tsd(&board), 1);
        assert_eq!(center_well_side_diff(&board), 0, "左右とも6段");
    }

    #[test]
    fn 中あけrenで消せる行数() {
        // 種3(1行)+ 中あけ4行: 5行すべてで REN が続く
        let mut rows = vec!["###....###"; 4];
        rows.push("###.######");
        assert_eq!(ren_rows(&Board::from_rows(&rows)), 5);
        // 種3が2行にまたがる(下に2マス・上に1マス)
        assert_eq!(ren_rows(&Board::from_rows(&["###....###", "###....###", "###.#..###", "#####..###"])), 4);
        // 種0(中あけ4行だけ)は半分、種2は4分の3
        assert_eq!(ren_rows(&Board::from_rows(&["###....###"; 4])), 2);
        assert_eq!(ren_rows(&Board::from_rows(&["###....###", "###....###", "###....###", "###.##.###"])), 3);
        // 下に埋まった行があっても、中あけの部分から数える。左右が欠けた行より上は数えない
        assert_eq!(ren_rows(&Board::from_rows(&["##.....###", "###....###", "###.######", "#########."])), 2);
        // テトリスの井戸(中央の1列だけ空き)や平らな盤面は中あけではない
        assert_eq!(ren_rows(&Board::from_rows(&["####.#####"; 4])), 0);
        assert_eq!(ren_rows(&Board::from_rows(&["#########."; 4])), 0);
        assert_eq!(ren_rows(&Board::new()), 0);
        // 種だけで中あけの行が無い
        assert_eq!(ren_rows(&Board::from_rows(&["###.######"])), 0);
        // 空中TSD の盤面: 中あけ2行(種0)の上に TSD の形。中あけ2行の半分
        assert_eq!(ren_rows(&air_board()), 1);
    }

    #[test]
    fn おじゃまの穴の列を高く積んだ段数() {
        // 実機(debug_log_20261108_192000)の形: 右の7列目に穴のあるおじゃまの上に中あけ2行。7列目だけ3段高い
        let board = Board::from_rows(&[
            "...#..##..",
            "...#####..",
            "#####..###",
            "###....###",
            "###....###",
            "#..#######",
            "#######.##",
            "#######.##",
        ]);
        assert_eq!(garbage_hole_overstack(&board), 2, "8・9列目(高さ6)より2段高い");
        // 同じ側の列と同じ高さまでなら数えない
        let flat = Board::from_rows(&["#####..###", "###....###", "###....###", "#######.##"]);
        assert_eq!(garbage_hole_overstack(&flat), 0);
        // 穴が中央4列にあれば数えない(中あけの上から届く)
        let center = Board::from_rows(&["......#...", "###....###", "###....###", "#####.####"]);
        assert_eq!(garbage_hole_overstack(&center), 0);
        // 中あけの行が無ければ数えない
        assert_eq!(garbage_hole_overstack(&Board::from_rows(&["#######...", "#######.##"])), 0);
        // 左の穴: 0列目だけ高い
        let left = Board::from_rows(&["#.........", "#.........", "###....###", ".#########"]);
        assert_eq!(garbage_hole_overstack(&left), 2);
    }

    #[test]
    fn パフェの見込み() {
        // 埋まりマスが奇数(9)ならどの高さでも取れない
        assert!(!pc_possible(&Board::from_rows(&["#########."])), "9マスは奇数");
        // 2段で10マス埋まっている(残り10マス)のは4の倍数にならないが、4段まで積めば30マスで取れる
        assert!(pc_possible(&Board::from_rows(&["#####.....", "#####....."])));
        // 1段に8マス(残り2)は1段では取れないが、2段なら残り12 = ミノ3つで取れる
        assert!(pc_possible(&Board::from_rows(&["########.."])));
        // 穴があれば取れない
        assert!(!pc_possible(&Board::from_rows(&["##########", "##.#######"])));
        // 5段以上は考えない
        assert!(!pc_possible(&Board::from_rows(&["##........"; 5])));
        assert!(!pc_possible(&Board::new()), "空の盤面はパフェ済み");
    }

    #[test]
    fn 出現位置の列の高さ() {
        let mut rows = vec!["###....###"; 15];
        rows.push("###.######");
        let board = Board::from_rows(&rows);
        assert_eq!((spawn_height(&board), spawn_danger(&board), board.max_height()), (1, 1, 16));
        assert_eq!(spawn_height(&Board::from_rows(&["#.........", "##....####"])), 1);
    }

    #[test]
    fn 穴のある壁より上の中あけは数えない() {
        // 【2026-11-14・ユーザー「C4Wを積む際に穴をあけて積み上げたため途中でRENがつながらなくなることがある」】
        // 実機(debug_log_20261114_164308_2p の局17)の形: 右の壁に穴のある行の上に中あけを積んでいた
        let mut rows = vec!["###....###"; 2];
        rows.extend(["###......#", "###.....##"]);
        rows.extend(vec!["###....###"; 5]);
        let board = Board::from_rows(&rows);
        assert_eq!(center_well_rows(&board), 7, "今までの数え方は穴の上も数える");
        assert_eq!(center_well_rows_connected(&board), 5, "底から途切れずに続く行だけ");
        assert_eq!(center_well_rows_connected(&Board::from_rows(&["###....###"; 6])), 6);
        assert_eq!(center_well_rows_connected(&Board::new()), 0);
    }

    #[test]
    fn 下から続くおじゃまの行を数える() {
        let mut rows = vec!["###....###"; 3];
        rows.extend(vec!["####.#####"; 5]);
        rows.extend(vec!["#.########"; 3]);
        assert_eq!(garbage_rows(&Board::from_rows(&rows)), 8);
        assert_eq!(garbage_rows(&Board::from_rows(&["###....###", "#.########"])), 1);
        assert_eq!(garbage_rows(&Board::from_rows(&["####.#####", "###....###"])), 0, "一番下がおじゃまでなければ 0");
        assert_eq!(garbage_rows(&Board::new()), 0);
    }
}

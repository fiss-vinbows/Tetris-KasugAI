//! 実際に SRS で入る C-spin(TST → TSD)の盤面を総当たりで探す(局面テストの元にする)。
//!
//! TST を入れるには溝の真上の行に横3マスの入口が要り、その行は TST では消えずに残る。そのため TST の後の TSD は
//! 「入口だった行(空き3マス)が、TST の溝の下の行の上に落ちてくる」形になる: TSD の底(空き1マス)は TST より下、
//! 屋根は入口の上の出っ張り。左に屋根・右が開いた「コ」の字の空間で、鎹を逆向きにした形。
//!
//! 一番下の行は空きが1マス(x=3 か x=5)、その上3段を TST の溝(出っ張り右、x=4)に固定し、さらにその上の3段は
//! x=2〜9 の埋まり方をすべて試す(約3,300万通りを並列に調べる)。手の生成で「T を回転で入れて TST になり、
//! 消した後の盤面で実際に TSD が入る」ものを、空きマスの多い(自然な)順に表示する。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example cspin_search

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use tetrisai_core::{generate, Board, Piece, PlayerState, Rules, Spin};

const ABOVE: u64 = 3;
const BITS: u64 = 8; // x=2〜9

fn rows_of(code: u64) -> Option<Vec<String>> {
    let hole = if code >> (ABOVE * BITS) == 0 { 3 } else { 5 };
    let mut rows = Vec::new();
    for r in (0..ABOVE).rev() {
        let empties = (code >> (r * BITS)) & ((1 << BITS) - 1);
        let row: String = (0..10).map(|x| if x >= 2 && empties >> (x - 2) & 1 == 1 { '.' } else { '#' }).collect();
        if row == "##########" {
            return None;
        }
        rows.push(row);
    }
    for r in ["####.#####", "####..####", "####.#####"] {
        rows.push(r.to_string());
    }
    rows.push((0..10).map(|x| if x == hole { '.' } else { '#' }).collect());
    Some(rows)
}

fn main() {
    let rules = Rules::ppt2();
    let spawn = rules.spawn();
    let total = 2u64 << (ABOVE * BITS);
    let next = AtomicU64::new(0);
    let with_tst = AtomicUsize::new(0);
    let found = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let start = next.fetch_add(4096, Ordering::SeqCst);
                if start >= total {
                    break;
                }
                for code in start..(start + 4096).min(total) {
                    let Some(rows) = rows_of(code) else { continue };
                    let refs: Vec<&str> = rows.iter().map(|r| r.as_str()).collect();
                    let board = Board::from_rows(&refs);
                    // 安い判定で先に絞る: 形の上で C-spin でなければ調べない
                    if tetrisai_engine::shape::c_spins(&board) == 0 {
                        continue;
                    }
                    let clears = |b: Board, lines: u32| {
                        generate(&b, Piece::T, spawn).into_iter().find(|m| {
                            m.placement.spin == Spin::Full && {
                                let mut st = PlayerState::new(b);
                                st.apply(&m.placement, &rules).lines == lines
                            }
                        })
                    };
                    let Some(tst) = clears(board, 3) else { continue };
                    with_tst.fetch_add(1, Ordering::Relaxed);
                    let mut st = PlayerState::new(board);
                    st.apply(&tst.placement, &rules);
                    if let Some(tsd) = clears(st.board, 2) {
                        let empties = rows.iter().map(|r| r.matches('.').count()).sum::<usize>();
                        found.lock().unwrap().push((empties, rows.clone(), tst.soft_drop_sections, tsd.soft_drop_sections));
                    }
                }
            });
        }
    });
    let mut found = found.into_inner().unwrap();
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    println!("盤面 {total} 通り、形の上で C-spin かつ TST が入る {}、TST → TSD が入る {}", with_tst.load(Ordering::Relaxed), found.len());
    for (empties, rows, sd1, sd2) in found.iter().take(6) {
        println!("空き {empties} マス(ソフトドロップ区間: TST {sd1}・TSD {sd2})");
        for r in rows {
            println!("  {r}");
        }
    }
}

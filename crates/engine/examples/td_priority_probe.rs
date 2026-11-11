//! 優先テンプレ(はちみつ砲・くろみつ砲・迷走砲)を、1巡目をソフトドロップなしで狙える割合を数える。
//! 7種の並びすべてで、見えている6ミノ(操作ミノ + NEXT 5)と袋の残り1個から判定する(今の選び方)。
//!
//! 使い方: cargo run --release -p tetrisai-engine --example td_priority_probe

use tetrisai_core::{Piece, Rules};
use tetrisai_engine::opening::{priority_templates, td_templates};

fn main() {
    const ALL: [Piece; 7] = [Piece::I, Piece::O, Piece::T, Piece::S, Piece::Z, Piece::J, Piece::L];
    let rules = Rules::ppt2();
    let total = 5040;
    let p = priority_templates();
    // [優先テンプレごと..., ほかのTD, どれも無理]
    let mut counts = vec![0usize; p.len() + 2];
    let mut order = ALL;
    let mut c = [0usize; 7];
    let mut i = 0;
    // 7個目(開幕の袋の残り)も足して、1巡目をソフトドロップなしで組めるか(今の選び方)
    let mut check = |order: &[Piece; 7]| {
        let (queue, bag) = (&order[..6], &order[6..]);
        let k = match p.iter().position(|t| t.buildable_hard_drop_known(queue, None, bag, &rules)) {
            Some(k) => k,
            None if td_templates().iter().any(|t| t.buildable_hard_drop_known(queue, None, bag, &rules)) => p.len(),
            None => p.len() + 1,
        };
        counts[k] += 1;
    };
    check(&order);
    // 7種の並びすべて(7! = 5040通り。Heap のアルゴリズム)
    while i < 7 {
        if c[i] < i {
            if i % 2 == 0 {
                order.swap(0, i);
            } else {
                order.swap(c[i], i);
            }
            check(&order);
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }
    let pct = |n: usize| 100.0 * n as f64 / total as f64;
    for (k, t) in p.iter().enumerate() {
        println!("{:<10} {} ({:.1}%)", t.name, counts[k], pct(counts[k]));
    }
    println!("ほかのTD   {} ({:.1}%)", counts[p.len()], pct(counts[p.len()]));
    println!("どれも無理 {} ({:.1}%)", counts[p.len() + 1], pct(counts[p.len() + 1]));
}

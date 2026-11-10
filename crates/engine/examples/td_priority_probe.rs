//! はちみつ砲・迷走砲を、ソフトドロップなしで狙える割合を数える。
//! ランダムな7種1巡の順で、見えている6ミノ(操作ミノ + NEXT 5)から判定する。
//!
//! 使い方: cargo run --release -p tetrisai-engine --example td_priority_probe

use tetrisai_core::{Piece, Rules};
use tetrisai_engine::opening::{priority_templates, td_templates};

fn main() {
    const ALL: [Piece; 7] = [Piece::I, Piece::O, Piece::T, Piece::S, Piece::Z, Piece::J, Piece::L];
    let rules = Rules::ppt2();
    let total = 5040;
    let (mut honey, mut meisou, mut other, mut none) = (0, 0, 0, 0);
    // 7種の並びすべて(7! = 5040通り)を調べる
    let mut order = ALL;
    let mut c = [0usize; 7];
    let mut i = 0;
    let mut check = |order: &[Piece; 7]| {
        let queue = &order[..6];
        let p = priority_templates();
        if p[0].buildable_hard_drop(queue, None, &rules) {
            honey += 1;
        } else if p[1].buildable_hard_drop(queue, None, &rules) {
            meisou += 1;
        } else if td_templates().iter().any(|t| t.buildable(queue, None, &rules)) {
            other += 1;
        } else {
            none += 1;
        }
    };
    check(&order);
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
    let pct = |n: i32| 100.0 * n as f64 / total as f64;
    println!("はちみつ砲 {honey} ({:.1}%)", pct(honey));
    println!("迷走砲     {meisou} ({:.1}%)", pct(meisou));
    println!("ほかのTD   {other} ({:.1}%)", pct(other));
    println!("どれも無理 {none} ({:.1}%)", pct(none));
}

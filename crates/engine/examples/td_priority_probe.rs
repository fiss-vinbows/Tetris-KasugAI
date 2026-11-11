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
    // 【2026-11-11】7個目(開幕の袋の残り)も足して、1巡目をソフトドロップなしで組めるか(今の選び方)。見えている6個だけの判定で
    // 選ばれていたのに、7個目でソフトドロップが要る並びを数える
    let (mut was_honey, mut was_meisou, mut soft_honey, mut soft_meisou) = (0, 0, 0, 0);
    let mut examples: Vec<String> = Vec::new();
    let mut check = |order: &[Piece; 7]| {
        let queue = &order[..6];
        let bag = &order[6..];
        let p = priority_templates();
        let old0 = p[0].buildable_hard_drop(queue, None, &rules);
        let old1 = !old0 && p[1].buildable_hard_drop(queue, None, &rules);
        was_honey += old0 as usize;
        was_meisou += old1 as usize;
        if old0 && !p[0].buildable_hard_drop_known(queue, None, bag, &rules) {
            soft_honey += 1;
            if examples.len() < 3 { examples.push(format!("はちみつ砲 {order:?}")); }
        }
        if old1 && !p[1].buildable_hard_drop_known(queue, None, bag, &rules) {
            soft_meisou += 1;
            if examples.len() < 6 { examples.push(format!("迷走砲 {order:?}")); }
        }
        if p[0].buildable_hard_drop_known(queue, None, bag, &rules) {
            honey += 1;
        } else if p[1].buildable_hard_drop_known(queue, None, bag, &rules) {
            meisou += 1;
        } else if td_templates().iter().any(|t| t.buildable_hard_drop_known(queue, None, bag, &rules)) {
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
    println!(
        "見えている6個だけの判定で選んでいた: はちみつ砲 {was_honey}・迷走砲 {was_meisou} / そのうち7個目でソフトドロップが要る: はちみつ砲 {soft_honey}・迷走砲 {soft_meisou}"
    );
    for e in &examples {
        println!("  例: {e}");
    }
}

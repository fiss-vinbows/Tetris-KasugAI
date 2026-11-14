//! 評価関数。
//!
//! 局面の点数 = これまでの手で得た「報酬」の合計 + 最後の盤面の「評価」。
//! - 報酬(reward): その手で起きたこと(ライン消去の種類・B2B・REN・パフェ・Tの無駄使い・操作の難しさ)
//! - 評価(eval): 盤面の形の良し悪し(穴・高さ・凸凹・井戸・T溝・B2B を保っているか)
//!
//! 項目名と既定値は Cold Clear 2 の default.json に合わせてある。CC2 用の設定 JSON
//! (TBA の config/cold_clear_beginner.json など)の `freestyle_weights` をそのまま読める。
//! 独自の項目(`soft_drop_section` など)は CC2 の設定には無いので既定値を使う。
//!
//! 知らない項目はエラーにする(2026-11-01)。黙って無視すると、古い実行ファイルに新しい項目入りの重みを
//! 渡しても気づけない(TBA の bot が M5 より前のままで、中あけの重みが効いていなかった)。書き間違いも同じ。

use serde::{Deserialize, Serialize};
use tetrisai_core::board::WIDTH;

use crate::shape;
use crate::strategy::Strategy;
use tetrisai_core::{Board, ClearResult, Move, Piece, PieceLocation, Rotation, Rules, Spin};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Weights {
    // ---- 盤面の評価 ----
    /// 穴の上に被さっているマスの数(穴ごとに `max_cell_covered_height` まで数える)
    pub cell_coveredness: f32,
    pub max_cell_covered_height: u32,
    pub holes: f32,
    /// 横方向に「埋まり↔空き」が切り替わる回数(凸凹の目安)
    pub row_transitions: f32,
    pub height: f32,
    /// 10段を超えた分
    pub height_upper_half: f32,
    /// 15段を超えた分
    pub height_upper_quarter: f32,
    /// 一番低い列を井戸とみなしたときの、テトリスで消せる深さ
    pub tetris_well_depth: f32,
    /// T溝に T を入れたときに消える行数(0〜3)ごとの点
    pub tslot: [f32; 4],
    pub has_back_to_back: f32,

    // ---- 手の報酬 ----
    /// T を TSD/TST 以外で使った
    pub wasted_t: f32,
    /// CC2 の「ソフトドロップのマス数」への減点。こちらでは使わない(読み込めるように残している)
    pub softdrop: f32,
    /// 消した行数(0〜4)ごとの点(Tスピンでない場合)
    pub normal_clears: [f32; 5],
    pub mini_spin_clears: [f32; 3],
    pub spin_clears: [f32; 4],
    pub back_to_back_clear: f32,
    /// 続いていた B2B を普通の消去で切った手の点(2026-11-05・利用者「単発消しで B2B を切りすぎ」)
    pub back_to_back_broken: f32,
    /// REN の攻撃量(ルール設定の REN 表)1行あたりの点
    pub combo_attack: f32,
    pub perfect_clear: f32,
    /// パフェのときは消し方の点を数えない
    pub perfect_clear_override: bool,

    // ---- 独自の項目 ----
    /// ソフトドロップの区間数1つあたり(TBA の難度 ◎○△ と同じ数え方)
    pub soft_drop_section: f32,
    /// ソフトドロップを接地前に止める回数1つあたり
    pub mid_air_stop: f32,
    /// まだ残っている予告1行あたり
    pub pending_garbage: f32,
    /// 予告をすべて受けたときに20段を超える分(1段あたり)
    pub overflow: f32,

    // ---- 対戦の駆け引き(M4。相手の情報があるときだけ効く。`attack_sent` は常に効く) ----
    /// 相手に届く攻撃(相殺に使った分を除く)1行あたり
    pub attack_sent: f32,
    /// 相手に届く攻撃1行あたり × 相手の危なさ(相手の高さ + 相手への予告、20段で1)
    pub attack_sent_danger: f32,
    /// 届く攻撃(攻撃ゲージを含む)が相手の残りの余裕(20段 − 相手の危なさ)以上になる(とどめ)
    pub lethal: f32,
    /// 自分の高さ(予告を含む)+ 相手が次に撃てる攻撃の見積もり が20段を超える分(1段あたり)
    pub threat_overflow: f32,

    // ---- 中盤の作戦(M5。既定値はすべて0で効かない。試験用の重みは config/weights/experimental_m5.toml) ----
    //
    // 【2026-11-01・ユーザー】中あけ専用の作戦(「低ければ中あけ積み、何段で撃つ」の切り替え)はやめ、どの局面でも
    // 同じ評価で比べる。中あけを選ぶかどうか・いつ撃つかは、貯めた REN の火力(`ren_potential`)と、高さ・予告・
    // 相手の攻撃の準備による危なさ(既存の項目)のつり合いで決まる。
    /// 中あけの行(左右3列ずつが埋まり中央4列が空いた行)1行あたり
    pub center_well_rows: f32,
    /// 中あけの左右の高さの差1段あたり(中あけの行があるときだけ数える)
    pub center_well_side_diff: f32,
    /// 中あけの上(空中)に TSD の溝がある
    pub air_tsd_slot: f32,
    /// 手を探す時の出現位置を、実機よりこの行数だけ低く見る(出せない時は本来の位置)。
    /// 【2026-11-07・実機 debug_log_20261106_213612】列ずれ13手はすべて高い盤面で起きていた。壁の上すれすれを横切る手は、
    /// 入力の間に1段落ちたりせり上がったりすると壁に止められ、中央へ落ちて C4W が崩れる。読みの段階で通らない手として扱う。
    pub spawn_margin: u32,
    /// 中あけが残っている(`shape::ren_rows` の見積もりが2行以上)のに、続いていた REN を消さない手で切った
    /// (2026-11-07。少しだけ消して積み直すのを減らす)
    pub ren_cut: f32,
    /// 自分の高さ + 相手の C4W の火力が天井を超えた1行あたり(2026-11-07。相手の貯めた REN を受け切れる高さに保つ)
    pub ren_threat_overflow: f32,
    /// 序盤の作戦の途中でも、相手の C4W の行数がこの値以上なら序盤を切り上げる(0 なら切り上げない)
    pub pivot_opening_rows: u32,
    /// 相手の C4W への答え方を固定する。0: 育ち具合で選ぶ、1: 必ず C4W で対抗、2: 必ず早撃ち
    pub c4w_response: u32,
    /// 重ねた TSD の溝(ダブルダガーなど)の2つ目以降1つあたり。【2026-11-07・ユーザー】盤面が低い時は TSD のソフトドロップに
    /// 時間がかかるので、すぐ撃たずに溝をためて、相手の攻撃を受けてからカウンターする
    pub tsd_stock: f32,
    /// 予告が無く、盤面が `tsd_hold_height` 以下の時に TSD を撃った手
    pub tsd_low_fire: f32,
    /// 予告があるのに TSD・TST を撃った手(受けてからのカウンター)
    pub tsd_counter: f32,
    /// `tsd_low_fire` で「低い」とみなす高さ
    pub tsd_hold_height: u32,
    /// 中央4列の底に種3(中央4列の3マス)がある(2026-11-01。左右が埋まる前の、対局の始めにしか作れない。
    /// 中あけの火力はその4〜5手先まで見えないので、種を置いておく価値をここで付ける)
    pub center_well_seed: f32,
    /// 中あけに貯めた REN の火力(今の REN の続きから、`shape::ren_rows` の行数だけ REN を続けたときの、
    /// REN 表による攻撃の合計)1行あたり。実際に撃った攻撃(`combo_attack` など)と同じ物差しで比べる
    pub ren_potential: f32,
    /// TSD の溝(空中のものを含む、一般的な形)1つあたり(2つまで)。`tslot` は CC2 と同じ単純な形しか見ない
    pub tsd_slot_any: f32,
    /// 中盤のパフェの見込みがある形(埋まりマスが偶数・穴なし・4段以内でそろえられる)
    pub pc_chance: f32,
    /// テトリスと TSD のどちらでも撃てる形(`shape::tetris_tsd_flex`、2つまで)1つあたり。
    /// 【2026-11-09・ユーザー「テトリスとTSDのどちらでも対応できる形を良形として考えてほしい」】
    pub tetris_tsd_flex: f32,
    /// テンプレか開幕パフェでパフェまで撃った後、次のテンプレが組めなければ、最初に撃つまで TSD を狙う(C4W に向かわない)。
    /// 【2026-11-09・ユーザー「TDパフェを取った後にC4Wに向かっているのがよくない。火力の種を早々に作ることの方が大事」】
    pub after_pc_seed: bool,
    /// パフェまで撃った後は、その局の終わりまで「C4W で対抗」を使わない(相手が C4W を目指していても、ふつうの作戦)
    pub after_pc_no_rival: bool,
    /// TDテンプレの途中で攻撃されたら(速攻の相手)、T を待ちながら C4W の形も作り、TSD まで撃ったらテンプレを終える
    /// (`OpeningTracker::with_attacked_well`。2026-11-10・ユーザー)
    pub td_attacked_well: bool,
    /// テンプレで何回消すまでに攻撃されたら速攻とみなすか(1 = TST の後まで)
    pub td_attacked_clears: u32,
    /// ソフトドロップで落とした段数1段あたり(低い盤面での Tスピンは落とす時間が長く効率が悪い)
    pub soft_drop_rows: f32,
    /// Tスピン(TSS・TSD・TST)以外の手のソフトドロップの区間1つあたり(`soft_drop_section` に足す)。
    /// 【2026-11-03・ユーザー(実機の全自動)】「Tスピン以外のソフトドロップを控えられませんか」。全自動では落とす時間が
    /// そのまま遅れになり、S・Z・O・I の差し込みや回さずに横へ滑り込ませる T でも使っていた(1局 約9手、全体の約5%)
    pub soft_drop_non_spin: f32,
    /// 左右移動・回転のキー1回あたり(`Move::inputs`)。【2026-11-08・ユーザー「移動速度を早くする学習」】全自動ではキー1回が
    /// 約33ms(フレームに合わせて送る)かかるので、同じくらいの形なら少ないキーで置ける手を選ばせる。自動調整で決める
    #[serde(default)]
    pub input_keys: f32,

    // ---- C-spin(暫定。既定値は0で効かない。試験用の重みは config/weights/experimental_cspin.toml) ----
    /// TST の溝(縦向きの T を入れると3行消える形)1つあたり(2つまで)
    pub tst_slot: f32,
    /// C-spin(TST の溝に T を入れて3行消した後に TSD の溝ができる形)1つあたり(1つまで)。
    /// 名前の由来(鎹を逆向きにした形)どおり、TST → TSD を組む形を積極的に目指させる
    pub c_spin: f32,

    // ---- 序盤(2026-11-01。opening.rs) ----
    /// 序盤にテンプレを選んだとき、図どおりの手(図のマスへ図と同じミノを置く)1手あたり。
    /// 自動調整はしない(序盤の作戦はランダムに選ぶので、勝敗から重みを決めにくい)。8 にすると、テンプレを選んだ対局の
    /// 約84%でテンプレの Tスピンを撃てる(4 では約74%、12 でも8と同じ。設計書 8.10)
    pub template_piece: f32,
    /// 開幕TSD の序盤で TSD を撃つ手の加点(2026-11-05)
    pub opening_tsd: f32,
    /// true なら、テンプレの最初の消去までも図どおりの手に加点するだけにする(以前の動き。比べるため)。既定の false では、
    /// 図どおりの手があれば図に無い手を選ばない(2026-11-08・ユーザー「はちみつ砲の2巡目を早々にあきらめた。組み切って」)
    #[serde(default)]
    pub template_bonus_only: bool,
    /// 盤面が低い時(10段以下)の TST の溝の追加の点(2026-11-05・ユーザー「盤面が低い時は TST を多用したい」)
    #[serde(default)]
    pub tst_slot_low: f32,
    /// 中あけの下のおじゃまの穴の列(左右の列)を、同じ側のほかの列と中あけの行より高く積んだ1段あたり
    /// (`shape::garbage_hole_overstack`)。【2026-11-08・ユーザー「C4W を積み込んでいる時におじゃまがせり上がったら、
    /// 穴がある列を高く積まないで。運が良ければせり上がった部分から REN がつながる」】
    #[serde(default)]
    pub garbage_hole_overstack: f32,

    // ---- 作戦層(2026-11-03。strategy.rs。全自動で相手の情報があるときだけ効く。既定値は0) ----
    /// 「C4W で対抗」のとき、貯めた REN の火力1行あたりに足す点
    pub rival_ren: f32,
    /// 「C4W で対抗」のとき、相手より少ない中あけの行数1行あたり(2026-11-07・ユーザー「積み込み量が不足して押し切られる」)
    pub rival_deficit: f32,
    /// 「C4W で対抗」のとき、相手より少ない行数のまま撃ち始めた手(盤面が `rival_fire_height` を超えていれば減点しない)
    pub rival_early_fire: f32,
    /// `rival_early_fire` を使う高さの上限
    pub rival_fire_height: u32,
    /// 「早撃ち」のとき、相手に届く攻撃1行あたりに足す点(C4W は組まない)
    pub rush_attack: f32,

    // ---- 早撃ちの詰め(2026-11-09・ユーザー「相手が開幕 C4W の時の対策を、C4W で対抗する以外の方法で」) ----
    // C4W の相手は、一番高い列(15段ほど)ではなく、中あけの底が出現位置(20段)に届いた時に倒れる(出現位置がふさがる)。
    // 実機でも C4W の盤面は29〜34段まで押し上げられて REN を続けていた(TBA の debug_log の「天井より上」)
    /// 相手が C4W のとき、相手の危なさを出現位置の列(中央4列)の高さで見る(`shape::spawn_height`)。false なら一番高い列
    pub c4w_spawn_danger: bool,
    /// 「早撃ち」のとき、相手に届く攻撃1行あたり × 相手の危なさ(出現位置の列の高さ + 予告)/ 20
    pub rush_pressure: f32,
    /// 「早撃ち」のとき、1回にまとめて届く攻撃(相手の REN の1手分の相殺を引く)で相手の出現位置をふさげる手
    /// (2026-11-09 に、まとめて届く量と相手の相殺を見るように直した。小分けに送ると相手の REN の攻撃に1手ずつ打ち消される)
    pub rush_lethal: f32,
    /// 「早撃ち」のとき、自分の盤面の高さ(予告を上乗せ)1段あたり
    pub rush_height: f32,
    /// 「早撃ち」のとき、自分の高さ + 相手の C4W の火力が20段を超えた1段あたり
    pub rush_ren_overflow: f32,
    /// 「早撃ち」のとき、相手の出現位置の余裕が8段以下なら、1回にまとめて届く攻撃(相殺を引く)が余裕を埋める割合(0〜1)
    /// 【2026-11-09】C4W の相手を17〜19段まで押し上げても、あと数行が届かずに負けた局が約3割あった
    #[serde(default)]
    pub rush_finish: f32,
    /// 早撃ちの間だけ使う重み一式(C4W 相手専用。設定ファイルの [rush] の表)。無ければこの重みのまま
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rush: Option<Box<Weights>>,

    // ---- 迎え撃ち(2026-11-14・strategy.rs の `Strategy::Counter`。既定値は0) ----
    /// 「迎え撃ち」のとき、自分への予告を打ち消した攻撃1行あたり
    #[serde(default)]
    pub counter_cancel: f32,
    /// 「迎え撃ち」のとき、相手に届いた攻撃(打ち消しに使わなかった分)1行あたり(負なら、予告が来るまで撃たずにためる)
    #[serde(default)]
    pub counter_sent: f32,
    /// 迎え撃ちの間だけ使う重み一式(設定ファイルの [counter] の表)。あれば、相手の C4W に間に合わない時に早撃ちの代わりに使う
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter: Option<Box<Weights>>,
}

/// 盤面の評価に渡す、局面ごとの値。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvalContext {
    /// 今後使える T の目安(T溝の評価を何回まで重ねるか)
    pub t_available: usize,
    /// 予告の影響(高さに上乗せする)
    pub garbage: Garbage,
    /// この局面の REN の状態(続けて消した回数。0 なら REN 中でない)
    pub consecutive_clears: u32,
    /// 相手の中あけの行数(相手の情報が無ければ 0)
    pub opponent_rows: u32,
    /// 序盤(テンプレか素の状態)か。序盤は中あけ REN を組まない(中あけの項目を使わない)
    pub opening: bool,
    /// 作戦層の作戦(strategy.rs)
    pub strategy: Strategy,
    /// C4W を使わない(中あけの項目を使わない。`Root::no_c4w`。2026-11-13)
    pub no_c4w: bool,
}

/// 相手の情報から作る、評価に使う値(探索の最初に1回だけ計算する)。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BattleContext {
    /// 相手の危なさ(相手の盤面の高さ + 相手への予告の行数)。相手の情報が無ければ None
    pub opponent_danger: Option<u32>,
    /// 相手の出現位置の列(中央4列)の高さ + 相手への予告の行数(`shape::spawn_height`。2026-11-09)
    pub opponent_spawn_danger: Option<u32>,
    /// 相手が次に撃てる攻撃の見積もり(行数)
    pub opponent_threat: u32,
    /// 相手が貯めた C4W を撃った時の火力の見積もり(行数)
    pub opponent_ren: u32,
    /// 相手の中あけの行数
    pub opponent_rows: u32,
    /// 自分の攻撃ゲージ(REN 中にためている攻撃。次にラインを消さない手を置くと相手へ送られる)
    pub gauge: u32,
    /// 作戦層の作戦(strategy.rs)
    pub strategy: Strategy,
    /// 相手が REN 中なら、相手の次の1手が打ち消す行数の見積もり(`opponent_cancel`。2026-11-09)
    pub opponent_cancel: u32,
}

/// 相手の次の1手が、届いた攻撃を打ち消す行数の見積もり。相手の攻撃ゲージ(REN の攻撃の合計)から REN 数を逆算し、
/// 次の REN の攻撃を返す。ゲージが 0 なら 0(REN の始めは攻撃が 0〜1 行なので小さい)。
/// 【2026-11-09】相手の REN 中に届いた攻撃は、相手が消した手の攻撃でその分だけ先に打ち消され、残りがせり上がる
pub fn opponent_cancel(rules: &Rules, gauge: u32) -> u32 {
    if gauge == 0 {
        return 0;
    }
    let mut sum = 0;
    for ren in 0..40 {
        if sum >= gauge {
            return rules.combo_attack(ren);
        }
        sum += rules.combo_attack(ren);
    }
    rules.combo_attack(40)
}

/// 予告の影響(探索の中で計算する)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Garbage {
    /// 盤面の高さに上乗せする行数(せり上がったとみなした分 + まだ残っている予告)
    pub extra_height: u32,
    /// まだ残っている予告の行数
    pub pending: u32,
}

impl Default for Weights {
    fn default() -> Self {
        Weights {
            cell_coveredness: -0.2,
            max_cell_covered_height: 6,
            holes: -1.5,
            row_transitions: -0.2,
            height: -0.4,
            height_upper_half: -1.5,
            height_upper_quarter: -5.0,
            tetris_well_depth: 0.3,
            tslot: [0.1, 1.5, 2.0, 4.0],
            has_back_to_back: 0.5,
            wasted_t: -1.5,
            softdrop: -0.2,
            normal_clears: [0.0, -2.0, -1.5, -1.0, 3.5],
            mini_spin_clears: [0.0, -1.5, -1.0],
            spin_clears: [0.0, 1.0, 4.0, 6.0],
            back_to_back_clear: 1.0,
            back_to_back_broken: 0.0,
            combo_attack: 1.5,
            perfect_clear: 15.0,
            perfect_clear_override: true,
            soft_drop_section: -0.5,
            mid_air_stop: -1.5,
            pending_garbage: -0.1,
            overflow: -10.0,
            attack_sent: 0.0,
            attack_sent_danger: 0.5,
            lethal: 20.0,
            threat_overflow: -2.0,
            center_well_rows: 0.0,
            center_well_side_diff: 0.0,
            air_tsd_slot: 0.0,
            spawn_margin: 0,
            ren_cut: 0.0,
            ren_threat_overflow: 0.0,
            pivot_opening_rows: 0,
            c4w_response: 0,
            tsd_stock: 0.0,
            tsd_low_fire: 0.0,
            tsd_counter: 0.0,
            tsd_hold_height: 8,
            center_well_seed: 0.0,
            ren_potential: 0.0,
            tsd_slot_any: 0.0,
            pc_chance: 0.0,
            tetris_tsd_flex: 0.0,
            after_pc_seed: false,
            after_pc_no_rival: false,
            td_attacked_well: false,
            td_attacked_clears: 1,
            soft_drop_rows: 0.0,
            soft_drop_non_spin: 0.0,
            input_keys: 0.0,
            tst_slot: 0.0,
            c_spin: 0.0,
            template_piece: 8.0,
            opening_tsd: 12.0,
            template_bonus_only: false,
            tst_slot_low: 0.0,
            garbage_hole_overstack: -1.5,
            rival_ren: 0.0,
            rival_deficit: 0.0,
            rival_early_fire: 0.0,
            rival_fire_height: 15,
            rush_attack: 0.0,
            c4w_spawn_danger: false,
            rush_pressure: 0.0,
            rush_lethal: 0.0,
            rush_height: 0.0,
            rush_ren_overflow: 0.0,
            rush_finish: 0.0,
            rush: None,
            counter_cancel: 0.0,
            counter_sent: 0.0,
            counter: None,
        }
    }
}

/// 手の報酬。`mv` は置いた手(操作の難しさを含む)、`result` はその結果。
pub fn reward(w: &Weights, rules: &Rules, mv: &Move, result: &ClearResult) -> f32 {
    let mut r = 0.0;
    if result.perfect_clear {
        r += w.perfect_clear;
    }
    if !result.perfect_clear || !w.perfect_clear_override {
        if result.back_to_back {
            r += w.back_to_back_clear;
        }
        if result.back_to_back_broken {
            r += w.back_to_back_broken;
        }
        let lines = result.lines as usize;
        r += match result.spin {
            Spin::None => w.normal_clears[lines.min(4)],
            Spin::Mini => w.mini_spin_clears[lines.min(2)],
            Spin::Full => w.spin_clears[lines.min(3)],
        };
        if result.lines > 0 {
            r += w.combo_attack * rules.combo_attack(result.ren) as f32;
        }
    }
    if mv.placement.location.piece == Piece::T && (result.lines < 2 || result.spin != Spin::Full) {
        r += w.wasted_t;
    }
    r += w.soft_drop_section * mv.soft_drop_sections as f32;
    if result.spin != Spin::Full {
        r += w.soft_drop_non_spin * mv.soft_drop_sections as f32;
    }
    r += w.mid_air_stop * mv.mid_air_stops as f32;
    r += w.input_keys * mv.inputs as f32;
    if mv.soft_drop_sections > 0 {
        // ソフトドロップで落とした段数の目安: 出現位置の高さから置いた位置まで
        let rows = (rules.spawn().y - mv.placement.location.y).max(0);
        r += w.soft_drop_rows * rows as f32;
    }
    r
}

/// 盤面の評価。
pub fn evaluate(w: &Weights, rules: &Rules, board: &Board, back_to_back: bool, ctx: &EvalContext) -> f32 {
    let (t_available, garbage) = (ctx.t_available, ctx.garbage);
    let mut e = 0.0;
    if back_to_back {
        e += w.has_back_to_back;
    }

    // 中盤の作戦(M5)
    if w.tsd_slot_any != 0.0 {
        e += w.tsd_slot_any * shape::tsd_slots(board).len().min(2) as f32;
    }
    if w.tetris_tsd_flex != 0.0 {
        e += w.tetris_tsd_flex * shape::tetris_tsd_flex(board).min(2) as f32;
    }
    // 【2026-11-01・ユーザー】いきなり REN を組むのは対策されると弱いので、序盤は中あけの項目を使わない
    // 【2026-11-03・ユーザー】作戦層の「早撃ち」では C4W を組まない
    // 【2026-11-13・ユーザー「C4Wを禁止するモードを設けたい」】C4W を使わないモードでも組まない
    if !ctx.opening && !ctx.no_c4w && !ctx.strategy.avoids_c4w() {
        e += center_well_value(w, rules, board, ctx);
        if ctx.strategy == Strategy::C4WRival && w.rival_ren != 0.0 {
            e += w.rival_ren * ren_potential(rules, board, ctx.consecutive_clears) as f32;
        }
        if ctx.strategy == Strategy::C4WRival && w.rival_deficit != 0.0 {
            e += w.rival_deficit * ctx.opponent_rows.saturating_sub(shape::ren_rows(board)) as f32;
        }
    }
    if w.pc_chance != 0.0 && shape::pc_possible(board) {
        e += w.pc_chance;
    }
    if w.tst_slot != 0.0 {
        e += w.tst_slot * shape::tst_slots(board).len().min(2) as f32;
    }
    if w.tst_slot_low != 0.0 && board.max_height() <= 10 {
        e += w.tst_slot_low * shape::tst_slots(board).len().min(2) as f32;
    }
    if w.c_spin != 0.0 {
        e += w.c_spin * shape::c_spins(board).min(1) as f32;
    }
    if w.tsd_stock != 0.0 {
        e += w.tsd_stock * tsd_chain(board).saturating_sub(1) as f32;
    }

    // T溝: 見つかった溝に T を入れたと仮定し、消える行数で点を付ける(2行以上消えるなら消して次を探す)
    let mut b = *board;
    for _ in 0..t_available {
        let Some(loc) = tslot_left(&b).or_else(|| tslot_right(&b)) else { break };
        let mut filled = b;
        filled.place(&loc);
        let lines = filled.full_lines().count_ones() as usize;
        e += w.tslot[lines.min(3)];
        if lines > 1 {
            filled.remove_lines(filled.full_lines());
            b = filled;
        }
    }

    // 穴と、穴の上に被さったマス
    let mut holes = 0;
    let mut coveredness = 0;
    for &c in &board.cols {
        let height = 64 - c.leading_zeros();
        let mut h = !c & ((1u64 << height) - 1);
        holes += h.count_ones();
        while h != 0 {
            let y = h.trailing_zeros();
            coveredness += (height - y).min(w.max_cell_covered_height);
            h &= h - 1;
        }
    }
    e += w.holes * holes as f32;
    e += w.cell_coveredness * coveredness as f32;

    // テトリスの井戸の深さ
    let (well_x, well_h) = (0..WIDTH).map(|x| (x, board.column_height(x))).min_by_key(|&(_, h)| h).unwrap();
    let full_except_well =
        board.cols.iter().enumerate().filter(|&(x, _)| x != well_x).fold(!0u64, |acc, (_, &c)| acc & c);
    let depth = (full_except_well >> well_h).trailing_ones();
    e += w.tetris_well_depth * depth as f32;

    // 高さ(予告の分を上乗せ)
    e += w.pending_garbage * garbage.pending as f32;
    let top = board.max_height() + garbage.extra_height;
    if top > 20 {
        e += w.overflow * (top - 20) as f32;
    }
    e += w.height * top as f32;
    if top > 10 {
        e += w.height_upper_half * (top - 10) as f32;
    }
    if top > 15 {
        e += w.height_upper_quarter * (top - 15) as f32;
    }

    // 行方向の切り替わり(左右の壁は埋まり扱い)
    let full = (1u64 << tetrisai_core::board::HEIGHT) - 1;
    let mut transitions = (full ^ board.cols[0]).count_ones() + (full ^ board.cols[WIDTH - 1]).count_ones();
    for pair in board.cols.windows(2) {
        transitions += (pair[0] ^ pair[1]).count_ones();
    }
    e += w.row_transitions * transitions as f32;

    e
}

/// 中あけの項目(中あけの行・左右の差・空中TSD・種3・貯めた REN の火力)。
fn center_well_value(w: &Weights, rules: &Rules, board: &Board, ctx: &EvalContext) -> f32 {
    let mut e = 0.0;
    if w.center_well_rows != 0.0 || w.center_well_side_diff != 0.0 {
        let rows = shape::center_well_rows(board);
        if rows > 0 {
            e += w.center_well_rows * rows as f32;
            e += w.center_well_side_diff * shape::center_well_side_diff(board) as f32;
        }
    }
    if w.air_tsd_slot != 0.0 {
        e += w.air_tsd_slot * shape::has_air_tsd(board).min(1) as f32;
    }
    if w.center_well_seed != 0.0 && shape::center_well_seed(board) == 3 {
        e += w.center_well_seed;
    }
    if w.garbage_hole_overstack != 0.0 {
        e += w.garbage_hole_overstack * shape::garbage_hole_overstack(board) as f32;
    }
    if w.ren_potential != 0.0 {
        e += w.ren_potential * ren_potential(rules, board, ctx.consecutive_clears) as f32;
    }
    e
}

/// 対抗の時に、相手より少ない行数のまま撃ち始めた手の点。相手が撃ち始めて行数が減れば、減点は無くなる。
pub fn rival_fire_value(w: &Weights, strategy: Strategy, prior: u32, lines: u32, board_before: &Board, opponent_rows: u32) -> f32 {
    if w.rival_early_fire == 0.0 || strategy != Strategy::C4WRival || prior > 0 || lines == 0 {
        return 0.0;
    }
    if board_before.max_height() > w.rival_fire_height {
        return 0.0;
    }
    let rows = shape::ren_rows(board_before);
    if rows >= 2 && rows < opponent_rows {
        w.rival_early_fire
    } else {
        0.0
    }
}

/// TSD・TST を撃つタイミングの点。`height` は置く前の盤面の高さ(せり上がった分を含む)、`pending` は置く前の予告の行数。
pub fn tsd_fire_value(w: &Weights, result: &ClearResult, height: u32, pending: u32) -> f32 {
    if result.spin != Spin::Full || result.lines < 2 {
        return 0.0;
    }
    if pending > 0 {
        w.tsd_counter
    } else if result.lines == 2 && height <= w.tsd_hold_height {
        w.tsd_low_fire
    } else {
        0.0
    }
}

/// T を順に入れていった時に使える TSD の溝の数(3つまで)。
pub fn tsd_chain(board: &Board) -> u32 {
    let mut b = *board;
    for n in 0..3 {
        let Some(loc) = tslot_left(&b).or_else(|| tslot_right(&b)) else { return n };
        b.place(&loc);
        let full = b.full_lines();
        if full.count_ones() != 2 {
            return n;
        }
        b.remove_lines(full);
    }
    3
}

/// 中あけが残っている(`shape::ren_rows` の見積もりが2行以上)のに REN を切った手の点。`prior` はこの手の前に続いていた消去の回数、`board` は置いた後の盤面。
pub fn ren_cut_value(w: &Weights, prior: u32, lines: u32, board: &Board) -> f32 {
    if w.ren_cut == 0.0 || prior == 0 || lines > 0 || shape::ren_rows(board) < 2 {
        return 0.0;
    }
    w.ren_cut
}

/// 中あけに貯めた REN の火力(行数)。今の REN(`consecutive_clears` 回続けて消した)の続きから、
/// `shape::ren_rows` の行数だけ REN を続けたときの、REN 表による攻撃の合計。
pub fn ren_potential(rules: &Rules, board: &Board, consecutive_clears: u32) -> u32 {
    // 次に消す手の REN は consecutive_clears(PlayerState::apply と同じ数え方)
    (0..shape::ren_rows(board)).map(|k| rules.combo_attack(consecutive_clears + k)).sum()
}

/// 対戦の駆け引きの評価。`outgoing` はこの局面までに相手に届く攻撃(相殺に使った分を除く)、
/// `our_top` は自分の盤面の高さ(予告の分を上乗せしたもの)。
/// `chunk` はこの局面までに1回にまとめて届く攻撃の最大(REN 中にためてまとめて送る分。自分への予告の相殺を除く)。
/// `cancelled` はこの局面までに自分への予告を打ち消した攻撃の行数(迎え撃ちで使う)。
pub fn battle_value(w: &Weights, ctx: &BattleContext, outgoing: u32, chunk: u32, our_top: u32, cancelled: u32) -> f32 {
    let mut v = w.attack_sent * outgoing as f32;
    if ctx.strategy == Strategy::Counter {
        v += w.counter_cancel * cancelled as f32 + w.counter_sent * outgoing as f32;
    }
    if ctx.strategy == Strategy::Rush {
        v += w.rush_attack * outgoing as f32;
        v += w.rush_height * our_top as f32;
        if let Some(danger) = ctx.opponent_spawn_danger {
            v += w.rush_pressure * outgoing as f32 * (danger as f32 / 20.0);
            let margin = 20u32.saturating_sub(danger);
            let net = chunk.saturating_sub(ctx.opponent_cancel);
            if chunk > 0 && net >= margin {
                v += w.rush_lethal;
            }
            if margin <= 8 {
                v += w.rush_finish * net.min(margin) as f32 / margin.max(1) as f32;
            }
        }
        let ren_top = our_top + ctx.opponent_ren;
        if ren_top > 20 {
            v += w.rush_ren_overflow * (ren_top - 20) as f32;
        }
    }
    if let Some(danger) = ctx.opponent_danger {
        v += w.attack_sent_danger * outgoing as f32 * (danger as f32 / 20.0);
        let margin = 20u32.saturating_sub(danger);
        if outgoing > 0 && ctx.gauge + outgoing >= margin {
            v += w.lethal;
        }
        let threat_top = our_top + ctx.opponent_threat;
        if threat_top > 20 {
            v += w.threat_overflow * (threat_top - 20) as f32;
        }
        let ren_top = our_top + ctx.opponent_ren;
        if ren_top > 20 {
            v += w.ren_threat_overflow * (ren_top - 20) as f32;
        }
    }
    v
}

/// 相手が次に撃てる攻撃の見積もり(行数)。相手の盤面の T溝(TSS/TSD/TST)とテトリスの井戸のうち
/// 大きいほうに、相手の攻撃ゲージと B2B の加算を足す。
pub fn opponent_threat(rules: &Rules, board: &Board, gauge: u32, back_to_back: bool) -> u32 {
    let mut best = 0;
    if let Some(loc) = tslot_left(board).or_else(|| tslot_right(board)) {
        let mut filled = *board;
        filled.place(&loc);
        let lines = filled.full_lines().count_ones();
        best = best.max(rules.line_attack(lines, Spin::Full));
    }
    // テトリスの井戸: 一番低い列以外がそろっている深さが4以上
    let (well_x, well_h) = (0..WIDTH).map(|x| (x, board.column_height(x))).min_by_key(|&(_, h)| h).unwrap();
    let full_except_well =
        board.cols.iter().enumerate().filter(|&(x, _)| x != well_x).fold(!0u64, |acc, (_, &c)| acc & c);
    if (full_except_well >> well_h).trailing_ones() >= 4 {
        best = best.max(rules.line_attack(4, Spin::None));
    }
    if best > 0 && back_to_back {
        best += rules.attack.back_to_back_bonus;
    }
    best + gauge
}

/// 左側に屋根がある T溝(CC2 の well_known_tslot_left と同じ)。
fn tslot_left(board: &Board) -> Option<PieceLocation> {
    for x in 0..WIDTH - 2 {
        let y = board.column_height(x) as i8;
        if board.column_height(x + 1) as i8 >= y {
            continue;
        }
        let x = x as i8;
        if !board.occupied((x + 2, y - 1)) || board.occupied((x + 2, y)) || !board.occupied((x + 2, y + 1)) {
            continue;
        }
        return Some(PieceLocation::new(Piece::T, Rotation::South, x + 1, y));
    }
    None
}

/// 右側に屋根がある T溝(CC2 の well_known_tslot_right と同じ)。
fn tslot_right(board: &Board) -> Option<PieceLocation> {
    for x in 0..WIDTH - 2 {
        let y = board.column_height(x + 2) as i8;
        if board.column_height(x + 1) as i8 >= y {
            continue;
        }
        let x = x as i8;
        if !board.occupied((x, y - 1)) || board.occupied((x, y)) || !board.occupied((x, y + 1)) {
            continue;
        }
        return Some(PieceLocation::new(Piece::T, Rotation::South, x + 1, y));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules::ppt2()
    }

    #[test]
    fn 穴があると評価が下がる() {
        let w = Weights::default();
        let flat = Board::from_rows(&["#########."]);
        let hole = Board::from_rows(&["#########.", "####.#####"]);
        let ctx = EvalContext { t_available: 1, ..Default::default() };
        assert!(evaluate(&w, &rules(), &hole, false, &ctx) < evaluate(&w, &rules(), &flat, false, &ctx));
    }

    #[test]
    fn t溝を見つける() {
        // 左に屋根のある TSD の溝
        let board = Board::from_rows(&[
            "##........",
            "#...######",
            "##.#######",
        ]);
        let loc = tslot_left(&board).or_else(|| tslot_right(&board));
        assert!(loc.is_some());
        let w = Weights::default();
        let with = evaluate(&w, &rules(), &board, false, &EvalContext { t_available: 1, ..Default::default() });
        let without = evaluate(&w, &rules(), &board, false, &EvalContext::default());
        assert!((with - without - w.tslot[2]).abs() < 1e-5, "TSD の溝は tslot[2] の点");
    }

    #[test]
    fn 相手の攻撃の準備を見積もる() {
        let rules = Rules::ppt2();
        // TSD の溝がある盤面
        let tsd = Board::from_rows(&["##........", "#...######", "##.#######"]);
        assert_eq!(opponent_threat(&rules, &tsd, 0, false), 4);
        assert_eq!(opponent_threat(&rules, &tsd, 0, true), 5, "B2B なら +1");
        assert_eq!(opponent_threat(&rules, &tsd, 3, false), 7, "ゲージの分を足す");
        // 4段そろった井戸(テトリス)
        let well = Board::from_rows(&["#########."; 4]);
        assert_eq!(opponent_threat(&rules, &well, 0, false), 4);
        assert_eq!(opponent_threat(&rules, &Board::new(), 0, false), 0);
    }

    #[test]
    fn とどめと相手の危なさ() {
        let w = Weights::default();
        // 相手は17段(余裕3段)。4行届けばとどめ、2行では足りない
        let ctx = BattleContext { opponent_danger: Some(17), ..Default::default() };
        let lethal = battle_value(&w, &ctx, 4, 4, 5, 0);
        let not = battle_value(&w, &ctx, 2, 2, 5, 0);
        assert!(lethal - not > w.lethal * 0.9, "{lethal} {not}");
        // 相手の情報が無ければ、届く攻撃の点(既定 0)だけ
        let none = BattleContext::default();
        assert_eq!(battle_value(&w, &none, 4, 4, 5, 0), 0.0);
    }

    #[test]
    fn 迎え撃ちは打ち消した行と届いた行に点を付ける() {
        // 【2026-11-14・ユーザー「火力をため込んで相殺して迎え撃つ」】
        let w = Weights { counter_cancel: 2.0, counter_sent: -0.5, ..Weights::default() };
        let counter = BattleContext { strategy: Strategy::Counter, ..Default::default() };
        assert_eq!(battle_value(&w, &counter, 4, 4, 5, 3), 2.0 * 3.0 - 0.5 * 4.0);
        // ほかの作戦では使わない
        let normal = BattleContext::default();
        assert_eq!(battle_value(&w, &normal, 4, 4, 5, 3), 0.0);
        // 迎え撃ちの重み一式([counter] の表)を読める
        let text = "c4w_response = 0
[counter]
counter_cancel = 1.5
tsd_stock = 1.0
";
        let parsed: Weights = toml::from_str(text).unwrap();
        let table = parsed.counter.expect("[counter] の表");
        assert_eq!((table.counter_cancel, table.tsd_stock), (1.5, 1.0));
    }

    #[test]
    fn 迎え撃ちでは中あけを組まない() {
        let rules = Rules::ppt2();
        let well = Board::from_rows(&["###....###"; 6]);
        let w = Weights { center_well_rows: 5.0, ..Weights::default() };
        let eval = |strategy| evaluate(&w, &rules, &well, false, &EvalContext { strategy, ..Default::default() });
        assert!(eval(Strategy::Normal) > eval(Strategy::Counter), "通常は中あけに加点する");
        assert_eq!(eval(Strategy::Counter), eval(Strategy::Rush), "迎え撃ちは早撃ちと同じく中あけを見ない");
    }

    #[test]
    fn 早撃ちのとどめは相手の相殺を引く() {
        let rules = Rules::ppt2();
        assert_eq!(opponent_cancel(&rules, 0), 0);
        // ゲージ6 = 0+0+1+1+2+2 → 次は6REN目で3行
        assert_eq!(opponent_cancel(&rules, 6), 3);
        assert_eq!(opponent_cancel(&rules, 40), 5);
        let w = Weights { rush_lethal: 10.0, ..Weights::default() };
        // 相手は出現位置17段(余裕3段)で、次の1手で3行打ち消す
        let ctx = BattleContext {
            opponent_spawn_danger: Some(17),
            opponent_cancel: 3,
            strategy: Strategy::Rush,
            ..Default::default()
        };
        assert_eq!(battle_value(&w, &ctx, 6, 6, 0, 0), 10.0, "6行まとめてなら届く");
        assert_eq!(battle_value(&w, &ctx, 6, 4, 0, 0), 0.0, "4行ずつでは打ち消される");
    }

    #[test]
    fn c_spinの加点() {
        let w = Weights { tst_slot: 2.0, c_spin: 3.0, ..Weights::default() };
        let ctx = EvalContext::default();
        let gain = |b: &Board| evaluate(&w, &rules(), b, false, &ctx) - evaluate(&Weights::default(), &rules(), b, false, &ctx);
        // C-spin の形(shape.rs のテストと同じ盤面): TST の溝 +2 と C-spin +3
        let c_spin = Board::from_rows(&[
            "##........", "##..#..#..", "####...###", "####.#####", "####..####", "####.#####", "#####.####",
        ]);
        assert!((gain(&c_spin) - 5.0).abs() < 1e-4, "{}", gain(&c_spin));
        // TST の溝だけ(TST の後に TSD が続かない)なら +2
        let tst_only = Board::from_rows(&["####.#####", "####..####", "####.#####"]);
        assert!((gain(&tst_only) - 2.0).abs() < 1e-4, "{}", gain(&tst_only));
    }

    #[test]
    fn 新しい作戦の重みは既定では効かない() {
        // 中あけ・空中TSD・パフェの見込み・種3・貯めた REN の火力がそろった盤面でも、既定の重みでは点が変わらない
        let w = Weights::default();
        let board = Board::from_rows(&["##........", "###....###", "###....###", "###.######"]);
        let base = Weights {
            center_well_rows: 0.0,
            center_well_side_diff: 0.0,
            air_tsd_slot: 0.0,
            center_well_seed: 0.0,
            ren_potential: 0.0,
            tsd_slot_any: 0.0,
            pc_chance: 0.0,
            ..Weights::default()
        };
        let ctx = EvalContext { t_available: 1, ..Default::default() };
        assert_eq!(evaluate(&w, &rules(), &board, false, &ctx), evaluate(&base, &rules(), &board, false, &ctx));
    }

    #[test]
    fn 中あけの評価() {
        let w = Weights { center_well_rows: 1.0, center_well_side_diff: -0.5, ..Weights::default() };
        let ctx = EvalContext { t_available: 1, ..Default::default() };
        let gain = |rows: &[&str]| {
            let b = Board::from_rows(rows);
            evaluate(&w, &rules(), &b, false, &ctx) - evaluate(&Weights::default(), &rules(), &b, false, &ctx)
        };
        assert!((gain(&["###....###", "###....###"]) - 2.0).abs() < 1e-4, "中あけ2行で +2");
        // 左右の差は中あけの行があるときだけ数える
        assert!((gain(&["###.......", "###....###", "###....###"]) - 1.5).abs() < 1e-4, "左右の差1段で −0.5");
        assert_eq!(gain(&["###.......", "#########."]), 0.0, "中あけが無ければ左右の差も数えない");
    }

    #[test]
    fn c4wを使わないモードでは中あけを評価しない() {
        // 【2026-11-13・ユーザー「C4Wを禁止するモードを設けたい」】
        let w = Weights { center_well_rows: 1.0, center_well_seed: 4.0, ..Weights::default() };
        let ban = EvalContext { t_available: 1, no_c4w: true, ..Default::default() };
        let b = Board::from_rows(&["###....###", "###.######"]);
        assert_eq!(evaluate(&w, &rules(), &b, false, &ban), evaluate(&Weights::default(), &rules(), &b, false, &ban));
        let normal = EvalContext { t_available: 1, ..Default::default() };
        assert!(evaluate(&w, &rules(), &b, false, &normal) > evaluate(&Weights::default(), &rules(), &b, false, &normal));
    }

    #[test]
    fn 種3の加点() {
        let w = Weights { center_well_seed: 4.0, ..Weights::default() };
        let ctx = EvalContext { t_available: 1, ..Default::default() };
        let gain = |rows: &[&str]| {
            let b = Board::from_rows(rows);
            evaluate(&w, &rules(), &b, false, &ctx) - evaluate(&Weights::default(), &rules(), &b, false, &ctx)
        };
        assert!((gain(&["###....###", "###.######"]) - 4.0).abs() < 1e-4, "種3(1行)");
        assert!((gain(&["###....###", "###.#..###", "#####..###"]) - 4.0).abs() < 1e-4, "種3(2行にまたがる)");
        assert!((gain(&["...###...."]) - 4.0).abs() < 1e-4, "左右が埋まる前の種3(対局の始め)");
        assert!(gain(&["###....###", "###....###"]).abs() < 1e-4, "種0");
        assert!(gain(&["###....###", "###..#.###", "###.######"]).abs() < 1e-4, "種4");
    }

    #[test]
    fn 貯めたrenの火力() {
        let rules = rules();
        // 種3 + 中あけ5行 = 6行の REN。REN 表 [0,0,1,1,2,2,...] で 0+0+1+1+2+2 = 6
        let mut rows = vec!["###....###"; 5];
        rows.push("###.######");
        let board = Board::from_rows(&rows);
        assert_eq!(ren_potential(&rules, &board, 0), 6);
        // 2回続けて消した後(REN 中)なら、続きの REN で 1+1+2+2+3+3 = 12
        assert_eq!(ren_potential(&rules, &board, 2), 12);
        assert_eq!(ren_potential(&rules, &Board::from_rows(&["#########."; 4]), 0), 0, "テトリスの井戸は数えない");
        // 評価では1行あたり ren_potential の点
        let w = Weights { ren_potential: 0.5, ..Weights::default() };
        let ctx = EvalContext { t_available: 1, ..Default::default() };
        let gain = evaluate(&w, &rules, &board, false, &ctx) - evaluate(&Weights::default(), &rules, &board, false, &ctx);
        assert!((gain - 3.0).abs() < 1e-4, "{gain}");
        // 中あけを高く積むほど1行あたりの火力が増える(REN 表が伸びる)
        let mut tall = vec!["###....###"; 11];
        tall.push("###.######");
        assert_eq!(ren_potential(&rules, &Board::from_rows(&tall), 0), 6 + 3 + 3 + 4 + 4 + 5 + 5);
    }

    #[test]
    fn パフェの見込みの加点() {
        let w = Weights { pc_chance: 3.0, ..Weights::default() };
        let even = Board::from_rows(&["########.."]);
        let odd = Board::from_rows(&["#########."]);
        let ctx = EvalContext::default();
        let gain = |b: &Board| evaluate(&w, &rules(), b, false, &ctx) - evaluate(&Weights::default(), &rules(), b, false, &ctx);
        assert!((gain(&even) - 3.0).abs() < 1e-4);
        assert_eq!(gain(&odd), 0.0, "奇数は考えない");
    }

    #[test]
    fn 低い盤面のソフトドロップほど減点が大きい() {
        use tetrisai_core::{Placement, Rotation};
        let w = Weights { soft_drop_rows: -0.1, ..Weights::default() };
        let rules = Rules::ppt2();
        let mv = |y: i8| Move {
            placement: Placement { location: PieceLocation::new(Piece::T, Rotation::South, 4, y), spin: Spin::Full },
            mid_air_stops: 0,
            soft_drop_sections: 1,
            inputs: 2,
        };
        let result = ClearResult { lines: 2, spin: Spin::Full, ..Default::default() };
        let low = reward(&w, &rules, &mv(1), &result);
        let high = reward(&w, &rules, &mv(11), &result);
        assert!((high - low - 1.0).abs() < 1e-4, "10段の差で 0.1 × 10 = 1.0 の差: {low} {high}");
    }

    #[test]
    fn キーの数だけ点を付ける() {
        // 【2026-11-08・ユーザー「移動速度を早くする学習」】左右移動・回転のキー1回ごとに input_keys
        use tetrisai_core::{Placement, Rotation};
        let w = Weights { input_keys: -0.2, ..Weights::default() };
        let rules = Rules::ppt2();
        let mv = |inputs: u8| Move {
            placement: Placement { location: PieceLocation::new(Piece::L, Rotation::North, 4, 5), spin: Spin::None },
            mid_air_stops: 0,
            soft_drop_sections: 0,
            inputs,
        };
        let result = ClearResult::default();
        let few = reward(&w, &rules, &mv(1), &result);
        let many = reward(&w, &rules, &mv(5), &result);
        assert!((few - many - 0.8).abs() < 1e-4, "4キーの差で 0.2 × 4 = 0.8: {few} {many}");
    }

    #[test]
    fn tスピン以外のソフトドロップだけ減点する() {
        // 【2026-11-03・ユーザー】「Tスピン以外のソフトドロップを控えられませんか」
        use tetrisai_core::{Placement, Rotation};
        let w = Weights { soft_drop_non_spin: -3.0, ..Weights::default() };
        let rules = Rules::ppt2();
        let mv = |piece: Piece, spin: Spin, sections: u8| Move {
            placement: Placement { location: PieceLocation::new(piece, Rotation::North, 4, 5), spin },
            mid_air_stops: 0,
            soft_drop_sections: sections,
            inputs: 2,
        };
        let gain = |m: &Move, result: &ClearResult| reward(&w, &rules, m, result) - reward(&Weights::default(), &rules, m, result);
        let tsd = ClearResult { lines: 2, spin: Spin::Full, ..Default::default() };
        assert_eq!(gain(&mv(Piece::T, Spin::Full, 1), &tsd), 0.0, "Tスピンのソフトドロップは減点しない");
        let none = ClearResult::default();
        assert!((gain(&mv(Piece::S, Spin::None, 1), &none) + 3.0).abs() < 1e-4, "S の差し込み");
        assert!((gain(&mv(Piece::T, Spin::None, 2), &none) + 6.0).abs() < 1e-4, "回さずに滑り込ませる T(2区間)");
        assert!((gain(&mv(Piece::T, Spin::Mini, 1), &ClearResult { lines: 1, spin: Spin::Mini, ..Default::default() }) + 3.0).abs() < 1e-4, "Tスピンミニも減点する");
        assert_eq!(gain(&mv(Piece::O, Spin::None, 0), &none), 0.0, "ハードドロップだけの手");
    }

    #[test]
    fn cc2の設定jsonの重みを読める() {
        // TBA の config/cold_clear_beginner.json と同じ形(独自の項目は既定値になる)
        let json = r#"{"holes": -2.0, "tslot": [0.0, 1.0, 2.0, 3.0], "mini_spin_clears": [0.0, 0.0, 0.0]}"#;
        let w: Weights = serde_json::from_str(json).unwrap();
        assert_eq!(w.holes, -2.0);
        assert_eq!(w.tslot, [0.0, 1.0, 2.0, 3.0]);
        assert_eq!(w.soft_drop_section, Weights::default().soft_drop_section);
    }

    #[test]
    fn 知らない項目はエラーにする() {
        // 古い実行ファイルに新しい項目入りの重みを渡したとき・書き間違えたときに、黙って無視しない
        let err = toml::from_str::<Weights>("holes = -1.0\ncenter_wel_rows = 3.0\n").unwrap_err().to_string();
        assert!(err.contains("center_wel_rows"), "{err}");
        assert!(serde_json::from_str::<Weights>(r#"{"holes": -1.0, "jeopardy": 1.0}"#).is_err());
    }

    #[test]
    fn ren_cut_only_when_well_remains() {
        // REN が続いていて、消さず、中あけが2行以上残る時だけ減点する
        let w = Weights { ren_cut: -20.0, ..Weights::default() };
        let well = Board::from_rows(&["###....###", "###....###", "###.######"]);
        assert_eq!(ren_cut_value(&w, 2, 0, &well), -20.0);
        assert_eq!(ren_cut_value(&w, 0, 0, &well), 0.0, "REN が無ければ切っていない");
        assert_eq!(ren_cut_value(&w, 2, 1, &well), 0.0, "消した手は切っていない");
        assert_eq!(ren_cut_value(&w, 2, 0, &Board::from_rows(&["###.######"])), 0.0, "中あけが残っていなければ自然な終わり");
    }

    #[test]
    fn tsd_fire_prefers_after_being_attacked() {
        let w = Weights { tsd_low_fire: -6.0, tsd_counter: 3.0, tsd_hold_height: 8, ..Weights::default() };
        let tsd = ClearResult { spin: Spin::Full, lines: 2, ..Default::default() };
        assert_eq!(tsd_fire_value(&w, &tsd, 5, 0), -6.0, "低くて予告が無ければ、ためる");
        assert_eq!(tsd_fire_value(&w, &tsd, 12, 0), 0.0, "高ければ減点しない");
        assert_eq!(tsd_fire_value(&w, &tsd, 5, 4), 3.0, "予告があればカウンター");
        let tss = ClearResult { spin: Spin::Full, lines: 1, ..Default::default() };
        assert_eq!(tsd_fire_value(&w, &tss, 5, 0), 0.0);
    }

    #[test]
    fn tsd_chain_counts_stacked_slots() {
        assert_eq!(tsd_chain(&Board::default()), 0);
        // 左に屋根がある TSD の溝1つ(CC2 の形)
        let one = Board::from_rows(&["##........", "#...######", "##.#######"]);
        assert_eq!(tsd_chain(&one), 1);
    }

    #[test]
    fn 置いてある重みのファイルはすべて読める() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/weights");
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            // styles.toml は打ち方の名前 → 重みのファイルの表で、重みではない(2026-11-14)
            if path.extension().is_some_and(|e| e == "toml") && !path.ends_with("styles.toml") {
                let text = std::fs::read_to_string(&path).unwrap();
                toml::from_str::<Weights>(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                count += 1;
            }
        }
        assert!(count > 0);
    }
}

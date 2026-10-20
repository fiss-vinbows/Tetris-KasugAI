//! TBP の bot を子プロセスとして起動し、やり取りする。

use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};
use tetrisai_core::{Piece, Placement};

use crate::agent::BattleView;
use crate::battle::Player;

/// 盤面を TBP の形(y=0(最下段)から順の行の配列。埋まりは "G")にする。
fn board_rows(board: &tetrisai_core::Board) -> Vec<Vec<Option<&'static str>>> {
    (0..tetrisai_core::board::HEIGHT as i8)
        .map(|y| (0..tetrisai_core::board::WIDTH as i8).map(|x| board.occupied((x, y)).then_some("G")).collect())
        .collect()
}

pub struct TbpBot {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// bot が名乗った名前(info の name)
    pub name: String,
}

impl TbpBot {
    pub fn spawn(exe: &Path, args: &[String]) -> io::Result<TbpBot> {
        let mut child = Command::new(exe)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut bot = TbpBot { child, stdin, stdout, name: String::new() };
        let info = bot.recv()?;
        bot.name = info["name"].as_str().unwrap_or("?").to_string();
        bot.send(&json!({"type": "rules"}))?;
        let ready = bot.recv()?;
        if ready["type"] != "ready" {
            return Err(io::Error::other(format!("rules に ready が返らない: {ready}")));
        }
        Ok(bot)
    }

    fn send(&mut self, msg: &Value) -> io::Result<()> {
        writeln!(self.stdin, "{msg}")?;
        self.stdin.flush()
    }

    fn recv(&mut self) -> io::Result<Value> {
        let mut line = String::new();
        if self.stdout.read_line(&mut line)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "bot のプロセスが終了した"));
        }
        serde_json::from_str(&line).map_err(io::Error::other)
    }

    /// 局面を渡し直す(対局の最初と、おじゃまがせり上がった後)。
    /// `bag_state` は NEXT の後に今の袋に残っているミノ。これを渡すと Cold Clear 2 は
    /// 見えていない NEXT を予測して読む(渡さないと予測しない)。
    pub fn start(&mut self, p: &Player, bag_state: &[Piece]) -> io::Result<()> {
        let board = board_rows(&p.state.board);
        let queue: Vec<String> = p.queue.iter().map(|q| q.to_char().to_string()).collect();
        self.send(&json!({
            "type": "start",
            "board": board,
            "queue": queue,
            "hold": p.hold.map(|h| h.to_char().to_string()),
            "combo": p.state.consecutive_clears,
            "back_to_back": p.state.back_to_back,
            "randomizer": {
                "type": "seven_bag",
                "bag_state": bag_state.iter().map(|q| q.to_char().to_string()).collect::<Vec<_>>(),
            },
        }))
    }

    /// 前回の提案どおりに指したことと、新しく見えたミノを伝える。
    pub fn play(&mut self, placement: &Placement, new_pieces: &[Piece]) -> io::Result<()> {
        self.send(&json!({"type": "play", "move": placement}))?;
        for p in new_pieces {
            self.send(&json!({"type": "new_piece", "piece": p.to_char().to_string()}))?;
        }
        Ok(())
    }

    /// 対戦の状況(予告・攻撃ゲージ・相手)を伝える(このAIの独自の拡張。Cold Clear 2 は知らないメッセージとして無視する)。
    pub fn battle(&mut self, view: &BattleView) -> io::Result<()> {
        let incoming: Vec<Value> = view.incoming.iter().map(|g| json!({"lines": g.lines, "delay": g.delay})).collect();
        let o = view.opponent;
        self.send(&json!({
            "type": "battle",
            "incoming": incoming,
            "gauge": view.gauge,
            "opponent": {
                "board": board_rows(&o.state.board),
                "incoming": o.incoming_lines(),
                "gauge": o.gauge,
                "back_to_back": o.state.back_to_back,
            },
        }))
    }

    /// 今の最善手を聞く。まだ無ければ None。
    pub fn suggest(&mut self) -> io::Result<Option<Placement>> {
        self.send(&json!({"type": "suggest"}))?;
        let msg = self.recv()?;
        let Some(first) = msg["moves"].as_array().and_then(|m| m.first()) else { return Ok(None) };
        serde_json::from_value(first.clone()).map(Some).map_err(io::Error::other)
    }

    pub fn stop(&mut self) -> io::Result<()> {
        self.send(&json!({"type": "stop"}))
    }
}

impl Drop for TbpBot {
    fn drop(&mut self) {
        let _ = self.send(&json!({"type": "quit"}));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

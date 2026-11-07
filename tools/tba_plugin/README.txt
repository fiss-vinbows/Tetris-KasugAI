KasugAI プラグイン(TBA 用)

入れ方
1. この zip を展開します。
2. 中の kasugai フォルダを、TBA の plugins フォルダの中へ入れます。
   例: TBA\plugins\kasugai\plugin.py になるように置きます。
3. TBA を起動し直します。エンジンの切り替えに「KasugAI」が出ます。

外し方
plugins\kasugai フォルダを消して、TBA を起動し直します。

中身
plugin.py          TBA のプラグインの定義
tetrisai-bot.exe   KasugAI 本体(Tetris Bot Protocol 対応)
weights.toml       評価の重み

動作環境: Windows(64ビット)
対応している TBA: プラグイン機能のある版
このプラグインは思考エンジンだけを足します。全自動モードは入っていません。

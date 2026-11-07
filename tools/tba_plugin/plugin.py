"""KasugAI を TBA の思考エンジンに足すプラグイン。

このフォルダを TBA の plugins/ の下に置くと、エンジンの切り替えに「KasugAI」が出る。
"""

from pathlib import Path

from src.plugin_api import Engine, Plugin

_HERE = Path(__file__).resolve().parent

PLUGIN = Plugin(
    id="kasugai",
    name="KasugAI",
    description="自作AI KasugAI を思考エンジンに足します。",
    engines={
        "kasugai": Engine("KasugAI", _HERE / "tetrisai-bot.exe", _HERE / "weights.toml"),
    },
)

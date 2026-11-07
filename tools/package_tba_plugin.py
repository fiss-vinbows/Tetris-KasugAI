"""TBA のプラグインとして配る zip を作る。

使い方: python tools/package_tba_plugin.py <版> <重みの TOML>
例:     python tools/package_tba_plugin.py v0.1.0 config/weights/run11_cut20.toml

配布用に、C ランタイムを静的につなぎ、自分のフォルダの名前を exe に残さないようにビルドする。
"""

import os
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> None:
    version, weights = sys.argv[1], ROOT / sys.argv[2]
    home = Path.home()
    env = dict(os.environ)
    env["RUSTFLAGS"] = " ".join(
        [
            "-C target-feature=+crt-static",
            f"--remap-path-prefix={ROOT}=kasugai",
            f"--remap-path-prefix={home}=home",
        ]
    )
    env["CARGO_PROFILE_RELEASE_DEBUG"] = "0"
    env["CARGO_PROFILE_RELEASE_STRIP"] = "true"
    target = ROOT / "target" / "plugin_release"
    subprocess.run(
        ["cargo", "build", "--release", "-p", "tetrisai-bot", "--target-dir", str(target)], cwd=ROOT, env=env, check=True
    )

    out = ROOT / "target" / "plugin_release" / "dist"
    if out.exists():
        shutil.rmtree(out)
    folder = out / "kasugai"
    folder.mkdir(parents=True)
    shutil.copy(ROOT / "tools" / "tba_plugin" / "plugin.py", folder / "plugin.py")
    shutil.copy(target / "release" / "tetrisai-bot.exe", folder / "tetrisai-bot.exe")
    shutil.copy(weights, folder / "weights.toml")
    shutil.copy(ROOT / "tools" / "tba_plugin" / "README.txt", out / "README.txt")

    archive = ROOT / "target" / "plugin_release" / f"kasugai-tba-plugin-{version}.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as z:
        for path in sorted(out.rglob("*")):
            if path.is_file():
                z.write(path, path.relative_to(out))
    print(archive)


if __name__ == "__main__":
    main()

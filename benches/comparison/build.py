"""Build the Windows apps used by docs/pages/comparison.md."""

import argparse
import os
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
SDK = HERE.parents[1]
TARGET = SDK / "target"
OUT = TARGET / "comparison" / "apps"
TRIPLE = "x86_64-pc-windows-msvc"
RELEASE = OUT.parent / TRIPLE / "release"
CLI = SDK / "packages/cli/cli/sk.ts"


def run(*args, cwd=HERE, **env):
    subprocess.run(args, cwd=cwd, env=os.environ | env, check=True)


def publish(app, *args):
    run("bun.exe", CLI, "publish", app, "--payload-only", *args)
    return app / "dist/release/.sk"


def host(binary, *options, **env):
    run("cargo", "build", "--locked", "--release", "--bin", binary,
        "--target", TRIPLE, "--target-dir", OUT.parent, "--config",
        'target.x86_64-pc-windows-msvc.linker="rust-lld"', *options,
        cwd=HERE / "embedder", **env)
    return RELEASE / (binary + ".exe")


def stage(source, group):
    directory = OUT / group
    directory.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, directory / source.name)


def skathi(groups):
    app = HERE / "app"
    run("bun.exe", "install", "--frozen-lockfile", "--ignore-scripts", cwd=app)
    payload = publish(app, "--no-aot") / "app.skpk"
    if "jit" in groups:
        stage(host("compare-skathi"), "jit")
    if "standalone" in groups:
        run("cargo", "build", "--locked", "--release", "-p", "sk-app", "--bin", "sk-app",
            "--no-default-features", "--features", "prod,icu-root,no-devtools",
            "--target", TRIPLE, "--target-dir", OUT.parent, "--config",
            'target.x86_64-pc-windows-msvc.linker="rust-lld"', cwd=SDK)
        stage(RELEASE / "sk-app.exe", "standalone")
        stage(payload, "standalone")
    if "full" in groups:
        stage(host("compare-skathi", "--no-default-features", "--features", "full"), "full")
    if "native" in groups:
        native_app = HERE / "marshal/app"
        run("bun.exe", "install", "--frozen-lockfile", "--ignore-scripts", cwd=native_app)
        marshal = publish(native_app, "--no-aot")
        for name, flags in [("vec", []), ("slice", ["--features", "direct"])]:
            stage(host("marshal", *flags), name)
            stage(marshal / "app.skpk", name)
    if groups.intersection({"aot", "trimmed"}):
        tool = SDK / "tools/sk-aot"
        compiler = tool / "target" / TRIPLE / "release/sk-aot.exe"
        table = str(tool / ".cache/table")
        run("cargo", "build", "--locked", "--release", "--target", TRIPLE,
            "--target-dir", tool / "target", cwd=tool)
        image = TARGET / "sk-image"
        run("cargo", "build", "--locked", "--release", "--target", TRIPLE,
            "--target-dir", image, cwd=SDK / "crates/sk-image")
        RELEASE.mkdir(parents=True, exist_ok=True)
        shutil.copy2(image / TRIPLE / "release/skimage.dll", RELEASE)
        for variant in ("aot", "trimmed"):
            if variant not in groups:
                continue
            options = {"REDJS_AOT_BUILTIN_TABLE_DIR": table}
            if variant == "trimmed":
                archive = OUT.parent / "no-jit-no-wasm"
                run("bun.exe", SDK / "scripts/aot/no-jit.ts", "--out", archive,
                    "--remove", "JIT,WebAssembly", cwd=SDK)
                options["REDJS_NO_JIT_DIR"] = str(archive)
            probe = host("compare-skathi", "--features", variant, **options)
            package = publish(app, "--aot", "--workload", probe, "--aot-compiler", compiler)
            stage(host("compare-skathi", "--features", variant,
                       COMPARE_PAYLOAD=str(payload), **options), variant)
            stage(package / "app.image.dll", variant)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    names = ["jit", "standalone", "native", "full", "aot", "trimmed", "electron", "tauri"]
    parser.add_argument("groups", nargs="*", help="default: " + " ".join(names))
    parser.add_argument("--electron-zip-dir", type=Path,
                        help="directory containing the pinned Electron release ZIP")
    args = parser.parse_args()
    groups = set(args.groups or names)
    if groups.difference(names):
        parser.error("unknown build: " + ", ".join(groups.difference(names)))
    if groups.difference({"electron", "tauri"}):
        skathi(groups)
    if "electron" in groups:
        app = HERE / "electron"
        run("npm.cmd", "ci", "--no-audit", "--no-fund", cwd=app)
        run("npm.cmd", "run", "build", cwd=app)
        flags = ["--", f"--electron-zip-dir={args.electron_zip_dir.resolve()}"] if args.electron_zip_dir else []
        run("npm.cmd", "run", "package", *flags, cwd=app)
    if "tauri" in groups:
        app = HERE / "tauri"
        run("npm.cmd", "ci", "--no-audit", "--no-fund", cwd=app)
        target = TARGET / "compare-tauri"
        run("npm.cmd", "run", "tauri", "build", "--", "--target", TRIPLE, cwd=app,
            CARGO_TARGET_DIR=str(target), CARGO_ENCODED_RUSTFLAGS="")
        stage(target / TRIPLE / "release/tauri-app.exe", "tauri")


if __name__ == "__main__":
    main()

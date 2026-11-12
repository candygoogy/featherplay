#!/usr/bin/env python3
"""Writes THIRD_PARTY.md: notices.md, the font licences, then every crate in the four builds
and the licence texts they carry, as cargo-about finds them.

    python3 packaging/third-party/generate.py

Needs cargo-about (`cargo install cargo-about --locked`) and the crates downloaded
(`cargo fetch`): it runs offline, from the files in the crates.

cargo-about runs once per target and the runs are merged here. Given all the targets at once,
it keeps a dependency whose cfg matches any of them, so a Windows-only crate brings its Linux
dependencies along (gtk under tray-icon, OpenSSL under native-tls), though no build compiles
them.
"""

import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
FONTS = ROOT / "apps/featherplay/ui/fonts"

# The release targets, and how the Builds column names each.
TARGETS = {
    "x86_64-pc-windows-msvc": "Windows x64",
    "aarch64-pc-windows-msvc": "Windows ARM64",
    "x86_64-unknown-linux-gnu": "Linux x86_64",
    "aarch64-unknown-linux-gnu": "Linux aarch64",
}


def about(target):
    """cargo-about's JSON for one target, by way of target/third-party/ (on Windows it won't
    write to a pipe)."""
    out = ROOT / "target/third-party" / f"{target}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    run = subprocess.run(
        ["cargo", "about", "generate", "--offline", "--locked", "--workspace",
         "-c", str(HERE / "about.toml"), "--format", "json", "--target", target,
         "-o", str(out)],
        cwd=ROOT, check=True, stderr=subprocess.PIPE, encoding="utf-8", errors="replace",
    )
    sys.stderr.write(run.stderr)
    # A clarification whose file changed (a new version of the crate) is only a warning to
    # cargo-about, which then falls back to generic texts.
    if "clarification for crate" in run.stderr:
        sys.exit("generate: a clarification in about.toml no longer matches; update it")
    return json.loads(out.read_text(encoding="utf-8"))


def version_key(version):
    """Semver order: 0.9 before 0.10, a pre-release before its release."""
    core, _, pre = version.split("+")[0].partition("-")
    return tuple(int(n) for n in core.split(".")), pre == "", pre


def crate_key(crate):
    name, version = crate
    return name, version_key(version)


def builds(targets):
    """The Builds cell: "all", "Windows", "Linux", or the builds by name."""
    if len(targets) == len(TARGETS):
        return "all"
    for os_name in ("Windows", "Linux"):
        if targets == {t for t, label in TARGETS.items() if label.startswith(os_name)}:
            return os_name
    return ", ".join(label for t, label in TARGETS.items() if t in targets)


def merge(runs):
    """Crates as {(name, version): (licence, {targets})}, and the licence texts as a list of
    (name, text, {(crate name, version)}), one per distinct text, in cargo-about's order."""
    crates = {}
    texts = {}
    for target, data in runs.items():
        for c in data["crates"]:
            key = (c["package"]["name"], c["package"]["version"])
            crates.setdefault(key, (c["license"], set()))[1].add(target)
        for lic in data["licenses"]:
            entry = texts.setdefault((lic["id"], lic["text"]), (lic["name"], lic["text"], set()))
            entry[2].update((u["crate"]["name"], u["crate"]["version"]) for u in lic["used_by"])
    # Stable: by name, and within a name in the order the runs gave them.
    return crates, sorted(texts.values(), key=lambda t: t[0])


def render(header, crates, texts):
    out = [header.rstrip("\n"), ""]
    out += ["## Font licences", ""]
    for name, folder in (("Manrope", "manrope"), ("Inter (Linux build only)", "inter")):
        text = (FONTS / folder / "OFL.txt").read_text(encoding="utf-8")
        out += [f"### {name}", "", "```", text.strip("\n").replace("\r\n", "\n"), "```", ""]
    out += [
        "## Crates",
        "",
        "Builds says which builds compile the crate in: all four, Windows (x64 and ARM64), "
        "Linux (x86_64 and aarch64), or the ones named.",
        "",
        "| Crate | Version | Licence | Builds |",
        "|---|---|---|---|",
    ]
    for key in sorted(crates, key=crate_key):
        licence, targets = crates[key]
        out.append(f"| {key[0]} | {key[1]} | {licence} | {builds(targets)} |")
    out += ["", "## Licence texts", "", "Each text below is as found in the crates that use it.", ""]
    for name, text, used_by in texts:
        users = ", ".join(f"{n} {v}" for n, v in sorted(used_by, key=crate_key))
        out += [f"### {name}", "", f"Used by: {users}", "", "```", text, "```", ""]
    return "\n".join(out)


def main():
    runs = {}
    for target in TARGETS:
        print(f"cargo-about: {target}", file=sys.stderr)
        runs[target] = about(target)
    crates, texts = merge(runs)
    header = (HERE / "notices.md").read_text(encoding="utf-8")
    path = ROOT / "THIRD_PARTY.md"
    path.write_text(render(header, crates, texts), encoding="utf-8", newline="\n")
    print(f"{path.name}: {len(crates)} crates, {len(texts)} licence texts", file=sys.stderr)


if __name__ == "__main__":
    main()

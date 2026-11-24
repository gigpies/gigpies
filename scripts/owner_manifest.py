#!/usr/bin/env python3
"""Write a reviewed GP05 owner-library manifest from clean committed owner checkouts.

Owner-backed acceptance tests (GP05_MANIFEST) need the exact PA/FX/REC libraries,
headers and provenance. Hand-copied manifests go stale whenever an owner library
is rebuilt; this tool records each owner's committed revision and the SHA-256 of
what is actually on disk, and refuses dirty trees or libraries older than HEAD.

    python3 scripts/owner_manifest.py OUTPUT_DIR [--root /home/shome/p]

Build the libraries first, from each owner checkout, under the shared lock:
    flock -xn /home/shome/p/.gigpies-build.lock env CARGO_INCREMENTAL=0 \
        cargo +1.97.1 build --locked -j1 --release --lib
It never builds, downloads or loads a library itself. Output stays private.
"""
import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

OWNERS = {
    "pa": ("shr-pa", "target/release/libshr_pa.so", "src/shr_pa_v2.h"),
    "fx": ("shr-fx", "target/release/libshr_fx.so", "include/shr_fx.h"),
    "rec": ("shr-rec", "target/release/libshr_rec.so", "include/shr_rec.h"),
}
BUILD = "CARGO_INCREMENTAL=0 cargo +1.97.1 build --locked -j1 --release --lib"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 16), b""):
            h.update(block)
    return h.hexdigest()


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args], check=True, capture_output=True, text=True
    ).stdout.strip()


def describe(root: Path, name: str, library: str, header: str) -> dict:
    repo = root / name
    if git(repo, "status", "--porcelain", "--untracked-files=no"):
        raise SystemExit(f"{name}: tracked changes present; commit or stash before recording")
    lib, hdr = repo / library, repo / header
    for p in (lib, hdr):
        if not p.is_file():
            raise SystemExit(f"{name}: missing {p}")
    # The library must be newer than every tracked build input (Cargo considers
    # the same files); commit timestamps alone would reject a fresh build.
    tracked = git(repo, "ls-files", "--", "src", "include", "build.rs", "Cargo.toml", "Cargo.lock")
    newest = max((repo / f).stat().st_mtime for f in tracked.splitlines() if (repo / f).is_file())
    if lib.stat().st_mtime < newest:
        raise SystemExit(f"{name}: library is older than its sources; rebuild with: {BUILD}")
    return {
        "repository": name,
        "revision": git(repo, "rev-parse", "HEAD"),
        "build": BUILD,
        "files": {str(lib): sha256(lib), str(hdr): sha256(hdr)},
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("output", type=Path)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args(argv)
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    manifest = {"version": 1}
    for key, (name, library, header) in OWNERS.items():
        owner = describe(args.root.resolve(), name, library, header)
        owner_path = out / f"{key}-owner.json"
        owner_path.write_text(json.dumps(owner, indent=2) + "\n")
        lib, hdr = (str(args.root.resolve() / name / p) for p in (library, header))
        manifest[key] = {
            "library": lib,
            "library_sha256": owner["files"][lib],
            "header": hdr,
            "header_sha256": owner["files"][hdr],
            "owner_manifest": str(owner_path),
            "owner_manifest_sha256": sha256(owner_path),
        }
    (out / "modules.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(out / "modules.json")
    return 0


if __name__ == "__main__":
    sys.exit(main())

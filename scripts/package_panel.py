#!/usr/bin/env python3
"""Freeze the shared package into authorized consumers' Cargo directory sources.
No registry upload, Git writes or path dependencies. Consumers resolve exact version.
"""
import hashlib,json,shutil,sys,tomllib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
source=ROOT/'src/instrument-panel'
CONSUMERS=['shr-desk','shr-lightdesk']
# Optional arguments select a subset, e.g. when one consumer still pins an older version.
selected=sys.argv[1:] or CONSUMERS
assert set(selected)<=set(CONSUMERS),'unknown consumer'
for repo in selected:
    root=ROOT.parent/repo
    dest=root/'vendor/shr-instrument-panel'
    dest.mkdir(parents=True,exist_ok=True)
    files=['Cargo.toml','LICENSE']+[str(p.relative_to(source)) for p in (source/'src').glob('*.rs')]+['assets/registry.json','assets/DEJAVU-LICENSE.txt']
    if (dest/'Cargo.toml').exists():
        old_version=tomllib.loads((dest/'Cargo.toml').read_text())['package']['version']
        new_version=tomllib.loads((source/'Cargo.toml').read_text())['package']['version']
        changed=any(not (dest/name).exists() or (dest/name).read_bytes()!=(source/name).read_bytes() for name in files)
        if changed and old_version==new_version:
            raise SystemExit('Changed frozen package: bump shared version and consumer exact requirements before packaging')
    for name in files:
        (dest/name).parent.mkdir(parents=True,exist_ok=True)
        shutil.copyfile(source/name,dest/name)
    hashes={name:hashlib.sha256((dest/name).read_bytes()).hexdigest() for name in sorted(files)}
    (dest/'.cargo-checksum.json').write_text(json.dumps({'files':hashes,'package':None},sort_keys=True)+'\n')
    config=root/'.cargo/config.toml'
    config.parent.mkdir(exist_ok=True)
    if not config.exists():
        config.write_text('''# Frozen shared source, authored in GigPies src/instrument-panel.
[registries.shr-panel]
index = "sparse+https://shr-panel.invalid/index/"
[source.shr-panel]
registry = "sparse+https://shr-panel.invalid/index/"
replace-with = "shr-panel-vendor"
[source.shr-panel-vendor]
directory = "vendor"
''')
    else:
        assert 'shr-panel-vendor' in config.read_text(), 'existing Cargo config requires merge review'
    print(repo,hashlib.sha256(json.dumps(hashes,sort_keys=True).encode()).hexdigest())

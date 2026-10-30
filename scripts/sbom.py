#!/usr/bin/env python3
"""Generate sbom.cdx.json, a CycloneDX 1.6 SBOM, from Cargo.lock and the crate manifests.

Offline by design: the inputs are Cargo.lock, the workspace manifests (read through
`cargo metadata --no-deps`) and each registry crate's own Cargo.toml under
$CARGO_HOME/registry/src. Standard library only, no network. The output is deterministic:
components are sorted and no timestamp is written, so a rerun is byte-identical.
"""

import json
import os
import re
import subprocess
import sys
import tomllib
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "sbom.cdx.json"
PURL = "pkg:cargo/"
# The btop release every collector mirrors; it versions the vendored theme files.
BTOP_VERSION = "1.3.0"
# Files that are not crates: (name, version, SPDX id, path). Licences are the ones
# recorded in each file's own licence record or header.
FILES = [
    ("GalaxaLabsTron Narrow", "1.000", "OFL-1.1", "crates/btop-web/assets/fonts/GalaxaLabsTronNarrow-VF.ttf"),
    ("htmx", "2.0.11", "BSD-2-Clause", "crates/btop-web/assets/htmx.min.js"),
    ("htmx-ext-sse", "2.2.2", "BSD-2-Clause", "crates/btop-web/assets/htmx-sse.min.js"),
]


def licenses(value):
    """Map a Cargo licence string to a CycloneDX licence entry.

    Cargo's legacy "/" separator means the disjunction SPDX writes as OR.
    """
    expression = " OR ".join(part.strip() for part in value.split("/"))
    if re.fullmatch(r"[A-Za-z0-9.+-]+", expression):
        return [{"license": {"id": expression}}]
    return [{"expression": expression}]


def registry_licenses():
    """Read the licence field from each cached crate's own manifest."""
    home = Path(os.environ.get("CARGO_HOME") or Path.home() / ".cargo")
    found = {}
    for manifest in home.glob("registry/src/*/*/Cargo.toml"):
        package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package", {})
        license = package.get("license")
        if isinstance(license, str) and "name" in package and "version" in package:
            found[(package["name"], package["version"])] = license
    return found


def workspace_licenses():
    """Read the licence of each workspace package, with inheritance resolved by cargo."""
    command = ["cargo", "metadata", "--offline", "--locked", "--no-deps", "--format-version", "1"]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(result.stderr)
    return {(p["name"], p["version"]): p.get("license") for p in json.loads(result.stdout)["packages"]}


def theme_files():
    """The btop-vendored themes, identified by the licence header btop's files carry."""
    found = []
    for path in sorted((ROOT / "crates/btop-web/themes").glob("*.theme")):
        text = path.read_text(encoding="utf-8")
        match = re.search(r"^# Licence: ([A-Za-z0-9.+-]+)", text, re.MULTILINE)
        if match:
            found.append((path.name, BTOP_VERSION, match.group(1), str(path.relative_to(ROOT))))
    return found


def verify(document, packages):
    """Fail rather than write an SBOM that misses or duplicates a Cargo.lock crate."""
    assert document["bomFormat"] == "CycloneDX"
    assert document["specVersion"] in ("1.5", "1.6")
    lock = {(p["name"], p["version"]) for p in packages}
    seen = Counter()
    for component in document["components"]:
        assert component["name"] and component["version"] and component["licenses"]
        purl = component.get("purl", "")
        if purl.startswith(PURL):
            name, version = purl[len(PURL):].split("@")
            seen[(name, version)] += 1
    assert seen.keys() == lock, f"crates missing from the SBOM: {sorted(lock - set(seen))}"
    assert all(count == 1 for count in seen.values()), "a crate appears more than once"


def main():
    packages = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))["package"]
    registry = registry_licenses()
    workspace = workspace_licenses()

    components = []
    noassertion = []
    for package in packages:
        name, version = package["name"], package["version"]
        application = package.get("source") is None
        license = workspace.get((name, version)) if application else registry.get((name, version))
        if not isinstance(license, str):
            noassertion.append(f"{name} {version}: no license field in its manifest")
            entries = [{"expression": "NOASSERTION"}]
        else:
            entries = licenses(license)
        components.append(
            {
                "type": "application" if application else "library",
                "name": name,
                "version": version,
                "purl": f"{PURL}{name}@{version}",
                "licenses": entries,
            }
        )
    components.sort(key=lambda c: (c["name"], c["version"]))

    for name, version, license, path in FILES + theme_files():
        components.append(
            {
                "type": "file",
                "name": name,
                "version": version,
                "licenses": [{"license": {"id": license}}],
                "properties": [{"name": "path", "value": path}],
            }
        )

    document = {"bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1, "components": components}
    verify(document, packages)
    OUT.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")

    counts = Counter(component["type"] for component in components)
    print(
        f"sbom.cdx.json: {len(components)} components - "
        f"{counts['application']} application, {counts['library']} library, {counts['file']} file"
    )
    print(f"Cargo.lock: {len(packages)} packages, each present exactly once")
    if noassertion:
        print("NOASSERTION: " + "; ".join(noassertion))


if __name__ == "__main__":
    main()

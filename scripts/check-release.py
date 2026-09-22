#!/usr/bin/env python3
"""Check the metadata shared by Cargo, Herdr, and a release tag (Python 3.11+)."""

import os
from pathlib import Path
import tomllib

root = Path(__file__).resolve().parent.parent
package = tomllib.loads((root / "Cargo.toml").read_text())["package"]
plugin = tomllib.loads((root / "herdr-plugin.toml").read_text())

assert package["version"] == plugin["version"], "Cargo and Herdr versions differ"
assert package["license"] == "MIT", "Expected MIT license metadata"
assert (root / "LICENSE").is_file(), "Missing license file"
assert package["name"] == plugin["id"] == "herdr-sidebar", (
    "Cargo and Herdr names differ"
)
assert package["repository"] == "https://github.com/mcostasilva/herdr-sidebar"
assert set(plugin["platforms"]) == {"macos", "linux"}

if os.environ.get("GITHUB_REF_TYPE") == "tag":
    assert os.environ["GITHUB_REF_NAME"] == f"v{package['version']}", (
        "Tag and package version differ"
    )

print(f"Release metadata OK: v{package['version']}")

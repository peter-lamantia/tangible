"""Prepare platform release assets and their checksums."""
import hashlib
import pathlib
import plistlib
import shutil
import subprocess
import sys
import tempfile

name, binary = sys.argv[1:]
binary = pathlib.Path(binary)
if "macos" in name:
    release_asset = pathlib.Path(name + ".dmg")
    with tempfile.TemporaryDirectory() as directory:
        contents = pathlib.Path(directory) / "Tangible.app" / "Contents"
        executable = contents / "MacOS" / "tangible"
        executable.parent.mkdir(parents=True)
        shutil.copy2(binary, executable)
        with (contents / "Info.plist").open("wb") as output:
            plistlib.dump({
                "CFBundleName": "Tangible",
                "CFBundleIdentifier": "com.tangible.viewer",
                "CFBundleExecutable": "tangible",
                "CFBundlePackageType": "APPL",
            }, output)
        subprocess.run([
            "hdiutil", "create", "-volname", "Tangible", "-srcfolder", directory,
            "-ov", "-format", "UDZO", str(release_asset.resolve()),
        ], check=True)
else:
    release_asset = pathlib.Path(name + binary.suffix)
    shutil.copy2(binary, release_asset)
release_asset.with_name(release_asset.name + ".sha256").write_text(
    f"{hashlib.sha256(release_asset.read_bytes()).hexdigest()}  {release_asset.name}\n"
)

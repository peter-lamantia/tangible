"""Prepare platform release assets and their checksums."""
import hashlib
import json
import os
import pathlib
import plistlib
import shutil
import subprocess
import sys
import tempfile


def notarize(disk_image):
    credentials = [
        "--apple-id", os.environ["APPLE_ID"],
        "--team-id", os.environ["APPLE_TEAM_ID"],
        "--password", os.environ["APPLE_APP_SPECIFIC_PASSWORD"],
    ]
    submission = subprocess.run([
        "xcrun", "notarytool", "submit", str(disk_image), *credentials,
        "--wait", "--output-format", "json",
    ], stdout=subprocess.PIPE, text=True)
    print(submission.stdout, end="", flush=True)
    if not submission.stdout.strip():
        raise SystemExit("Apple notarization returned no submission result.")

    result = json.loads(submission.stdout)
    if submission.returncode or result.get("status") != "Accepted":
        if submission_id := result.get("id"):
            subprocess.run([
                "xcrun", "notarytool", "log", submission_id, *credentials,
            ], check=False)
        raise SystemExit("Apple notarization did not accept the disk image.")

    subprocess.run(["xcrun", "stapler", "staple", str(disk_image)], check=True)
    subprocess.run(["xcrun", "stapler", "validate", str(disk_image)], check=True)


name, binary = sys.argv[1:]
binary = pathlib.Path(binary)
if "macos" in name:
    release_asset = pathlib.Path(name + ".dmg")
    signing_identity = os.environ.get("MACOS_SIGNING_IDENTITY")
    if signing_identity:
        signing = [
            "codesign", "--sign", signing_identity,
            "--keychain", os.environ["MACOS_SIGNING_KEYCHAIN"], "--timestamp",
        ]

    with tempfile.TemporaryDirectory() as directory:
        app = pathlib.Path(directory) / "Tangible.app"
        contents = app / "Contents"
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
        if signing_identity:
            subprocess.run([
                *signing, "--force", "--options", "runtime", str(app),
            ], check=True)
            subprocess.run(["codesign", "--verify", "--strict", str(app)], check=True)
        subprocess.run([
            "hdiutil", "create", "-volname", "Tangible", "-srcfolder", directory,
            "-ov", "-format", "UDZO", str(release_asset.resolve()),
        ], check=True)
    if signing_identity:
        subprocess.run([
            *signing, "--identifier", f"com.tangible.viewer.{name}", str(release_asset),
        ], check=True)
        notarize(release_asset)
else:
    release_asset = pathlib.Path(name + binary.suffix)
    shutil.copy2(binary, release_asset)
release_asset.with_name(release_asset.name + ".sha256").write_text(
    f"{hashlib.sha256(release_asset.read_bytes()).hexdigest()}  {release_asset.name}\n"
)

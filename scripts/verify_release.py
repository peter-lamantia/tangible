"""Verify packaged macOS and Windows release assets on their native runners."""
import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile


def verify_macos(asset):
    requirement = "=anchor apple generic and certificate leaf[field.1.2.840.113635.100.6.1.13]"
    verify_signature = ["codesign", "--verify", "--strict", "-R", requirement]
    subprocess.run([*verify_signature, asset], check=True)
    subprocess.run(["xcrun", "stapler", "validate", asset], check=True)

    with tempfile.TemporaryDirectory() as directory:
        mount = Path(directory) / "mounted"
        mount.mkdir()
        subprocess.run([
            "hdiutil", "attach", asset, "-readonly", "-nobrowse", "-mountpoint", mount,
        ], check=True)
        try:
            shortcut = mount / "Applications"
            if not shortcut.is_symlink() or shortcut.readlink() != Path("/Applications"):
                raise SystemExit("DMG is missing the /Applications shortcut.")

            app = mount / "Tangible.app"
            contents = app / "Contents"
            with (contents / "Info.plist").open("rb") as plist:
                info = plistlib.load(plist)
            executable = contents / "MacOS" / "tangible"
            if info.get("CFBundleExecutable") != executable.name or not (
                executable.is_file() and os.access(executable, os.X_OK)
            ):
                raise SystemExit("Tangible.app is missing its executable.")

            expected_icon = Path(__file__).resolve().parents[1] / "assets" / "app-icon.icns"
            icon = contents / "Resources" / expected_icon.name
            if info.get("CFBundleIconFile") != icon.name or not (
                icon.is_file() and icon.read_bytes() == expected_icon.read_bytes()
            ):
                raise SystemExit("Tangible.app is missing the expected application icon.")

            subprocess.run([*verify_signature, app], check=True)
        finally:
            subprocess.run(["hdiutil", "detach", mount], check=True)


def verify_windows(asset):
    extract_icon = ctypes.WinDLL("shell32").ExtractIconExW
    icon_pointer = ctypes.POINTER(wintypes.HICON)
    extract_icon.argtypes = [
        wintypes.LPCWSTR, ctypes.c_int, icon_pointer, icon_pointer, wintypes.UINT,
    ]
    extract_icon.restype = wintypes.UINT
    destroy_icon = ctypes.WinDLL("user32").DestroyIcon
    destroy_icon.argtypes = [wintypes.HICON]
    destroy_icon.restype = wintypes.BOOL

    # A negative index selects the numeric resource ID declared in build.rs.
    icon = wintypes.HICON()
    count = extract_icon(str(asset), -1, ctypes.byref(icon), None, 1)
    if icon:
        destroy_icon(icon)
    if count != 1 or not icon:
        raise SystemExit("The Windows executable is missing a readable application icon at ID 1.")


if __name__ == "__main__":
    (filename,) = sys.argv[1:]
    asset = Path(filename).resolve(strict=True)
    if asset.suffix == ".dmg":
        verify_macos(asset)
    elif asset.suffix == ".exe":
        verify_windows(asset)
    else:
        raise SystemExit(f"Unsupported release asset: {asset.name}")
    print(f"Verified {asset.name}")

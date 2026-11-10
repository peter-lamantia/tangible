"""Build portable release archives on each platform."""
import hashlib
import pathlib
import sys
import zipfile

name, binary = sys.argv[1:]
binary = pathlib.Path(binary)
archive = pathlib.Path(f"{name}.zip")
with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as output:
    if "macos" in name:
        root = "Tangible.app/Contents"
        output.write(binary, f"{root}/MacOS/tangible")
        output.writestr(f"{root}/Info.plist", '''<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleName</key><string>Tangible</string><key>CFBundleIdentifier</key><string>com.tangible.viewer</string><key>CFBundleExecutable</key><string>tangible</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>''')
    else:
        output.write(binary, binary.name)
        if "linux" in name:
            output.write("assets/impossible_cube_fuchsia.svg", "tangible.svg")
            output.writestr("tangible.desktop", "[Desktop Entry]\nType=Application\nName=Tangible\nExec=tangible %f\nIcon=tangible\nCategories=Graphics;3DGraphics;\nTerminal=false\n")
    output.write("README.md", "README.md")
    output.write("assets/ARTWORK.md", "ARTWORK.md")
archive.with_name(archive.name + ".sha256").write_text(f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")

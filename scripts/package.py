"""Name a release executable and write its checksum."""
import hashlib
import pathlib
import shutil
import sys

name, binary = sys.argv[1:]
binary = pathlib.Path(binary)
release_binary = pathlib.Path(name + binary.suffix)
shutil.copy2(binary, release_binary)
release_binary.with_name(release_binary.name + ".sha256").write_text(
    f"{hashlib.sha256(release_binary.read_bytes()).hexdigest()}  {release_binary.name}\n"
)

"""Package the native CLI without losing executable permissions in zip archives."""
from pathlib import Path
import sys
from zipfile import ZipFile, ZIP_DEFLATED
root, output = map(Path, sys.argv[1:])
with ZipFile(output, "w", ZIP_DEFLATED) as archive:
    for path in sorted(root.iterdir()):
        if path.is_file():
            archive.write(path, path.name)

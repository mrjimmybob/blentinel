"""Unpacks a downloaded artifact into an empty folder, refusing anything a
release folder can't hold (proposal 0013 §10.2 step 2, D-107).

    python3 -I unzip-release.py <artifact.zip> <empty folder>

A release folder is flat: every entry must be a plain file with a plain name
(letters, digits, '.', '_', '-'), at most 64 of them, at most MAX_BYTES in
all. Nothing is written outside the folder, and no link or special file is
made. The signature isn't checked here: verify-release.sh does that next.
"""

import os
import re
import stat
import sys
import zipfile

NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
MAX_FILES = 64
MAX_BYTES = 1 << 30


def fail(message: str) -> None:
    print(f"unzip-release: {message}", file=sys.stderr)
    sys.exit(1)


def main() -> None:
    if len(sys.argv) != 3:
        fail("usage: unzip-release.py <artifact.zip> <empty folder>")
    archive, folder = sys.argv[1], sys.argv[2]
    if os.listdir(folder):
        fail(f"{folder} isn't empty")
    try:
        zf = zipfile.ZipFile(archive)
    except (OSError, zipfile.BadZipFile) as e:
        fail(f"{archive} isn't a zip: {e}")
    with zf:
        entries = zf.infolist()
        if not entries or len(entries) > MAX_FILES:
            fail(f"{len(entries)} entries: a release has 1 to {MAX_FILES}")
        names = [e.filename for e in entries]
        if len(set(names)) != len(names):
            fail("a name appears twice")
        if sum(e.file_size for e in entries) > MAX_BYTES:
            fail(f"more than {MAX_BYTES} bytes")
        for entry in entries:
            mode = entry.external_attr >> 16
            if entry.is_dir() or not NAME.match(entry.filename) or (mode and not stat.S_ISREG(mode)):
                fail(f"{entry.filename!r} isn't a plain file at the top level")
        for entry in entries:
            target = os.path.join(folder, entry.filename)
            with zf.open(entry) as source, open(target, "xb") as out:
                written = 0
                while chunk := source.read(1 << 20):
                    written += len(chunk)
                    if written > entry.file_size:
                        fail(f"{entry.filename} is larger than the zip says")
                    out.write(chunk)


if __name__ == "__main__":
    main()

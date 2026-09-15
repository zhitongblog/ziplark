#!/usr/bin/env bash
#
# Regenerate the RAR fixtures in crates/ziplark-core/tests/fixtures/.
#
# RAR is proprietary and there is no Rust writer for it, so the fixtures are
# built once with the official `rar` tool and committed. This script documents
# exactly what each one contains, so a fixture is never a black box.
#
# Usage:  RAR=/path/to/rar scripts/make-rar-fixtures.sh
#
# Get `rar` from https://www.rarlab.com/download.htm (the trial binary can
# create archives). It has to be a **6.x** build: RAR 7 dropped `-ma4`, and the
# legacy `.r00` volume scheme only exists in RAR4. Everything here is tiny on
# purpose — the biggest fixture is a three-part volume set of 20 KB each.
set -euo pipefail

RAR="${RAR:-rar}"
command -v "$RAR" >/dev/null || { echo "no rar binary — set RAR=/path/to/rar" >&2; exit 1; }

repo="$(cd "$(dirname "$0")/.." && pwd)"
out="$repo/crates/ziplark-core/tests/fixtures"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

mkdir -p "$out"
cd "$work"

# Deterministic payload. big.bin is seeded random so it does not compress: at a
# 20 KB volume size a 50 KB file genuinely spans three parts, and the middle
# part carries no file header of its own.
mkdir -p tree/docs
python3 - <<'PY'
import pathlib, random
r = random.Random(20260915)
pathlib.Path("tree/big.bin").write_bytes(bytes(r.randrange(256) for _ in range(50_000)))
pathlib.Path("tree/docs/readme.txt").write_text("hello from a rar fixture\n")
pathlib.Path("tree/docs/notes.txt").write_text("second small file\n")
PY

rar() { "$RAR" "$@" >/dev/null; }

# 1. Multi-volume RAR5, modern `.partN.rar` naming. big.bin spans all parts,
#    so the middle part carries a continuation header and no start-of-file.
rar a -ma5 -v20k -ep1 multi.rar tree
mv multi.part*.rar "$out/"

# 2. Multi-volume RAR4 with the legacy `-vn` naming: `legacy.rar`, `legacy.r00`,
#    `legacy.r01`. Still the form most old downloads arrive in, and the one
#    whose first part is `.rar` rather than `.r01` — which is exactly what
#    naive first-part arithmetic gets wrong.
rar a -ma4 -vn -v20k -ep1 legacy.rar tree
mv legacy.rar legacy.r0* "$out/"

# 3. Header-encrypted: even the file names need the password ("ziplark").
rar a -ma5 -hpziplark -ep1 hdrenc.rar tree/docs
mv hdrenc.rar "$out/"

# 4. Solid archive with a recovery record and an archive comment.
printf 'Ziplark test fixture.\nSecond line.\n' > comment.txt
rar a -ma5 -s -rr5p -zcomment.txt -ep1 solid.rar tree/docs
mv solid.rar "$out/"

# 5. CJK / legacy-code-page names. RAR5 always stores names as Unicode, so this
#    proves the listing is not mangling them on the way out.
mkdir -p cjk
echo hi > "cjk/中文文件.txt"
echo hi > "cjk/日本語のファイル.txt"
rar a -ma5 -ep1 cjk.rar cjk
mv cjk.rar "$out/"

# 6. Symlink path-escape attempt: `escape/evil` is stored as a link pointing
#    outside the destination, and the entry after it writes *through* that link
#    (`escape/evil/owned.txt`). The second name contains no `..` at all, so a
#    name-only guard waves it through and the write lands in /tmp. Extraction
#    must refuse it and leave nothing outside the destination.
#
#    The two entries are added from separate staging directories with `-ap`,
#    because one path cannot be both a symlink and a directory on disk at once —
#    and adding them in one pass would make rar replace the link entry instead
#    of keeping both.
mkdir -p stage-link stage-file
ln -s /tmp/ziplark-rar-escape stage-link/evil
echo pwned > stage-file/owned.txt
( cd stage-link && rar a -ma5 -ol -apescape ../linkescape.rar evil )
( cd stage-file && rar a -ma5 -apescape/evil ../linkescape.rar owned.txt )
mv linkescape.rar "$out/"

# 6b. A volume set whose *early* files are complete in the first volume: the
#     small files are added before the big spanning one, so deleting the last
#     volume leaves them recoverable. This is the shape a half-finished
#     download has, and what salvaging is for.
rar a -ma5 -v20k -ep1 salvage.rar tree/docs tree/big.bin
mv salvage.part*.rar "$out/"

# 6c. Permissions and links as a unix host stores them: an executable script, a
#     plain file, and a symlink — the metadata an archiver is expected not to
#     lose.
mkdir -p perms
printf '#!/bin/sh\necho hi\n' > perms/run.sh
chmod 755 perms/run.sh
echo data > perms/plain.txt
chmod 644 perms/plain.txt
ln -s plain.txt perms/link.txt
rar a -ma5 -ol -ep1 perms.rar perms
mv perms.rar "$out/"

# 7. Damaged archive: valid headers, corrupt file data. Testing must name the
#    entries that fail instead of giving up on the whole archive.
rar a -ma5 -ep1 damaged.rar tree
python3 - <<'PY'
import pathlib
p = pathlib.Path("damaged.rar")
b = bytearray(p.read_bytes())
# big.bin is added first, so its compressed data sits right after the opening
# headers and ends well before the two small files' headers and data. Flipping
# a slice in the middle of it fails that one entry's checksum and leaves every
# header, and the other two entries, intact.
for i in range(2_000, 20_000):
    b[i] ^= 0xFF
p.write_bytes(bytes(b))
PY
mv damaged.rar "$out/"

# 8. A self-extracting archive: an executable stub, then the archive. Real SFX
#    stubs are a few hundred KB of machine code, not worth committing, so this
#    one fakes the stub with 4 KB of filler. What matters to a reader is the
#    same either way: the RAR signature is not at offset 0.
python3 - "$out" <<'PYSFX'
import pathlib, sys
out = pathlib.Path(sys.argv[1])
(out / "sfx.exe").write_bytes(b"MZ" + b"\x00" * 4094 + (out / "cjk.rar").read_bytes())
PYSFX

cd "$out"
echo "wrote:"
ls -l multi.part*.rar legacy.rar legacy.r0* salvage.part*.rar perms.rar hdrenc.rar solid.rar cjk.rar linkescape.rar damaged.rar sfx.exe

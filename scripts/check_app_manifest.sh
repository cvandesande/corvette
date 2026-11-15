#!/usr/bin/env bash
# Checks a tree's web app manifest, the icons it names, and the shell's link:
# the members an install needs, each named icon present at its stated pixel
# size, and exactly one manifest link before the head's close.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "check_app_manifest: usage: $0 <tree> <shell-file>" >&2
  exit 1
fi

TREE="$1"
SHELL_FILE="$2"

if [[ ! -d "$TREE" ]]; then
  echo "check_app_manifest: no such directory: $TREE" >&2
  exit 1
fi

python3 -I - "$TREE" "$SHELL_FILE" <<'PY'
import json
import re
import struct
import sys
from pathlib import Path

tree = Path(sys.argv[1])
shell_file = sys.argv[2]
failures = []


def fail(what):
    failures.append(what)


def check_shell_colour(text):
    colour_tags = []
    for match in re.finditer(r"<meta\b[^>]*>", text, re.IGNORECASE):
        attributes = dict(re.findall(r'([A-Za-z][A-Za-z0-9-]*)\s*=\s*"([^"]*)"', match.group(0)))
        if attributes.get("name") == "theme-color":
            colour_tags.append(attributes)
    if not colour_tags:
        fail(shell_file + " holds no theme-color meta")
        return None
    if len(colour_tags) > 1:
        fail(shell_file + " holds " + str(len(colour_tags)) + " theme-color metas, want exactly one")
        return None
    return colour_tags[0].get("content")


def check_manifest(manifest, colour):
    if manifest.get("name") != "Corvette":
        fail("manifest name is not Corvette: " + repr(manifest.get("name")))
    if manifest.get("short_name") != "Corvette":
        fail("manifest short_name is not Corvette: " + repr(manifest.get("short_name")))
    for member in ("id", "start_url", "scope"):
        if manifest.get(member) != "/":
            fail("manifest " + member + " is not /: " + repr(manifest.get(member)))
    if manifest.get("display") not in ("standalone", "fullscreen"):
        fail("manifest display is not standalone or fullscreen: " + repr(manifest.get("display")))
    for member in ("background_color", "theme_color"):
        if colour is not None and manifest.get(member) != colour:
            fail("manifest " + member + " is not " + colour + ": " + repr(manifest.get(member)))
    if "orientation" in manifest:
        fail("manifest carries an orientation member: " + repr(manifest["orientation"]))


def check_icon_file(src, side):
    path = tree / src.lstrip("/")
    if not path.is_file():
        fail("manifest icon file is missing: " + str(path))
        return
    head = path.read_bytes()[:33]
    if (head[:8] != b"\x89PNG\r\n\x1a\n" or head[8:12] != b"\x00\x00\x00\x0d"
            or head[12:16] != b"IHDR" or len(head) != 33):
        fail("icon file is not a PNG whose first chunk is IHDR of length 13: " + str(path))
        return
    width, height = struct.unpack(">II", head[16:24])
    if width != side or height != side:
        fail("icon " + src + " states " + str(side) + "x" + str(side)
             + " but the PNG pixels are " + str(width) + "x" + str(height))


def check_icons(manifest):
    icons = manifest.get("icons")
    if not isinstance(icons, list) or not icons:
        fail("manifest icons is not a non-empty list")
        return
    usable = []
    for entry in icons:
        if not isinstance(entry, dict):
            fail("manifest icons entry is not an object: " + repr(entry))
            continue
        src = entry.get("src")
        if not isinstance(src, str) or not src.startswith("/icons/"):
            fail("manifest icon src does not start with /icons/: " + repr(src))
            continue
        sizes = entry.get("sizes")
        stated = re.fullmatch(r"([0-9]+)x([0-9]+)", sizes) if isinstance(sizes, str) else None
        if stated is None or stated.group(1) != stated.group(2):
            fail("manifest icon " + src + " sizes is not <n>x<n> with equal sides: " + repr(sizes))
            continue
        if entry.get("type") != "image/png":
            fail("manifest icon " + src + " type is not image/png: " + repr(entry.get("type")))
        purpose = entry.get("purpose")
        if purpose not in ("any", "maskable"):
            fail("manifest icon " + src + " purpose is not any or maskable: " + repr(purpose))
        side = int(stated.group(1))
        check_icon_file(src, side)
        usable.append((purpose, side))
    if not any(purpose == "any" and side >= 192 for purpose, side in usable):
        fail("manifest has no icon with purpose any and a side of at least 192")
    if not any(purpose == "maskable" for purpose, side in usable):
        fail("manifest has no icon with purpose maskable")


def check_shell(text):
    if text is None:
        fail("shell file is missing: " + str(tree / shell_file))
        return
    links = [match for match in re.finditer(r"<link\b[^>]*>", text, re.IGNORECASE)
             if re.search(r'rel\s*=\s*"manifest"', match.group(0), re.IGNORECASE)]
    if not links:
        fail(shell_file + " holds no manifest link")
        return
    if len(links) > 1:
        fail(shell_file + " holds " + str(len(links)) + " manifest links, want exactly one")
    tag = links[0].group(0)
    attributes = dict(re.findall(r'([A-Za-z][A-Za-z0-9-]*)\s*=\s*"([^"]*)"', tag))
    if attributes.get("href") != "/manifest.json":
        fail("manifest link href is not /manifest.json: " + repr(attributes.get("href")))
    if re.search(r"\bcrossorigin\b", tag, re.IGNORECASE):
        fail("manifest link carries a crossorigin attribute")
    head_end = text.lower().rfind("</head>")
    if head_end < 0 or links[0].start() > head_end:
        fail("manifest link does not come before the head's close")


shell_path = tree / shell_file
shell_text = shell_path.read_text(encoding="utf-8") if shell_path.is_file() else None
colour = None if shell_text is None else check_shell_colour(shell_text)

manifest_path = tree / "manifest.json"
if not manifest_path.is_file():
    fail("manifest file is missing: " + str(manifest_path))
else:
    try:
        parsed = json.loads(manifest_path.read_text(encoding="utf-8"))
    except ValueError as err:
        parsed = None
        fail("manifest.json does not parse as JSON: " + str(err))
    if parsed is not None and not isinstance(parsed, dict):
        fail("manifest.json is not a JSON object")
        parsed = None
    if isinstance(parsed, dict):
        check_manifest(parsed, colour)
        check_icons(parsed)
check_shell(shell_text)

if failures:
    for what in failures:
        print("check_app_manifest: " + what, file=sys.stderr)
    sys.exit(1)
print("check_app_manifest: ok")
PY

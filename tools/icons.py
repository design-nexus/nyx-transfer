#!/usr/bin/env python3
"""Transfer's symbolic icons, drawn from the mockups' icon set (nyx-design's
mockups/revamp/apps/transfer.js, mockups/revamp/mock.js and mockups/git/git.js) so the
app shows the same glyphs.

GTK fills every shape of a symbolic icon with the text colour, so the mockups' stroked
outlines are turned into filled shapes first (picosvg: `pip install picosvg`, e.g. in a
virtualenv). Writes apps/nyx-transfer/data/icons/transfer-<name>-symbolic.svg and
apps/nyx-transfer/src/icons.rs (the list the app installs).
Usage: tools/icons.py [path to nyx-design]
"""
import pathlib
import re
import sys

from picosvg.svg import SVG

root = pathlib.Path(__file__).resolve().parent.parent
design = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else root.parent / "nyx-design")
# Earlier sources win when two draw the same name.
sources = [
    (design / "mockups/revamp/apps/transfer.js", "const ICONS = {"),
    (design / "mockups/revamp/mock.js", "const I = {"),
    (design / "mockups/git/git.js", "const GI = {"),
]
WANTED = """xfer arrow-r arrow-u both monitor home folder download image music film archive
server cloud plus chev-d chev-l chev-r chev-u filter sort code file file-text check-c x
pause play refresh sidebar copy alert sync""".split()

icons = {}
for path, start in sources:
    text = path.read_text()
    i = text.index(start)
    j = text.index("};", i)
    for m in re.finditer(r"""^\s*"?([a-z0-9-]+)"?:\s*'([^']*)'""", text[i:j], re.M):
        if m.group(1) in WANTED:
            icons.setdefault(m.group(1), m.group(2))
missing = [name for name in WANTED if name not in icons]
if missing:
    sys.exit(f"not in the mockups: {' '.join(missing)}")

out = root / "apps/nyx-transfer/data/icons"
out.mkdir(parents=True, exist_ok=True)
for old in out.glob("transfer-*-symbolic.svg"):
    old.unlink()
names = []
for name, body in sorted(icons.items()):
    body = body.replace("currentColor", "#2e3436")
    drawn = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">'
        '<g fill="none" stroke="#2e3436" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">'
        f"{body}</g></svg>"
    )
    filled = SVG.fromstring(drawn).topicosvg().tostring(pretty_print=False)
    svg = filled.replace("<svg ", '<svg width="16" height="16" ', 1) + "\n"
    (out / f"transfer-{name}-symbolic.svg").write_text(svg)
    names.append(name)

rs = ["// Written by tools/icons.py from the mockups' icon set; run it again to change them.", "pub const ICONS: &[(&str, &str)] = &["]
for name in names:
    rs.append(f'    ("transfer-{name}-symbolic.svg", include_str!("../data/icons/transfer-{name}-symbolic.svg")),')
rs.append("];")
(root / "apps/nyx-transfer/src/icons.rs").write_text("\n".join(rs) + "\n")
print(f"{len(names)} icons")

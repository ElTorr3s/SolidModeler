#!/usr/bin/env python3
"""Regenerate the summary table in docs/feature-parity.md.

Counts the rows under each `## ` section and rewrites the block between the
summary markers. Run it after changing any status.
"""

import pathlib
import re

DOC = pathlib.Path(__file__).with_name("feature-parity.md")
START, END = "<!-- summary:start -->", "<!-- summary:end -->"
SKIP = {"How to read the tables", "Summary"}
NONE = "—"


def main() -> None:
    text = DOC.read_text()
    sections: dict[str, dict[str, int]] = {}
    current = None
    for line in text.splitlines():
        if line.startswith("## "):
            title = line[3:].strip()
            current = None if title in SKIP else sections.setdefault(title, {})
            continue
        if current is None or not line.startswith("| ") or line.startswith("| Inventor"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) != 5:
            continue
        inv, sw, _step, status, _notes = cells
        key = (
            "both" if inv != NONE and sw != NONE
            else "inventor" if inv != NONE
            else "solidworks" if sw != NONE
            else "own"
        )
        for k in ("items", key, status):
            current[k] = current.get(k, 0) + 1

    cols = [
        ("items", "Items"),
        ("both", "In both"),
        ("inventor", "Inventor only"),
        ("solidworks", "SolidWorks only"),
        ("own", "Ours only"),
        ("done", "Done"),
        ("in progress", "In progress"),
    ]
    rows = ["| Environment | " + " | ".join(h for _, h in cols) + " |"]
    rows.append("|---" * (len(cols) + 1) + "|")
    totals = {k: 0 for k, _ in cols}
    for title, counts in sections.items():
        rows.append(f"| {title} | " + " | ".join(str(counts.get(k, 0)) for k, _ in cols) + " |")
        for k, _ in cols:
            totals[k] += counts.get(k, 0)
    rows.append("| **Total** | " + " | ".join(f"**{totals[k]}**" for k, _ in cols) + " |")

    block = f"{START}\n" + "\n".join(rows) + f"\n{END}"
    text = re.sub(re.escape(START) + ".*?" + re.escape(END), block, text, flags=re.S)
    DOC.write_text(text)


if __name__ == "__main__":
    main()

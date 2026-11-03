#!/usr/bin/env python3
"""Print a dotted JSON path from stdin for the native UI gate.

Missing values and invalid JSON print nothing; lists are comma-separated.
"""
import json
import sys


def pick(doc, path):
    for part in path.split("."):
        if isinstance(doc, dict):
            doc = doc.get(part)
        elif isinstance(doc, list) and part.isdigit() and int(part) < len(doc):
            doc = doc[int(part)]
        else:
            return None
        if doc is None:
            return None
    return doc


def show(v):
    if v is None:
        return ""
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, list):
        return ",".join(show(i) for i in v)
    if isinstance(v, dict):
        return json.dumps(v, sort_keys=True, separators=(",", ":"))
    return str(v)




def main():
    args = sys.argv[1:]
    path = args[0] if args else ""
    try:
        doc = json.loads(sys.stdin.read())
    except ValueError:
        return 0
    print(show(pick(doc, path) if path else doc))
    return 0


if __name__ == "__main__":
    sys.exit(main())

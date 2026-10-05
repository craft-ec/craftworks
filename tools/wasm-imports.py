#!/usr/bin/env python3
"""A wasm module's IMPORTS, one per line: `module name` (the import section read directly; no tool needed)."""
import sys


def leb(b, i):
    r = s = 0
    while True:
        x = b[i]
        i += 1
        r |= (x & 0x7F) << s
        s += 7
        if x < 0x80:
            return r, i


def imports(b):
    if b[:4] != b"\0asm":
        raise SystemExit("not a wasm module")
    i = 8
    while i < len(b):
        sid = b[i]
        size, i = leb(b, i + 1)
        if sid == 2:
            n, j = leb(b, i)
            for _ in range(n):
                ml, j = leb(b, j)
                mod = b[j : j + ml].decode()
                j += ml
                fl, j = leb(b, j)
                name = b[j : j + fl].decode()
                j += fl
                kind = b[j]
                j += 1
                if kind == 0:  # function: its type index
                    _, j = leb(b, j)
                elif kind == 1:  # table: element type, limits
                    j += 1
                    flags, j = leb(b, j)
                    _, j = leb(b, j)
                    if flags & 1:
                        _, j = leb(b, j)
                elif kind == 2:  # memory: limits
                    flags, j = leb(b, j)
                    _, j = leb(b, j)
                    if flags & 1:
                        _, j = leb(b, j)
                elif kind == 3:  # global: value type, mutability
                    j += 2
                yield mod, name
            return
        i += size


for mod, name in imports(open(sys.argv[1], "rb").read()):
    print(mod, name)

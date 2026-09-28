"""Put wasm-bindgen's snippet files INTO its glue, so the glue imports nothing by relative path (a package is loaded
as bytes, never as a file beside others). Each `import * as importN from "./snippets/<crate>/<file>.js"` becomes a
const holding that file's exports. Exits non-zero if a snippet it names is missing."""
import pathlib, re, sys

glue = pathlib.Path(sys.argv[1])
src = glue.read_text()
pat = re.compile(r'^import \* as (\w+) from "(\./snippets/[^"]+\.js)";?\s*$', re.M)

def inline(m):
    name, rel = m.group(1), m.group(2)
    code = (glue.parent / rel).read_text()
    exported = re.findall(r'^export function (\w+)', code, re.M)
    body = re.sub(r'^export function', 'function', code, flags=re.M)
    return f"const {name} = (() => {{\n{body}\nreturn {{ {', '.join(exported)} }};\n}})();"

out, n = pat.subn(inline, src)
if "./snippets/" in re.sub(r'"\./snippets/[^"]+": import\d+,', "", out):
    sys.exit(f"{glue}: a snippet import is left")
glue.write_text(out)
print(f"{glue.name}: {n} snippet(s) inlined")

# Which released epochs accepted what

Written by `regenerate_the_corpus`. Only entries EVERY epoch
accepted are frozen in accepted.bin; this records the rest,
because a case one released contract takes and another refuses
is a fact about the epochs and not a rounding error.

-   2 x branch   accepted by: 1521dddb9ecbaa16
-  22 x leaf     accepted by: 21ae7e734d40224d+1521dddb9ecbaa16

## What this table does and does not say

READABLE under every epoch; WRITABLE under the current one.

New writes go to the current epoch only (ARCHITECTURE §3), so a writer has to
satisfy that one contract — which is what the WRITE arm checks, against the
wasm the contracts build produced. Older epochs' data must stay readable,
which is what the CORPUS arm checks.

The difference above is the case that distinction exists for: `21ae7e73`
predates the parity rule and refuses any branch that lists parity ids, which
is every branch this SDK writes. So this SDK cannot write a tree deeper than
a leaf under that epoch — and does not have to. Read this table as "we must
stay writable under every epoch ever released" and you have a promise nobody
made, and one the upgrade procedure exists precisely to avoid needing.

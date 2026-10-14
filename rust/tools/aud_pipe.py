#!/usr/bin/env python3
"""Drive a running Audacity 3.x via mod-script-pipe (Linux). Usage: aud_pipe.py 'Cmd: a=b' ..."""
import os, sys
uid = os.getuid()
TO = f"/tmp/audacity_script_pipe.to.{uid}"
FROM = f"/tmp/audacity_script_pipe.from.{uid}"
_to = open(TO, "w"); _from = open(FROM, "r")
def cmd(c):
    _to.write(c + "\n"); _to.flush()
    out = []
    while True:
        line = _from.readline()
        if line == "\n" and out: break
        if line == "": break
        out.append(line)
    r = "".join(out)
    print(f">> {c}\n{r.strip()}", file=sys.stderr)
    return r
if __name__ == "__main__":
    for c in sys.argv[1:]: cmd(c)

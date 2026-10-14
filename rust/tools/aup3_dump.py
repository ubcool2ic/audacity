#!/usr/bin/env python3
"""Decode an .aup3 project/autosave doc to XML text (reference decoder for tests)."""
import sqlite3, struct, sys
def decode(d, doc):
    names = {}; cs = 1; out = []; intag = False; buf = d + doc; i = 0
    def s(b):
        return b.decode('utf-8') if cs == 1 else b.decode('utf-16le' if cs == 2 else 'utf-32le')
    def nm(): 
        nonlocal i; v = struct.unpack_from('<H', buf, i)[0]; i += 2; return names[v]
    while i < len(buf):
        t = buf[i]; i += 1
        if t == 0: cs = buf[i]; i += 1
        elif t == 15:
            idn, ln = struct.unpack_from('<HH', buf, i); i += 4; names[idn] = s(buf[i:i+ln]); i += ln
        elif t == 1:
            if intag: out.append('>')
            out.append('<' + nm()); intag = True
        elif t == 2:
            if intag: out.append('/>'); intag=False; nm()
            else: out.append('</%s>' % nm())
        elif t == 3:
            n = nm(); ln = struct.unpack_from('<i', buf, i)[0]; i += 4; out.append(' %s="%s"' % (n, s(buf[i:i+ln]))); i += ln
        elif t in (4, 6):
            n = nm(); out.append(' %s="%d"' % (n, struct.unpack_from('<i', buf, i)[0])); i += 4
        elif t == 5: n = nm(); out.append(' %s="%d"' % (n, buf[i])); i += 1
        elif t == 7: n = nm(); out.append(' %s="%d"' % (n, struct.unpack_from('<q', buf, i)[0])); i += 8
        elif t == 8: n = nm(); out.append(' %s="%d"' % (n, struct.unpack_from('<I', buf, i)[0])); i += 4
        elif t == 9: n = nm(); v, dg = struct.unpack_from('<fi', buf, i); i += 8; out.append(' %s="%r"f%d' % (n, v, dg))
        elif t == 10: n = nm(); v, dg = struct.unpack_from('<di', buf, i); i += 12; out.append(' %s="%r"d%d' % (n, v, dg))
        elif t in (11, 12):
            ln = struct.unpack_from('<i', buf, i)[0]; i += 4; txt = s(buf[i:i+ln]); i += ln
            if intag and t == 11: out.append('>'); intag = False
            out.append(('[DATA]' if t == 11 else '[RAW]') + txt)
        elif t == 16: n = nm(); ln = struct.unpack_from('<i', buf, i)[0]; i += 4 + ln; out.append(' %s=<blob %d>' % (n, ln))
        else: out.append('[type %d]' % t)
        if t in (1,2) : out.append('\n') if t==2 else None
    return ''.join(out)
db = sqlite3.connect(sys.argv[1])
print('app_id', hex(db.execute('pragma application_id').fetchone()[0]), 'user_version', hex(db.execute('pragma user_version').fetchone()[0]))
print([r[0] for r in db.execute("select sql from sqlite_master")])
for tbl in ('project', 'autosave'):
    for d, doc in db.execute(f'select dict, doc from {tbl}'):
        print(f'--- {tbl} dict={len(d)} doc={len(doc)}'); print(decode(d, doc))
for r in db.execute('select blockid, sampleformat, summin, summax, sumrms, length(summary256), length(summary64k), length(samples) from sampleblocks'): print(r)

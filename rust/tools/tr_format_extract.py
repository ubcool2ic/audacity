import re,json,glob,os
R="/workspace/audacity-fork/rust/crates/au-app/src/"
files=[f for f in glob.glob(R+"**/*.rs",recursive=True) if "/bin/" not in f and not f.endswith("_tests.rs")]
sites=[]
for f in files:
    s=open(f).read()
    mt=re.search(r"#\[cfg\(test\)\]\s*mod tests\s*\{",s); cut=mt.start() if mt else -1; lim=cut if cut>=0 else len(s)
    for m in re.finditer(r'format!\(\s*"',s[:lim]):
        i=m.start()+len("format!"); d=0; j=i; inq=False; esc=False
        while j<len(s):
            c=s[j]
            if inq:
                if esc: esc=False
                elif c=='\\': esc=True
                elif c=='"': inq=False
            else:
                if c=='"': inq=True
                elif c=='(': d+=1
                elif c==')':
                    d-=1
                    if d==0: break
            j+=1
        expr=s[m.start():j+1]
        mm=re.match(r'format!\(\s*"((?:[^"\\]|\\.)*)"',expr)
        if not mm or len(expr)>600: continue
        lit=mm.group(1)
        if not re.search(r'[A-Za-z]{3,}',lit): continue
        line=s.count("\n",0,m.start())+1
        ls=s.rfind("\n",0,m.start())+1; le=s.find("\n",j)
        ctx=s[max(0,s.rfind("\n",0,ls-1)+1):le]
        sites.append(dict(id=len(sites),file=os.path.relpath(f,R),line=line,expr=expr,context=ctx))
json.dump(sites,open("sites.json","w"),indent=1)
print(len(sites))

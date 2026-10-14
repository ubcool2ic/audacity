import json,re,collections
R="/workspace/audacity-fork/rust/crates/au-app/src/"
sites={s["id"]:s for s in json.load(open("sites.json"))}
txt=open("out.txt").read(); txt=re.search(r"```json\n(.*?)```",txt,re.S).group(1)
dec=json.loads(txt)
byf=collections.defaultdict(list); n=0;nf=0
for d in dec:
    if d["action"]!="replace": continue
    s=sites[d["id"]]; byf[s["file"]].append((s,d["new"])); nf+=d.get("msgid_from_audacity",False)
for f,lst in byf.items():
    src=open(R+f).read()
    # offsets of line starts
    edits=[]
    for s,new in lst:
        ls=0
        for _ in range(s["line"]-1): ls=src.index("\n",ls)+1
        i=src.index(s["expr"],ls); edits.append((i,len(s["expr"]),new))
    for i,l,new in sorted(edits,reverse=True): src=src[:i]+new+src[i+l:]; n+=1
    open(R+f,"w").write(src)
print("replaced",n,"audacity msgids",nf,"skipped",sum(d["action"]=="skip" for d in dec))

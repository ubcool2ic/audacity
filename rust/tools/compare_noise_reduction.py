import numpy as np, subprocess, sys, os, json
sys.path.insert(0,os.path.dirname(os.path.abspath(__file__))); from compare_effects import write_wav, read_wav, db
W=sys.argv[1] if len(sys.argv)>1 else '/tmp/nrcmp'; R=os.path.dirname(os.path.dirname(os.path.abspath(__file__))); T=R+'/target/release/examples/fx_tool'
"""Noise Reduction vs Audacity 3.7.3: reference = tools/aud_ref/noise_reduction_ref.cpp (NoiseReductionBase worker code
verbatim + Audacity's installed lib-fft.so SpectrumTransformer)."""
os.makedirs(W, exist_ok=True)
if not os.path.exists('/tmp/nr_ref'):
    subprocess.run(['g++','-O1','-std=c++17','-o','/tmp/nr_ref',R+'/tools/aud_ref/noise_reduction_ref.cpp','-L/usr/lib/audacity','-l:lib-fft.so','-Wl,-rpath,/usr/lib/audacity'],check=True)
rng=np.random.default_rng(1); sr=44100
n=(rng.standard_normal(sr*2)*0.05).astype('<f4'); n.tofile(f'{W}/noise.f32')
t=np.arange(sr*3)/sr; x=(0.3*np.sin(2*np.pi*1000*t)+rng.standard_normal(sr*3)*0.05).astype('<f4'); x.tofile(f'{W}/nrin.f32')
write_wav(f'{W}/noise.wav',n[None]); write_wav(f'{W}/nrin.wav',x[None])
cases=[(12,6,3,0),(6,6,6,0),(24,12,0,0),(12,6,3,1),(12,6,3,2),(18,3,9,0)]
for (r,s,f,c) in cases:
    ref=np.frombuffer(subprocess.run(['/tmp/nr_ref',f'{W}/noise.f32',f'{W}/nrin.f32','44100','1',str(r),str(s),str(f),str(c)],capture_output=True,check=True).stdout,'<f4')
    subprocess.run([T,'apply','NoiseReduction',f'{W}/nrin.wav',f'{W}/nr.rs.wav',f'NoiseReduction={r}',f'Sensitivity={s}',f'FrequencySmoothing={f}',f'NoiseReductionChoice={c}',f'profile={W}/noise.wav'],check=True,capture_output=True)
    b=read_wav(f'{W}/nr.rs.wav')[0]
    m=min(len(ref),len(b)); d=ref[:m]-b[:m]
    print(json.dumps(dict(params=(r,s,f,c),len_ref=len(ref),len_rust=len(b),rms_diff_db=float(round(db(np.sqrt(np.mean(d**2))),1)),peak=float(round(db(np.max(np.abs(d))),1)))))

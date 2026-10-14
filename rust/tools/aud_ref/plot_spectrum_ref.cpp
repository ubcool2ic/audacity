// Reference: links Audacity 3.7.3 lib-fft.so SpectrumAnalyst::Calculate, mirrors
// PlotSpectrumBase::GetAudio (sum of channels) and FreqWindow::OnExport text.
// usage: plot_spectrum_ref in.f32raw nch rate alg func size > out.txt  (interleaved f32)
#include <cstdio>
#include <cstdlib>
#include <vector>
#include <functional>
#define FFT_API
class SpectrumAnalyst {
public:
   enum Algorithm { Spectrum, Autocorrelation, CubeRootAutocorrelation, EnhancedAutocorrelation, Cepstrum, NumAlgorithms };
   using ProgressFn = std::function<void(long long, long long)>;
   SpectrumAnalyst(); ~SpectrumAnalyst();
   bool Calculate(Algorithm alg, int windowFunc, size_t windowSize, double rate,
      const float *data, size_t dataLen, float *pYMin = NULL, float *pYMax = NULL, ProgressFn progress = NULL);
   const float *GetProcessed() const; int GetProcessedSize() const;
private: Algorithm mAlg; double mRate; size_t mWindowSize; std::vector<float> mProcessed;
};
int main(int argc,char**argv){
  FILE*f=fopen(argv[1],"rb"); int nch=atoi(argv[2]); double rate=atof(argv[3]);
  int alg=atoi(argv[4]),func=atoi(argv[5]); size_t ws=atol(argv[6]);
  std::vector<float> in; float x; while(fread(&x,4,1,f)==1) in.push_back(x);
  size_t n=in.size()/nch; std::vector<float> d(n,0.f);
  for(size_t i=0;i<n;i++){ d[i]=in[i*nch]; for(int c=1;c<nch;c++) d[i]+=in[i*nch+c]; }
  SpectrumAnalyst a; float ymin,ymax;
  if(n<ws){ return 2; }
  a.Calculate((SpectrumAnalyst::Algorithm)alg,func,ws,rate,d.data(),n,&ymin,&ymax);
  int ps=a.GetProcessedSize(); const float*p=a.GetProcessed();
  if(alg==0){ printf("Frequency (Hz)\tLevel (dB)\n"); for(int i=1;i<ps;i++) printf("%f\t%f\n", i*rate/ws, p[i]); }
  else { printf("Lag (seconds)\tFrequency (Hz)\tLevel\n"); for(int i=1;i<ps;i++) printf("%f\t%f\t%f\n", i/rate, rate/i, p[i]); }
}

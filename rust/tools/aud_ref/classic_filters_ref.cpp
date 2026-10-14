// Reference runner for Audacity 3.7.3 Classic Filters. Audacity 3.7.3 segfaults when Classic Filters
// is run from mod-script-pipe/macros (ScienFilterBase::ProcessInitialize -> Biquad::Reset on a null
// mpBiquad, because CalcFilter only ran before Init set mNyquist). This links the very lib-math.so the
// installed Audacity uses and replays ScienFilterBase::CalcFilter + ProcessBlock (per channel, biquads
// in series in place) exactly. Usage: classic_filters_ref type subtype order cutoff ripple stopband rate < f32 > f32
#include <memory>
#include <cstdio>
#include <cstdlib>
#include <vector>
template<typename X> class ArrayOf : public std::unique_ptr<X[]> { public: ArrayOf() {} ArrayOf(ArrayOf&&) = default; };
struct Biquad {
   Biquad(); void Reset(); void Process(const float* pfIn, float* pfOut, int iNumSamples);
   double fNumerCoeffs[3]; double fDenomCoeffs[2]; double fPrevIn, fPrevPrevIn, fPrevOut, fPrevPrevOut;
   static ArrayOf<Biquad> CalcButterworthFilter(int order, double fn, double fc, int type);
   static ArrayOf<Biquad> CalcChebyshevType1Filter(int order, double fn, double fc, double ripple, int type);
   static ArrayOf<Biquad> CalcChebyshevType2Filter(int order, double fn, double fc, double ripple, int type);
};
int main(int argc, char** argv) {
   int type = atoi(argv[1]), sub = atoi(argv[2]), order = atoi(argv[3]);
   float cutoff = atof(argv[4]), ripple = atof(argv[5]), stop = atof(argv[6]); double rate = atof(argv[7]);
   std::vector<float> x; float v; while (fread(&v, 4, 1, stdin) == 1) x.push_back(v);
   double nyq = rate / 2.0;
   ArrayOf<Biquad> bq = type == 0 ? Biquad::CalcButterworthFilter(order, nyq, cutoff, sub)
      : type == 1 ? Biquad::CalcChebyshevType1Filter(order, nyq, cutoff, ripple, sub)
      : Biquad::CalcChebyshevType2Filter(order, nyq, cutoff, stop, sub);
   for (int i = 0; i < (order + 1) / 2; i++) bq[i].Reset();
   std::vector<float> out(x.size());
   const size_t block = 262144; // PerTrackEffect block (track max block size); biquads are streaming anyway
   for (size_t p = 0; p < x.size(); p += block) {
      size_t n = std::min(block, x.size() - p); const float* ib = &x[p];
      for (int i = 0; i < (order + 1) / 2; i++) { bq[i].Process(ib, &out[p], (int)n); ib = &out[p]; }
   }
   fwrite(out.data(), 4, out.size(), stdout);
}

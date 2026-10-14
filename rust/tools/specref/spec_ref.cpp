// Reference spectrogram columns using Audacity 3.7.3's own lib-fft.so and the
// SpectrumCache/SpectrogramSettings code (CalculateOneSpectrum, Frequencies and
// Pitch (EAC) paths), copied verbatim. Build: see build.sh.
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <vector>
template<typename T> using ArrayOf = std::unique_ptr<T[]>;
struct FFTParam { ArrayOf<int> BitReversed; ArrayOf<float> SinTable; size_t Points; };
struct FFTDeleter { void operator()(FFTParam*) const; };
using HFFT = std::unique_ptr<FFTParam, FFTDeleter>;
HFFT GetFFT(size_t);
void RealFFTf(float*, const FFTParam*);
void NewWindowFunc(int, size_t, bool, float*);
bool ComputeSpectrum(const float*, size_t, size_t, float*, bool, int);

static void ComputeSpectrumUsingRealFFTf(float* buffer, const FFTParam* hFFT, const float* window, size_t len, float* out) {
   size_t i;
   if (len > hFFT->Points * 2) len = hFFT->Points * 2;
   for (i = 0; i < len; i++) buffer[i] *= window[i];
   for (; i < (hFFT->Points * 2); i++) buffer[i] = 0;
   RealFFTf(buffer, hFFT);
   float power = buffer[0] * buffer[0];
   if (power <= 0) out[0] = -160.0; else out[0] = 10.0 * log10f(power);
   for (i = 1; i < hFFT->Points; i++) {
      const int index = hFFT->BitReversed[i];
      const float re = buffer[index], im = buffer[index + 1];
      power = re * re + im * im;
      if (power <= 0) out[i] = -160.0; else out[i] = 10.0 * log10f(power);
   }
}

int main(int argc, char** argv) {
   if (argc < 10) { fprintf(stderr, "usage: in.f32 rate win zp wtype fgain alg positions.txt out.f32\n"); return 2; }
   FILE* f = fopen(argv[1], "rb"); std::vector<float> s; float v; while (fread(&v, 4, 1, f) == 1) s.push_back(v); fclose(f);
   double rate = atof(argv[2]); size_t windowSize = atoi(argv[3]), zp = atoi(argv[4]);
   int windowType = atoi(argv[5]), frequencyGain = atoi(argv[6]), alg = atoi(argv[7]);
   std::vector<long long> where; f = fopen(argv[8], "r"); long long w; while (fscanf(f, "%lld", &w) == 1) where.push_back(w); fclose(f);
   const size_t fftLen = windowSize * zp, padding = (windowSize * (zp - 1)) / 2;
   const size_t nBins = alg == 2 ? windowSize / 2 : fftLen / 2;  // NBins(): EAC uses windowSize/2
   HFFT hFFT = GetFFT(fftLen);
   // RecreateWindow(WINDOW)
   std::vector<float> window(fftLen); double scale; size_t ii; size_t ws = windowSize;
   const bool extra = padding > 0; if (extra) ++ws; const size_t endOfWindow = padding + ws;
   for (ii = 0; ii < padding; ++ii) { window[ii] = 0.0; window[fftLen - ii - 1] = 0.0; }
   for (; ii < endOfWindow; ++ii) window[ii] = 1.0;
   NewWindowFunc(windowType, ws, extra, window.data() + padding);
   scale = 0.0; for (ii = padding; ii < endOfWindow; ++ii) scale += window[ii];
   if (scale > 0) scale = 2.0 / scale;
   for (ii = padding; ii < endOfWindow; ++ii) window[ii] *= scale;
   // gain factors
   std::vector<float> gainFactors;
   if (frequencyGain > 0) {
      const double factor = ((double)rate / (double)fftLen) / 1000.0; auto half = fftLen / 2;
      gainFactors.push_back(frequencyGain * log10(factor));
      for (size_t x = 1; x < half; x++) gainFactors.push_back(frequencyGain * log10(factor * x));
   }
   const long long numSamples = s.size();
   std::vector<float> out(nBins * where.size()), scratch(3 * fftLen);
   for (size_t xx = 0; xx < where.size(); ++xx) {
      long long from = where[xx];
      float* results = &out[nBins * xx];
      if (from < 0 || from >= numSamples) { std::fill(results, results + nBins, 0.0f); continue; }
      const bool autocorrelation = alg == 2;
      bool copy = !autocorrelation || (padding > 0);
      float* adj = scratch.data() + padding; const float* useBuffer = nullptr;
      size_t myLen = windowSize; from -= windowSize >> 1;
      if (from < 0) { for (auto i = from; i < 0; ++i) *adj++ = 0; myLen += from; from = 0; copy = true; }
      if (from + (long long)myLen >= numSamples) { size_t newlen = numSamples - from; for (size_t i = newlen; i < myLen; ++i) adj[i] = 0; myLen = newlen; copy = true; }
      if (myLen > 0) { useBuffer = s.data() + from; if (copy) memcpy(adj, useBuffer, myLen * sizeof(float)); }
      if (copy || !useBuffer) useBuffer = scratch.data();
      if (autocorrelation) ComputeSpectrum(useBuffer, windowSize, windowSize, results, true, windowType);
      else {
         ComputeSpectrumUsingRealFFTf((float*)useBuffer, hFFT.get(), window.data(), fftLen, results);
         if (!gainFactors.empty()) for (size_t i = 0; i < nBins; ++i) results[i] += gainFactors[i];
      }
   }
   f = fopen(argv[9], "wb"); fwrite(out.data(), 4, out.size(), f); fclose(f);
   printf("%zu %zu\n", where.size(), nBins);
}

// Reference spectrogram columns using Audacity 3.7.3's own lib-fft.so and the
// SpectrumCache/SpectrogramSettings code (CalculateOneSpectrum, Frequencies,
// Pitch (EAC), and Reassignment paths), copied as literally as possible.
// Build: see build.sh.
#include <algorithm>
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
void DerivativeOfWindowFunc(int, size_t, bool, float*);
bool ComputeSpectrum(const float*, size_t, size_t, float*, bool, int);

static void ComputeSpectrumUsingRealFFTf(
   float* buffer, const FFTParam* hFFT, const float* window, size_t len,
   float* out)
{
   size_t i;
   if (len > hFFT->Points * 2) len = hFFT->Points * 2;
   for (i = 0; i < len; i++) buffer[i] *= window[i];
   for (; i < (hFFT->Points * 2); i++) buffer[i] = 0;
   RealFFTf(buffer, hFFT);
   float power = buffer[0] * buffer[0];
   if (power <= 0) out[0] = -160.0;
   else out[0] = 10.0 * log10f(power);
   for (i = 1; i < hFFT->Points; i++) {
      const int index = hFFT->BitReversed[i];
      const float re = buffer[index], im = buffer[index + 1];
      power = re * re + im * im;
      if (power <= 0) out[i] = -160.0;
      else out[i] = 10.0 * log10f(power);
   }
}

int main(int argc, char** argv)
{
   if (argc < 10) {
      fprintf(stderr,
         "usage: in.f32 rate win zp wtype fgain alg positions.txt out.f32 [pixelsPerSecond]\n");
      return 2;
   }

   FILE* f = fopen(argv[1], "rb");
   std::vector<float> s;
   float v;
   while (fread(&v, 4, 1, f) == 1) s.push_back(v);
   fclose(f);

   double rate = atof(argv[2]);
   size_t windowSize = atoi(argv[3]), zp = atoi(argv[4]);
   int windowType = atoi(argv[5]), frequencyGain = atoi(argv[6]);
   int alg = atoi(argv[7]);

   if (alg == 1 && argc < 11) {
      fprintf(stderr,
         "usage: in.f32 rate win zp wtype fgain alg positions.txt out.f32 pixelsPerSecond\n");
      return 2;
   }
   const double pixelsPerSecond = alg == 1 ? atof(argv[10]) : 0.0;

   std::vector<long long> where;
   f = fopen(argv[8], "r");
   long long w;
   while (fscanf(f, "%lld", &w) == 1) where.push_back(w);
   fclose(f);

   const size_t fftLen = windowSize * zp;
   const size_t padding = (windowSize * (zp - 1)) / 2;
   const size_t numPixels = alg == 1
      ? (where.empty() ? 0 : where.size() - 1)
      : where.size();
   const size_t nBins = alg == 2 ? windowSize / 2 : fftLen / 2;
   HFFT hFFT = GetFFT(fftLen);

   // RecreateWindow(WINDOW)
   std::vector<float> window(fftLen);
   double scale;
   size_t ii;
   size_t ws = windowSize;
   const bool extra = padding > 0;
   if (extra) ++ws;
   const size_t endOfWindow = padding + ws;
   for (ii = 0; ii < padding; ++ii) {
      window[ii] = 0.0;
      window[fftLen - ii - 1] = 0.0;
   }
   for (; ii < endOfWindow; ++ii) window[ii] = 1.0;
   NewWindowFunc(windowType, ws, extra, window.data() + padding);
   scale = 0.0;
   for (ii = padding; ii < endOfWindow; ++ii) scale += window[ii];
   if (scale > 0) scale = 2.0 / scale;
   for (ii = padding; ii < endOfWindow; ++ii) window[ii] *= scale;

   std::vector<float> tWindow, dWindow;
   if (alg == 1) {
      // RecreateWindow(TWINDOW), using the scale established by WINDOW.
      tWindow.resize(fftLen);
      ii = 0;
      for (ii = 0; ii < padding; ++ii) {
         tWindow[ii] = 0.0;
         tWindow[fftLen - ii - 1] = 0.0;
      }
      for (; ii < endOfWindow; ++ii) tWindow[ii] = 1.0;
      NewWindowFunc(windowType, ws, extra, tWindow.data() + padding);
      for (int jj = static_cast<int>(padding),
         multiplier = -static_cast<int>(ws) / 2;
         jj < static_cast<int>(endOfWindow); ++jj, ++multiplier)
         tWindow[jj] *= multiplier;
      for (ii = padding; ii < endOfWindow; ++ii) tWindow[ii] *= scale;

      // RecreateWindow(DWINDOW), using the scale established by WINDOW.
      dWindow.resize(fftLen);
      ii = 0;
      for (ii = 0; ii < padding; ++ii) {
         dWindow[ii] = 0.0;
         dWindow[fftLen - ii - 1] = 0.0;
      }
      for (; ii < endOfWindow; ++ii) dWindow[ii] = 1.0;
      DerivativeOfWindowFunc(windowType, ws, extra, dWindow.data() + padding);
      for (ii = padding; ii < endOfWindow; ++ii) dWindow[ii] *= scale;
   }

   // gain factors
   std::vector<float> gainFactors;
   if (frequencyGain > 0) {
      const double factor = ((double)rate / (double)fftLen) / 1000.0;
      auto half = fftLen / 2;
      gainFactors.push_back(frequencyGain * log10(factor));
      for (size_t x = 1; x < half; x++)
         gainFactors.push_back(frequencyGain * log10(factor * x));
   }

   const long long numSamples = static_cast<long long>(s.size());

   if (alg == 1) {
      static const double epsilon = 1e-16;
      std::vector<float> freq(nBins * numPixels, 0.0f);
      std::vector<float> scratch(3 * fftLen);

      const double samplesPerPixel = rate / pixelsPerSecond;

      auto CalculateOneSpectrum =
         [&](int xx, int lowerBoundX, int upperBoundX) {
            bool result = false;
            long long from;
            if (xx < 0)
               from = static_cast<long long>(
                  where[0] + xx * samplesPerPixel);
            else if (xx > static_cast<int>(numPixels))
               from = static_cast<long long>(
                  where[numPixels] + (xx - static_cast<int>(numPixels)) *
                     samplesPerPixel);
            else
               from = where[xx];

            if (from < 0 || from >= numSamples) {
               if (xx >= 0 && xx < static_cast<int>(numPixels)) {
                  float* const results = &freq[nBins * xx];
                  std::fill(results, results + nBins, 0.0f);
               }
            }
            else {
               bool copy = true;
               float* adj = scratch.data() + padding;
               // SPECREF_ZERO: clear scratch per column. Audacity reuses it, so with zero padding the
               // extra window sample (windowSize+1) multiplies the previous column's FFT output in
               // dWindow (non-zero there for Hamming/Welch/Gaussian); we do not reproduce that artifact.
               if (getenv("SPECREF_ZERO")) std::fill(scratch.begin(), scratch.end(), 0.0f);
               float* useBuffer = nullptr;

               size_t myLen = windowSize;
               from -= static_cast<long long>(windowSize >> 1);
               if (from < 0) {
                  for (auto jj = from; jj < 0; ++jj) *adj++ = 0;
                  myLen += static_cast<size_t>(from);
                  from = 0;
                  copy = true;
               }

               if (from + static_cast<long long>(myLen) >= numSamples) {
                  const size_t newlen = static_cast<size_t>(numSamples - from);
                  for (size_t jj = newlen; jj < myLen; ++jj) adj[jj] = 0;
                  myLen = newlen;
                  copy = true;
               }

               if (myLen > 0) {
                  useBuffer = s.data() + from;
                  if (copy)
                     memcpy(adj, useBuffer, myLen * sizeof(float));
               }

               if (copy || !useBuffer) useBuffer = scratch.data();

               float* const scratch2 = scratch.data() + fftLen;
               std::copy(scratch.data(), scratch2, scratch2);

               float* const scratch3 = scratch.data() + 2 * fftLen;
               std::copy(scratch.data(), scratch2, scratch3);

               {
                  const float* const useWindow = window.data();
                  for (size_t jj = 0; jj < fftLen; ++jj)
                     scratch[jj] *= useWindow[jj];
                  RealFFTf(scratch.data(), hFFT.get());
               }

               {
                  const float* const useDWindow = dWindow.data();
                  for (size_t jj = 0; jj < fftLen; ++jj)
                     scratch2[jj] *= useDWindow[jj];
                  RealFFTf(scratch2, hFFT.get());
               }

               {
                  const float* const useTWindow = tWindow.data();
                  for (size_t jj = 0; jj < fftLen; ++jj)
                     scratch3[jj] *= useTWindow[jj];
                  RealFFTf(scratch3, hFFT.get());
               }

               for (size_t jj = 0; jj < hFFT->Points; ++jj) {
                  const int index = hFFT->BitReversed[jj];
                  const float denomRe = scratch[index];
                  const float denomIm = jj == 0 ? 0 : scratch[index + 1];
                  const double power =
                     denomRe * denomRe + denomIm * denomIm;
                  if (power < epsilon) continue;

                  double freqCorrection;
                  {
                     const double multiplier = -(fftLen / (2.0f * M_PI));
                     const float numRe = scratch2[index];
                     const float numIm = jj == 0 ? 0 : scratch2[index + 1];
                     const double quotIm =
                        (-numRe * denomIm + numIm * denomRe) / power;
                     freqCorrection = multiplier * quotIm;
                  }

                  const int bin =
                     static_cast<int>(static_cast<int>(jj) +
                        freqCorrection + 0.5f);
                  if (bin >= 0 && bin < static_cast<int>(hFFT->Points)) {
                     double timeCorrection;
                     {
                        const float numRe = scratch3[index];
                        const float numIm =
                           jj == 0 ? 0 : scratch3[index + 1];
                        timeCorrection =
                           (numRe * denomRe + numIm * denomIm) / power;
                     }

                     const int correctedX = static_cast<int>(floor(
                        0.5 + xx + timeCorrection * pixelsPerSecond / rate));
                     if (correctedX >= lowerBoundX &&
                         correctedX < upperBoundX) {
                        result = true;
                        const auto ind =
                           static_cast<int>(nBins) * correctedX + bin;
                        freq[ind] += static_cast<float>(power);
                     }
                  }
               }
            }
            return result;
         };

      // copyBegin=copyEnd=0: process both ranges, including the empty range.
      for (int jj = 0; jj < 2; ++jj) {
         const int lowerBoundX = jj == 0 ? 0 : 0;
         const int upperBoundX = jj == 0 ? 0 : static_cast<int>(numPixels);

         for (int xx = lowerBoundX; xx < upperBoundX; ++xx)
            CalculateOneSpectrum(xx, lowerBoundX, upperBoundX);

         const double pixelsPerSample = pixelsPerSecond / rate;
         const int limit = std::min(
            static_cast<int>(0.5 + fftLen * pixelsPerSample), 100);

         int xx = lowerBoundX;
         for (int ii = 0; ii < limit; ++ii) {
            const bool result =
               CalculateOneSpectrum(--xx, lowerBoundX, upperBoundX);
            if (!result) break;
         }

         xx = upperBoundX;
         for (int ii = 0; ii < limit; ++ii) {
            const bool result =
               CalculateOneSpectrum(xx++, lowerBoundX, upperBoundX);
            if (!result) break;
         }

         for (xx = lowerBoundX; xx < upperBoundX; ++xx) {
            float* const results = &freq[nBins * xx];
            for (size_t ii = 0; ii < nBins; ++ii) {
               float& power = results[ii];
               if (power <= 0) power = -160.0;
               else power = 10.0 * log10f(power);
            }
            if (!gainFactors.empty()) {
               for (size_t ii = 0; ii < nBins; ++ii)
                  results[ii] += gainFactors[ii];
            }
         }
      }

      f = fopen(argv[9], "wb");
      fwrite(freq.data(), 4, freq.size(), f);
      fclose(f);
      printf("%zu %zu\n", numPixels, nBins);
      return 0;
   }

   std::vector<float> out(nBins * where.size()), scratch(3 * fftLen);
   for (size_t xx = 0; xx < where.size(); ++xx) {
      long long from = where[xx];
      float* results = &out[nBins * xx];
      if (from < 0 || from >= numSamples) {
         std::fill(results, results + nBins, 0.0f);
         continue;
      }
      const bool autocorrelation = alg == 2;
      bool copy = !autocorrelation || (padding > 0);
      float* adj = scratch.data() + padding;
      const float* useBuffer = nullptr;
      size_t myLen = windowSize;
      from -= windowSize >> 1;
      if (from < 0) {
         for (auto i = from; i < 0; ++i) *adj++ = 0;
         myLen += from;
         from = 0;
         copy = true;
      }
      if (from + static_cast<long long>(myLen) >= numSamples) {
         size_t newlen = static_cast<size_t>(numSamples - from);
         for (size_t i = newlen; i < myLen; ++i) adj[i] = 0;
         myLen = newlen;
         copy = true;
      }
      if (myLen > 0) {
         useBuffer = s.data() + from;
         if (copy) memcpy(adj, useBuffer, myLen * sizeof(float));
      }
      if (copy || !useBuffer) useBuffer = scratch.data();
      if (autocorrelation)
         ComputeSpectrum(useBuffer, windowSize, windowSize, results, true,
            windowType);
      else {
         ComputeSpectrumUsingRealFFTf(
            (float*)useBuffer, hFFT.get(), window.data(), fftLen, results);
         if (!gainFactors.empty())
            for (size_t i = 0; i < nBins; ++i)
               results[i] += gainFactors[i];
      }
   }

   f = fopen(argv[9], "wb");
   fwrite(out.data(), 4, out.size(), f);
   fclose(f);
   printf("%zu %zu\n", where.size(), nBins);
}

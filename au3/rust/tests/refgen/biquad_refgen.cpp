// biquad_refgen.cpp -- writes tests/data/biquad_ref.bin from the ORIGINAL,
// unmodified au3/libraries/au3-math/Biquad.cpp.
//
// Binary format (little endian):
//   int32   ncases
//   ncases x {
//     int32 kind            // 0 butterworth, 1 cheby1, 2 cheby2
//     int32 order
//     int32 subtype         // 0 lowpass, 1 highpass
//     int32 nsections
//     nsections x { double B0, B1, B2, A1, A2 }   // state is zeroed
//     float[256] output     // cascade output for the fixed input below
//   }
//   int32   ncheby         // 40
//   ncheby x { int32 order, double x, double cheby_poly(order, x) }
//
// Fixed input: x[n] = sin(0.05*n) + 0.3*sin(1.7*n), computed in double and
// narrowed to float; each section processes the block in place.
//
// Build (from au3/rust/tests/refgen):
//   mkdir -p stubs/wx stubs/au3-utility
//   cat > stubs/wx/utils.h <<'EOF'
//   #pragma once
//   #include <cmath>
//   #define wxMax(a,b) ((a)>(b)?(a):(b))
//   #define wxASSERT(x) ((void)0)
//   #define DB_TO_LINEAR(x) pow(10.0,(x)/20.0)
//   EOF
//   cat > stubs/au3-utility/MemoryX.h <<'EOF'
//   #pragma once
//   #include <cstddef>
//   #include <vector>
//   #define MATH_API
//   template<typename T> class ArrayOf {
//      std::vector<T> mData;
//   public:
//      ArrayOf() = default;
//      explicit ArrayOf(std::size_t n, bool initialize = false) : mData(n) {
//         if (initialize) for (auto& e : mData) e = T();
//      }
//      T& operator[](std::size_t i) { return mData[i]; }
//      const T& operator[](std::size_t i) const { return mData[i]; }
//      std::size_t size() const { return mData.size(); }
//   };
//   EOF
//   g++ -std=c++17 -O2 -I. -Istubs -I../../../libraries/au3-math \
//       biquad_refgen.cpp -o biquad_refgen
//   ./biquad_refgen ../../tests/data/biquad_ref.bin

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>

#include "Biquad.cpp"

namespace
{
FILE* gOut = nullptr;

void WrI32(int32_t v) { std::fwrite(&v, sizeof(v), 1, gOut); }
void WrF64(double v)  { std::fwrite(&v, sizeof(v), 1, gOut); }
void WrF32(float v)   { std::fwrite(&v, sizeof(v), 1, gOut); }

ArrayOf<Biquad> Design(int kind, int order, double fn, double fc, double ripple, int subtype)
{
   switch (kind) {
   case 0: return Biquad::CalcButterworthFilter(order, fn, fc, subtype);
   case 1: return Biquad::CalcChebyshevType1Filter(order, fn, fc, ripple, subtype);
   default: return Biquad::CalcChebyshevType2Filter(order, fn, fc, ripple, subtype);
   }
}
}

int main(int argc, char** argv)
{
   const char* path = (argc > 1) ? argv[1] : "biquad_ref.bin";
   gOut = std::fopen(path, "wb");
   if (!gOut) {
      std::perror(path);
      return 1;
   }

   const int kinds[] = { 0, 1, 2 };
   const int orders[] = { 1, 2, 3, 5, 8, 10 };
   const int subtypes[] = { 0, 1 };
   const double fN = 44100.0;
   const double fC = 1000.0;

   WrI32(3 * 6 * 2);
   for (int kind : kinds) {
      for (int order : orders) {
         for (int subtype : subtypes) {
            const double ripple = (kind == 1) ? 1.0 : ((kind == 2) ? 40.0 : 0.0);
            ArrayOf<Biquad> sections = Design(kind, order, fN, fC, ripple, subtype);
            const int ns = (order + 1) / 2;

            WrI32(kind);
            WrI32(order);
            WrI32(subtype);
            WrI32(ns);
            for (int s = 0; s < ns; ++s) {
               for (int k = 0; k < 3; ++k) WrF64(sections[s].fNumerCoeffs[k]);
               for (int k = 0; k < 2; ++k) WrF64(sections[s].fDenomCoeffs[k]);
            }

            float buf[256];
            for (int n = 0; n < 256; ++n)
               buf[n] = float(std::sin(0.05 * n) + 0.3 * std::sin(1.7 * n));
            for (int s = 0; s < ns; ++s)
               sections[s].Process(buf, buf, 256);
            for (int n = 0; n < 256; ++n) WrF32(buf[n]);
         }
      }
   }

   const double xs[] = { 0.1, 0.5, 0.9, 1.5 };
   WrI32(40);
   for (int order = 1; order <= 10; ++order) {
      for (double x : xs) {
         WrI32(order);
         WrF64(x);
         WrF64(Biquad::ChebyPoly(order, x));
      }
   }

   std::fclose(gOut);
   return 0;
}

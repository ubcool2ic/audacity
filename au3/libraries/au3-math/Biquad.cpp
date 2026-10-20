/**********************************************************************

Audacity: A Digital Audio Editor

Biquad.cpp

Norm C
Max Maisel

***********************************************************************/

#include "Biquad.h"

#include "au3_rs_math.h"

namespace
{
// Runs the Rust filter designer and copies the result into the ArrayOf the
// C++ API returns.
ArrayOf<Biquad> Transfer(int kind, int order, double fn, double fc, double ripple, int subtype)
{
   Biquad sections[(Biquad::MAX_Order + 1) / 2];
   const int n = au3rs_biquad_calc_filter(kind, order, fn, fc, ripple, subtype, sections);
   ArrayOf<Biquad> result(size_t(n), true);
   for (int i = 0; i < n; ++i) {
      result[i] = sections[i];
   }
   return result;
}
}

Biquad::Biquad()
{
   fNumerCoeffs[B0] = 1;
   fNumerCoeffs[B1] = 0;
   fNumerCoeffs[B2] = 0;
   fDenomCoeffs[A1] = 0;
   fDenomCoeffs[A2] = 0;
   Reset();
}

void Biquad::Reset()
{
   au3rs_biquad_reset(this);
}

void Biquad::Process(const float* pfIn, float* pfOut, int iNumSamples)
{
   au3rs_biquad_process(this, pfIn, pfOut, iNumSamples);
}

ArrayOf<Biquad> Biquad::CalcButterworthFilter(int order, double fn, double fc, int subtype)
{
   return Transfer(0, order, fn, fc, 0.0, subtype);
}

ArrayOf<Biquad> Biquad::CalcChebyshevType1Filter(int order, double fn, double fc, double ripple, int subtype)
{
   return Transfer(1, order, fn, fc, ripple, subtype);
}

ArrayOf<Biquad> Biquad::CalcChebyshevType2Filter(int order, double fn, double fc, double ripple, int subtype)
{
   return Transfer(2, order, fn, fc, ripple, subtype);
}

void Biquad::ComplexDiv(double fNumerR, double fNumerI, double fDenomR, double fDenomI,
                        double* pfQuotientR, double* pfQuotientI)
{
   au3rs_biquad_complex_div(fNumerR, fNumerI, fDenomR, fDenomI, pfQuotientR, pfQuotientI);
}

bool Biquad::BilinTransform(double fSX, double fSY, double* pfZX, double* pfZY)
{
   return au3rs_biquad_bilin_transform(fSX, fSY, pfZX, pfZY);
}

float Biquad::Calc2D_DistSqr(double fX1, double fY1, double fX2, double fY2)
{
   return au3rs_biquad_calc2d_distsqr(fX1, fY1, fX2, fY2);
}

double Biquad::ChebyPoly(int Order, double NormFreq)   // NormFreq = 1 at the f0 point (where response is R dB down)
{
   return au3rs_biquad_cheby_poly(Order, NormFreq);
}

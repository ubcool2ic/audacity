#ifndef AU3_RS_MATH_H
#define AU3_RS_MATH_H

#include <stdint.h>
#include <stdbool.h>

/* The Rust extern "C" entry points of the au3-rs crate (currently the Biquad
   port).  Usable from both C and C++. */

#ifdef __cplusplus
struct Biquad;
#define AU3RS_BIQUAD struct Biquad
extern "C" {
#else
typedef struct au3rs_biquad {
   double numer[3];
   double denom[2];
   double prev_in;
   double prev_prev_in;
   double prev_out;
   double prev_prev_out;
} au3rs_biquad;
#define AU3RS_BIQUAD au3rs_biquad
#endif

void    au3rs_biquad_reset(AU3RS_BIQUAD* b);
void    au3rs_biquad_process(AU3RS_BIQUAD* b, const float* in, float* out, int32_t n);
float   au3rs_biquad_process_one(AU3RS_BIQUAD* b, float x);

/* kind: 0 butterworth, 1 chebyshev type 1, 2 chebyshev type 2.
   subtype: 0 lowpass, 1 highpass.
   `out` must hold at least (10 + 1) / 2 = 5 sections.
   Returns the number of sections written. */
int32_t au3rs_biquad_calc_filter(int32_t kind, int32_t order, double fN,
                                 double fC, double ripple, int32_t subtype,
                                 AU3RS_BIQUAD* out);

double  au3rs_biquad_cheby_poly(int32_t order, double norm_freq);
double  au3rs_biquad_cheby_coeff(int32_t i, int32_t j);
void    au3rs_biquad_complex_div(double numer_r, double numer_i,
                                 double denom_r, double denom_i,
                                 double* quotient_r, double* quotient_i);
bool    au3rs_biquad_bilin_transform(double s_x, double s_y,
                                     double* z_x, double* z_y);
float   au3rs_biquad_calc2d_distsqr(double x1, double y1, double x2, double y2);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* AU3_RS_MATH_H */

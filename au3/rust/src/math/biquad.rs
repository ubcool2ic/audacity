//! Port of `au3/libraries/au3-math/Biquad.cpp`.
//!
//! All floating point arithmetic is done in `f64` in exactly the same order as
//! the C++ original (including the places where the original narrows to
//! `float`, e.g. `Calc2D_DistSqr` and the `sqrt()` calls on its result).

use std::ptr;

/// Smallest supported filter order.
pub const MIN_ORDER: i32 = 1;
/// Largest supported filter order.
pub const MAX_ORDER: usize = 10;

/// Numerator coefficient index B0.
pub const B0: usize = 0;
/// Numerator coefficient index B1.
pub const B1: usize = 1;
/// Numerator coefficient index B2.
pub const B2: usize = 2;
/// Denominator coefficient index A1.
pub const A1: usize = 0;
/// Denominator coefficient index A2.
pub const A2: usize = 1;

/// Low pass subtype (`Biquad::kLowPass`).
pub const K_LOW_PASS: i32 = 0;
/// High pass subtype (`Biquad::kHighPass`).
pub const K_HIGH_PASS: i32 = 1;

const PI: f64 = std::f64::consts::PI;

/// Chebyshev polynomials of the first kind, coefficients 0..=9 per order.
pub const S_F_CHEBY_COEFFS: [[f64; MAX_ORDER + 1]; MAX_ORDER] = [
    [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [-1.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [0.0, -3.0, 0.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [1.0, 0.0, -8.0, 0.0, 8.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [0.0, 5.0, 0.0, -20.0, 0.0, 16.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [-1.0, 0.0, 18.0, 0.0, -48.0, 0.0, 32.0, 0.0, 0.0, 0.0, 0.0],
    [0.0, -7.0, 0.0, 56.0, 0.0, -112.0, 0.0, 64.0, 0.0, 0.0, 0.0],
    [1.0, 0.0, -32.0, 0.0, 160.0, 0.0, -256.0, 0.0, 128.0, 0.0, 0.0],
    [0.0, 9.0, 0.0, -120.0, 0.0, 432.0, 0.0, -576.0, 0.0, 256.0, 0.0],
    [-1.0, 0.0, 50.0, 0.0, -400.0, 0.0, 1120.0, 0.0, -1280.0, 0.0, 512.0],
];

/// Represents a biquad digital filter (`#[repr(C)]` mirror of the C++ struct).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    /// B0 B1 B2
    pub numer: [f64; 3],
    /// A1 A2 (A0 == 1.0)
    pub denom: [f64; 2],
    /// Previous input sample.
    pub prev_in: f64,
    /// Input sample before that.
    pub prev_prev_in: f64,
    /// Previous output sample.
    pub prev_out: f64,
    /// Output sample before that.
    pub prev_prev_out: f64,
}

/// `#define square(a) ((a) * (a))`
#[inline]
fn square(a: f64) -> f64 {
    a * a
}

/// `DB_TO_LINEAR(x)`
#[inline]
fn db_to_linear(x: f64) -> f64 {
    10.0f64.powf(x / 20.0)
}

impl Biquad {
    /// Creates a biquad with unity numerator and cleared state.
    pub fn new() -> Self {
        let mut b = Biquad {
            numer: [1.0, 0.0, 0.0],
            denom: [0.0, 0.0],
            prev_in: 0.0,
            prev_prev_in: 0.0,
            prev_out: 0.0,
            prev_prev_out: 0.0,
        };
        b.reset();
        b
    }

    /// Clears the filter state.
    pub fn reset(&mut self) {
        self.prev_in = 0.0;
        self.prev_prev_in = 0.0;
        self.prev_out = 0.0;
        self.prev_prev_out = 0.0;
    }

    /// Processes one sample (`Biquad::ProcessOne`).
    pub fn process_one(&mut self, f_in: f32) -> f32 {
        let f_out = f64::from(f_in) * self.numer[B0]
            + self.prev_in * self.numer[B1]
            + self.prev_prev_in * self.numer[B2]
            - self.prev_out * self.denom[A1]
            - self.prev_prev_out * self.denom[A2];
        self.prev_prev_in = self.prev_in;
        self.prev_in = f64::from(f_in);
        self.prev_prev_out = self.prev_out;
        self.prev_out = f_out;
        f_out as f32
    }

    /// Processes a block of samples (`Biquad::Process`).
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        for (i, o) in input.iter().zip(output.iter_mut()) {
            *o = self.process_one(*i);
        }
    }
}

impl Default for Biquad {
    fn default() -> Self {
        Biquad::new()
    }
}

/// Complex division (`Biquad::ComplexDiv`).
pub fn complex_div(nr: f64, ni: f64, dr: f64, di: f64) -> (f64, f64) {
    let denom = square(dr) + square(di);
    (
        (nr * dr + ni * di) / denom,
        (ni * dr - nr * di) / denom,
    )
}

/// Bilinear transform (`Biquad::BilinTransform`).
pub fn bilin_transform(sx: f64, sy: f64) -> (f64, f64) {
    let denom = square(1.0 - sx) + square(sy);
    (
        (1.0 - square(sx) - square(sy)) / denom,
        2.0 * sy / denom,
    )
}

/// Squared 2-D distance; returns `float` like the original.
pub fn calc2d_dist_sqr(x1: f64, y1: f64, x2: f64, y2: f64) -> f32 {
    (square(x1 - x2) + square(y1 - y2)) as f32
}

/// Evaluates the Chebyshev polynomial (`Biquad::ChebyPoly`).
pub fn cheby_poly(order: i32, norm_freq: f64) -> f64 {
    if order < MIN_ORDER || order as usize > MAX_ORDER {
        return 0.0;
    }
    let o = order as usize;
    let mut x = 1.0f64;
    let mut sum = 0.0f64;
    for i in 0..=o {
        sum += S_F_CHEBY_COEFFS[o - 1][i] * x;
        x *= norm_freq;
    }
    sum
}

/// Reads one entry of the Chebyshev coefficient table (replaces the C++ array).
pub fn cheby_coeff(i: i32, j: i32) -> f64 {
    if i < MIN_ORDER || i as usize > MAX_ORDER || j < 0 || j as usize > MAX_ORDER {
        return 0.0;
    }
    S_F_CHEBY_COEFFS[(i - 1) as usize][j as usize]
}

/// `Biquad::CalcButterworthFilter`.
pub fn calc_butterworth_filter(order: i32, f_n: f64, fc: f64, subtype: i32) -> Vec<Biquad> {
    if order < MIN_ORDER || order as usize > MAX_ORDER {
        return Vec::new();
    }
    let sections = ((order + 1) / 2) as usize;
    let mut p = vec![Biquad::new(); sections];

    let mut f_norm = fc / f_n;
    if f_norm >= 0.9999 {
        f_norm = f64::from(0.9999f32);
    }
    let f_c = (PI * f_norm / 2.0).tan();
    let mut dc = 1.0f64;
    let low = subtype == K_LOW_PASS;
    let mut zx;
    let mut zy;

    if (order & 1) == 0 {
        for ipair in 0..(order / 2) as usize {
            let ang = PI - (ipair as f64 + 0.5) * PI / order as f64;
            let spx = f_c * ang.cos();
            let spy = f_c * ang.sin();
            let (x, y) = bilin_transform(spx, spy);
            zx = x;
            zy = y;
            let b = &mut p[ipair];
            b.numer[B0] = 1.0;
            b.numer[B1] = if low { 2.0 } else { -2.0 };
            b.numer[B2] = 1.0;
            b.denom[A1] = -2.0 * zx;
            b.denom[A2] = square(zx) + square(zy);
            if low {
                dc *= f64::from(calc2d_dist_sqr(1.0, 0.0, zx, zy));
            } else {
                dc *= f64::from(calc2d_dist_sqr(-1.0, 0.0, zx, zy));
            }
        }
    } else {
        let (x, y) = bilin_transform(-f_c, 0.0);
        zx = x;
        zy = y;
        p[0].numer[B0] = 1.0;
        p[0].numer[B1] = if low { 1.0 } else { -1.0 };
        p[0].numer[B2] = 0.0;
        p[0].denom[A1] = -zx;
        p[0].denom[A2] = 0.0;
        if low {
            dc = 1.0 - zx;
        } else {
            dc = zx + 1.0;
        }
        for ipair in 1..=(order / 2) as usize {
            let ang = PI - ipair as f64 * PI / order as f64;
            let spx = f_c * ang.cos();
            let spy = f_c * ang.sin();
            let (x, y) = bilin_transform(spx, spy);
            zx = x;
            zy = y;
            let b = &mut p[ipair];
            b.numer[B0] = 1.0;
            b.numer[B1] = if low { 2.0 } else { -2.0 };
            b.numer[B2] = 1.0;
            b.denom[A1] = -2.0 * zx;
            b.denom[A2] = square(zx) + square(zy);
            if low {
                dc *= f64::from(calc2d_dist_sqr(1.0, 0.0, zx, zy));
            } else {
                dc *= f64::from(calc2d_dist_sqr(-1.0, 0.0, zx, zy));
            }
        }
    }

    let scale = dc / ((1i32 << order) as f64);
    p[0].numer[B0] *= scale;
    p[0].numer[B1] *= scale;
    p[0].numer[B2] *= scale;

    p
}

/// Evaluates the Chebyshev polynomial (`Biquad::ChebyPoly`).
pub fn calc_chebyshev_type1_filter(
    order: i32,
    f_n: f64,
    fc: f64,
    ripple: f64,
    subtype: i32,
) -> Vec<Biquad> {
    if order < MIN_ORDER || order as usize > MAX_ORDER {
        return Vec::new();
    }
    let sections = ((order + 1) / 2) as usize;
    let mut p = vec![Biquad::new(); sections];

    let mut f_norm = fc / f_n;
    if f_norm >= 0.9999 {
        f_norm = f64::from(0.9999f32);
    }
    let f_c = (PI * f_norm / 2.0).tan();
    let mut dc = 1.0f64;
    let beta = (f_norm * PI).cos();
    let low = subtype == K_LOW_PASS;

    let eps = (10.0f64.powf(ripple.max(0.001) / 10.0) - 1.0).sqrt();
    let a = (1.0 / eps + (1.0 / square(eps) + 1.0).sqrt()).ln() / order as f64;

    let mut zx;
    let mut zy;
    let mut zzero_x;

    for ipair in 0..(order / 2) as usize {
        let ang = (2 * ipair as i32 + 1) as f64 * PI / (2 * order) as f64;
        let spx = -f_c * a.sinh() * ang.sin();
        let spy = f_c * a.cosh() * ang.cos();
        let (mut x, mut y) = bilin_transform(spx, spy);
        if low {
            zzero_x = -1.0;
            dc = f64::from(calc2d_dist_sqr(1.0, 0.0, x, y));
            dc /= 2.0 * 2.0;
        } else {
            let (nx, ny) = complex_div(beta - x, -y, 1.0 - beta * x, -beta * y);
            x = nx;
            y = ny;
            zzero_x = 1.0;
            dc = f64::from(calc2d_dist_sqr(-1.0, 0.0, x, y));
            dc /= 2.0 * 2.0;
        }
        zx = x;
        zy = y;
        let b = &mut p[ipair];
        b.numer[B0] = dc;
        b.numer[B1] = -2.0 * zzero_x * dc;
        b.numer[B2] = dc;
        b.denom[A1] = -2.0 * zx;
        b.denom[A2] = square(zx) + square(zy);
    }

    if (order & 1) == 0 {
        let temp = db_to_linear(-ripple.max(0.001));
        p[0].numer[B0] *= temp;
        p[0].numer[B1] *= temp;
        p[0].numer[B2] *= temp;
    } else {
        let ipair = ((order - 1) / 2) as usize;
        let spx = -f_c * a.sinh();
        let (mut x, mut y) = bilin_transform(spx, 0.0);
        if low {
            zzero_x = -1.0;
            let d = calc2d_dist_sqr(1.0, 0.0, x, y);
            // C++ promotes the float return of Calc2D_DistSqr to double before
            // calling the double-arg sqrt, so the square root is done in double.
            dc = f64::from(d).sqrt();
            dc /= 2.0;
        } else {
            let (nx, ny) = complex_div(beta - x, -y, 1.0 - beta * x, -beta * y);
            x = nx;
            y = ny;
            zzero_x = 1.0;
            let d = calc2d_dist_sqr(-1.0, 0.0, x, y);
            dc = f64::from(d).sqrt();
            dc /= 2.0;
        }
        zx = x;
        zy = y;
        let b = &mut p[ipair];
        b.numer[B0] = dc;
        b.numer[B1] = -zzero_x * dc;
        b.numer[B2] = 0.0;
        b.denom[A1] = -zx;
        b.denom[A2] = 0.0;
    }

    p
}

/// Calculates the coefficients of a Chebyshev type II filter of the given
/// order, cutoff frequency `fc`, sample rate `f_n` and passband `ripple`,
/// returned as a cascade of biquads.
pub fn calc_chebyshev_type2_filter(
    order: i32,
    f_n: f64,
    fc: f64,
    ripple: f64,
    subtype: i32,
) -> Vec<Biquad> {
    if order < MIN_ORDER || order as usize > MAX_ORDER {
        return Vec::new();
    }
    let sections = ((order + 1) / 2) as usize;
    let mut p = vec![Biquad::new(); sections];

    let mut f_norm = fc / f_n;
    if f_norm >= 0.9999 {
        f_norm = f64::from(0.9999f32);
    }
    let f_c = (PI * f_norm / 2.0).tan();
    let mut dc = 1.0f64;
    let beta = (f_norm * PI).cos();
    let low = subtype == K_LOW_PASS;

    let eps = db_to_linear(-ripple.max(0.001));
    let a = (1.0 / eps + (1.0 / square(eps) + 1.0).sqrt()).ln() / order as f64;

    let mut zpx;
    let mut zpy;
    let mut zzx;
    let mut zzy;

    for ipair in 0..(order / 2) as usize {
        let ang = (2 * ipair as i32 + 1) as f64 * PI / (2 * order) as f64;
        let (spx, spy) = complex_div(f_c, 0.0, -a.sinh() * ang.sin(), a.cosh() * ang.cos());
        let (x, y) = bilin_transform(spx, spy);
        zpx = x;
        zpy = y;
        let s_zero_y = f_c / ang.cos();
        let (x, y) = bilin_transform(0.0, s_zero_y);
        zzx = x;
        zzy = y;

        if low {
            let num = f64::from(calc2d_dist_sqr(1.0, 0.0, zpx, zpy));
            let den = f64::from(calc2d_dist_sqr(1.0, 0.0, zzx, zzy));
            dc = num / den;
        } else {
            let (nx, ny) = complex_div(beta - zpx, -zpy, 1.0 - beta * zpx, -beta * zpy);
            zpx = nx;
            zpy = ny;
            let (nx, ny) = complex_div(beta - zzx, -zzy, 1.0 - beta * zzx, -beta * zzy);
            zzx = nx;
            zzy = ny;
            let num = f64::from(calc2d_dist_sqr(-1.0, 0.0, zpx, zpy));
            let den = f64::from(calc2d_dist_sqr(-1.0, 0.0, zzx, zzy));
            dc = num / den;
        }

        let b = &mut p[ipair];
        b.numer[B0] = dc;
        b.numer[B1] = -2.0 * zzx * dc;
        b.numer[B2] = (square(zzx) + square(zzy)) * dc;
        b.denom[A1] = -2.0 * zpx;
        b.denom[A2] = square(zpx) + square(zpy);
    }

    if (order & 1) != 0 {
        let ipair = ((order - 1) / 2) as usize;
        let ang = (2 * ipair as i32 + 1) as f64 * PI / (2 * order) as f64;
        let (spx, spy) = complex_div(f_c, 0.0, -a.sinh() * ang.sin(), a.cosh() * ang.cos());
        let (x, y) = bilin_transform(spx, spy);
        zpx = x;
        zpy = y;
        let mut zzero_x = -1.0;
        // Note: the original passes `-fZPoleY` as the imaginary part of the
        // denominator in the highpass branch; reproduced verbatim.
        // C++ widens the f32 result of Calc2D_DistSqr to double *before*
        // taking the square root; do the same here to match bit-for-bit.
        if low {
            let d = calc2d_dist_sqr(1.0, 0.0, zpx, zpy);
            dc = f64::from(d).sqrt();
            dc /= 2.0;
        } else {
            let (nx, ny) = complex_div(beta - zpx, -zpy, 1.0 - beta * zpx, -zpy);
            zpx = nx;
            zpy = ny;
            zzero_x = 1.0;
            let d = calc2d_dist_sqr(-1.0, 0.0, zpx, zpy);
            dc = f64::from(d).sqrt();
            dc /= 2.0;
        }
        let b = &mut p[ipair];
        b.numer[B0] = dc;
        b.numer[B1] = -zzero_x * dc;
        b.numer[B2] = 0.0;
        b.denom[A1] = -zpx;
        b.denom[A2] = 0.0;
    }

    p
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

/// Resets the filter state of `b`.
#[no_mangle]
pub unsafe extern "C" fn au3rs_biquad_reset(b: *mut Biquad) {
    if b.is_null() {
        return;
    }
    // SAFETY: caller guarantees a valid, exclusively usable pointer.
    unsafe { (*b).reset() }
}

/// Processes `n` samples from `input` into `output` (may alias).
#[no_mangle]
pub unsafe extern "C" fn au3rs_biquad_process(
    b: *mut Biquad,
    input: *const f32,
    output: *mut f32,
    n: i32,
) {
    if b.is_null() || input.is_null() || output.is_null() || n <= 0 {
        return;
    }
    for i in 0..n as usize {
        // SAFETY: caller guarantees both buffers hold `n` elements and that
        // `b` is uniquely borrowed for the duration of the call.
        let x = unsafe { *input.add(i) };
        let y = unsafe { (*b).process_one(x) };
        unsafe { *output.add(i) = y };
    }
}

/// Processes a single sample.
#[no_mangle]
pub unsafe extern "C" fn au3rs_biquad_process_one(b: *mut Biquad, x: f32) -> f32 {
    if b.is_null() {
        return 0.0;
    }
    unsafe { (*b).process_one(x) }
}

/// Designs a filter; writes up to 5 sections and returns their count.
#[no_mangle]
pub unsafe extern "C" fn au3rs_biquad_calc_filter(
    kind: i32,
    order: i32,
    f_n: f64,
    f_c: f64,
    ripple: f64,
    subtype: i32,
    out: *mut Biquad,
) -> i32 {
    if out.is_null() {
        return 0;
    }
    let sections = match kind {
        1 => calc_chebyshev_type1_filter(order, f_n, f_c, ripple, subtype),
        2 => calc_chebyshev_type2_filter(order, f_n, f_c, ripple, subtype),
        _ => calc_butterworth_filter(order, f_n, f_c, subtype),
    };
    for (i, b) in sections.iter().enumerate() {
        unsafe { ptr::write(out.add(i), *b) };
    }
    sections.len() as i32
}

/// Chebyshev polynomial value.
#[no_mangle]
pub extern "C" fn au3rs_biquad_cheby_poly(order: i32, norm_freq: f64) -> f64 {
    cheby_poly(order, norm_freq)
}

/// Chebyshev coefficient table accessor.
#[no_mangle]
pub extern "C" fn au3rs_biquad_cheby_coeff(i: i32, j: i32) -> f64 {
    cheby_coeff(i, j)
}

/// Complex division.
#[no_mangle]
pub unsafe extern "C" fn au3rs_biquad_complex_div(
    numer_r: f64,
    numer_i: f64,
    denom_r: f64,
    denom_i: f64,
    quotient_r: *mut f64,
    quotient_i: *mut f64,
) {
    let (r, i) = complex_div(numer_r, numer_i, denom_r, denom_i);
    if !quotient_r.is_null() {
        unsafe { *quotient_r = r };
    }
    if !quotient_i.is_null() {
        unsafe { *quotient_i = i };
    }
}

/// Bilinear transform.
#[no_mangle]
pub unsafe extern "C" fn au3rs_biquad_bilin_transform(
    s_x: f64,
    s_y: f64,
    z_x: *mut f64,
    z_y: *mut f64,
) -> bool {
    let (x, y) = bilin_transform(s_x, s_y);
    if !z_x.is_null() {
        unsafe { *z_x = x };
    }
    if !z_y.is_null() {
        unsafe { *z_y = y };
    }
    true
}

/// Squared 2-D distance (returns `float` as in C++).
#[no_mangle]
pub extern "C" fn au3rs_biquad_calc2d_distsqr(x1: f64, y1: f64, x2: f64, y2: f64) -> f32 {
    calc2d_dist_sqr(x1, y1, x2, y2)
}

// ---------------------------------------------------------------------------
// Tests against the reference data produced by tests/refgen/biquad_refgen.cpp
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const REF: &[u8] = include_bytes!("../../tests/data/biquad_ref.bin");

    struct Reader<'a> {
        data: &'a [u8],
        pos: usize,
    }

    impl<'a> Reader<'a> {
        fn i32(&mut self) -> i32 {
            let v = i32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
            self.pos += 4;
            v
        }

        fn f64(&mut self) -> f64 {
            let v = f64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
            self.pos += 8;
            v
        }

        fn f32(&mut self) -> f32 {
            let v = f32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
            self.pos += 4;
            v
        }
    }

    fn close_f64(a: f64, b: f64) -> bool {
        if a == b {
            return true;
        }
        let scale = a.abs().max(b.abs());
        (a - b).abs() <= 1e-12 * scale
    }

    fn close_f32(a: f32, b: f32) -> bool {
        if a == b {
            return true;
        }
        let scale = a.abs().max(b.abs());
        (a - b).abs() <= 1e-6 * scale
    }

    fn design(kind: i32, order: i32, f_n: f64, f_c: f64, ripple: f64, subtype: i32) -> Vec<Biquad> {
        match kind {
            1 => calc_chebyshev_type1_filter(order, f_n, f_c, ripple, subtype),
            2 => calc_chebyshev_type2_filter(order, f_n, f_c, ripple, subtype),
            _ => calc_butterworth_filter(order, f_n, f_c, subtype),
        }
    }

    fn test_input() -> Vec<f32> {
        (0..256)
            .map(|n| {
                let n = n as f64;
                ((0.05 * n).sin() + 0.3 * (1.7 * n).sin()) as f32
            })
            .collect()
    }

    #[test]
    fn matches_reference_binary() {
        let mut r = Reader { data: REF, pos: 0 };
        let ncases = r.i32();
        assert_eq!(ncases, 3 * 6 * 2);
        for _ in 0..ncases {
            let kind = r.i32();
            let order = r.i32();
            let subtype = r.i32();
            let nsections = r.i32() as usize;

            let mut expected_coeffs = Vec::with_capacity(nsections);
            for _ in 0..nsections {
                let mut c = [0.0f64; 5];
                for v in c.iter_mut() {
                    *v = r.f64();
                }
                expected_coeffs.push(c);
            }
            let mut expected_out = vec![0.0f32; 256];
            for v in expected_out.iter_mut() {
                *v = r.f32();
            }

            let ripple = match kind {
                1 => 1.0,
                2 => 40.0,
                _ => 0.0,
            };
            let mut sections = design(kind, order, 44100.0, 1000.0, ripple, subtype);
            assert_eq!(sections.len(), nsections, "kind={kind} order={order}");

            for (b, c) in sections.iter().zip(expected_coeffs.iter()) {
                for k in 0..3 {
                    assert!(
                        close_f64(b.numer[k], c[k]),
                        "numer[{k}] {} != {} (kind={kind} order={order} sub={subtype})",
                        b.numer[k],
                        c[k]
                    );
                }
                for k in 0..2 {
                    assert!(
                        close_f64(b.denom[k], c[3 + k]),
                        "denom[{k}] {} != {} (kind={kind} order={order} sub={subtype})",
                        b.denom[k],
                        c[3 + k]
                    );
                }
            }

            let mut buf = test_input();
            for b in sections.iter_mut() {
                for i in 0..256 {
                    buf[i] = b.process_one(buf[i]);
                }
            }
            for i in 0..256 {
                assert!(
                    close_f32(buf[i], expected_out[i]),
                    "out[{i}] {} != {} (kind={kind} order={order} sub={subtype})",
                    buf[i],
                    expected_out[i]
                );
            }
        }

        let ncheby = r.i32();
        assert_eq!(ncheby, 40);
        for _ in 0..ncheby {
            let order = r.i32();
            let x = r.f64();
            let expected = r.f64();
            let got = cheby_poly(order, x);
            assert!(
                close_f64(got, expected),
                "cheby_poly({order}, {x}) {} != {}",
                got,
                expected
            );
        }
    }
}

//! The x86_64-msvc configuration: the asm shim against libvpx's own C, and the
//! decoder at every SIMD level against the pure-C hash.

use super::*;

type Helper = unsafe extern "C" fn(*const u8, isize, *mut u8, isize, u32, *const i16);
type Convolve = unsafe extern "C" fn(*const u8, isize, *mut u8, isize, *const [i16; 8], c_int, c_int, c_int, c_int, c_int, c_int);

// libvpx's symbols are `vnvpx_`-prefixed (VENDOR.md, "Symbol prefix").
extern "C" {
    #[link_name = "vnvpx_vp9_filter_kernels"]
    static vp9_filter_kernels: [*const [i16; 8]; 5];
}

macro_rules! convolves {
    ($($name:ident),* $(,)?) => {
        extern "C" {
            $(
                #[link_name = concat!("vnvpx_", stringify!($name))]
                fn $name(s: *const u8, ss: isize, d: *mut u8, ds: isize, f: *const [i16; 8], x0: c_int, xs: c_int, y0: c_int, ys: c_int, w: c_int, h: c_int);
            )*
        }
    };
}
convolves!(
    vpx_convolve8_horiz_c, vpx_convolve8_vert_c, vpx_convolve8_avg_horiz_c, vpx_convolve8_avg_vert_c,
    vpx_convolve8_c, vpx_convolve8_avg_c,
);

macro_rules! helpers {
    ($($name:ident),* $(,)?) => {
        extern "C" {
            $(
                #[link_name = concat!("vnvpx_", stringify!($name))]
                fn $name(s: *const u8, ss: isize, d: *mut u8, ds: isize, h: u32, f: *const i16);
            )*
        }
    };
}
helpers!(
    vpx_filter_block1d16_h8_ssse3, vpx_filter_block1d16_v8_ssse3, vpx_filter_block1d4_v8_ssse3,
    vpx_filter_block1d16_h8_avg_ssse3, vpx_filter_block1d16_v8_avg_ssse3,
    vpx_filter_block1d8_h8_avg_ssse3, vpx_filter_block1d8_v8_avg_ssse3,
    vpx_filter_block1d4_h8_avg_ssse3, vpx_filter_block1d4_v8_avg_ssse3,
    vpx_filter_block1d16_h2_ssse3, vpx_filter_block1d16_v2_ssse3, vpx_filter_block1d8_h2_ssse3,
    vpx_filter_block1d8_v2_ssse3, vpx_filter_block1d4_h2_ssse3, vpx_filter_block1d4_v2_ssse3,
    vpx_filter_block1d16_h2_avg_ssse3, vpx_filter_block1d16_v2_avg_ssse3, vpx_filter_block1d8_h2_avg_ssse3,
    vpx_filter_block1d8_v2_avg_ssse3, vpx_filter_block1d4_h2_avg_ssse3, vpx_filter_block1d4_v2_avg_ssse3,
    vpx_filter_block1d16_h2_sse2, vpx_filter_block1d16_v2_sse2, vpx_filter_block1d8_h2_sse2,
    vpx_filter_block1d8_v2_sse2, vpx_filter_block1d4_h2_sse2, vpx_filter_block1d4_v2_sse2,
    vpx_filter_block1d16_h2_avg_sse2, vpx_filter_block1d16_v2_avg_sse2, vpx_filter_block1d8_h2_avg_sse2,
    vpx_filter_block1d8_v2_avg_sse2, vpx_filter_block1d4_h2_avg_sse2, vpx_filter_block1d4_v2_avg_sse2,
    vpx_filter_block1d16_h8_sse2, vpx_filter_block1d16_v8_sse2, vpx_filter_block1d8_h8_sse2,
    vpx_filter_block1d8_v8_sse2, vpx_filter_block1d4_h8_sse2, vpx_filter_block1d4_v8_sse2,
    vpx_filter_block1d16_h8_avg_sse2, vpx_filter_block1d16_v8_avg_sse2, vpx_filter_block1d8_h8_avg_sse2,
    vpx_filter_block1d8_v8_avg_sse2, vpx_filter_block1d4_h8_avg_sse2, vpx_filter_block1d4_v8_avg_sse2,
);

convolves!(
    vpx_convolve8_horiz_sse2, vpx_convolve8_vert_sse2, vpx_convolve8_avg_horiz_sse2, vpx_convolve8_avg_vert_sse2,
    vpx_convolve8_sse2, vpx_convolve8_avg_sse2,
    vpx_convolve8_horiz_ssse3, vpx_convolve8_vert_ssse3, vpx_convolve8_avg_horiz_ssse3, vpx_convolve8_avg_vert_ssse3,
    vpx_convolve8_ssse3, vpx_convolve8_avg_ssse3,
    vpx_convolve8_horiz_avx2, vpx_convolve8_vert_avx2, vpx_convolve8_avg_horiz_avx2, vpx_convolve8_avg_vert_avx2,
    vpx_convolve8_avx2, vpx_convolve8_avg_avx2,
);

/// A 1-D pass's direction and whether it averages.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pass {
    H,
    V,
    HAvg,
    VAvg,
}

/// Source and destination planes with a margin the filters may read into.
struct Planes {
    src: Vec<u8>,
    dst: Vec<u8>,
}

const STRIDE: usize = 96;
const ROWS: usize = 160;
/// Where a block starts: 8 rows and 8 columns into the planes.
const ORIGIN: usize = 8 * STRIDE + 8;

impl Planes {
    fn new(noise: &mut Noise) -> Planes {
        let mut src = noise.bytes(STRIDE * ROWS);
        // Runs of extremes, so clipping happens.
        for (i, b) in src.iter_mut().enumerate() {
            match (i / 7) % 11 {
                0 => *b = 0,
                1 => *b = 255,
                _ => {}
            }
        }
        Planes { src, dst: noise.bytes(STRIDE * ROWS) }
    }
}

fn kernel(table: usize, phase: usize) -> *const i16 {
    unsafe { vp9_filter_kernels[table].add(phase) as *const i16 }
}

/// libvpx's C for one helper's job: `w` × `h` at ORIGIN.
fn reference(planes: &Planes, dst: &mut [u8], pass: Pass, table: usize, phase: usize, w: usize, h: usize) {
    let kernels = unsafe { vp9_filter_kernels[table] };
    let (src, d) = (planes.src[ORIGIN..].as_ptr(), dst[ORIGIN..].as_mut_ptr());
    let (s, w, h, p) = (STRIDE as isize, w as c_int, h as c_int, phase as c_int);
    unsafe {
        match pass {
            Pass::H => vpx_convolve8_horiz_c(src, s, d, s, kernels, p, 16, 0, 16, w, h),
            Pass::V => vpx_convolve8_vert_c(src, s, d, s, kernels, 0, 16, p, 16, w, h),
            Pass::HAvg => vpx_convolve8_avg_horiz_c(src, s, d, s, kernels, p, 16, 0, 16, w, h),
            Pass::VAvg => vpx_convolve8_avg_vert_c(src, s, d, s, kernels, 0, 16, p, 16, w, h),
        }
    }
}

/// One shim helper against the C for every phase of every kernel table it
/// can be handed (`tables`), at several heights, on fresh noise each time.
fn check_helper(name: &str, helper: Helper, w: usize, taps: usize, pass: Pass, tables: &[usize]) {
    let mut noise = Noise(0x1234_5678 ^ (w as u64) << 8 ^ taps as u64);
    for &table in tables {
        for phase in 1..16 {
            let planes = Planes::new(&mut noise);
            for h in [1usize, 2, 4, 8, 15, 16, 33, 64, 71, 100] {
                let mut expected = planes.dst.clone();
                reference(&planes, &mut expected, pass, table, phase, w, h);
                let mut actual = planes.dst.clone();
                // A vertical helper gets src already moved up taps/2 - 1 rows.
                let up = if matches!(pass, Pass::V | Pass::VAvg) { (taps / 2 - 1) * STRIDE } else { 0 };
                unsafe {
                    helper(planes.src[ORIGIN - up..].as_ptr(), STRIDE as isize, actual[ORIGIN..].as_mut_ptr(), STRIDE as isize, h as u32, kernel(table, phase));
                }
                assert!(actual == expected, "{name}: table {table}, phase {phase}, h {h} differs from vpx_convolve8_*_c");
            }
        }
    }
}

/// Every kernel table with outer taps (regular, smooth, sharp, and the 4-tap
/// one the averaging 8-tap helpers also take), and the bilinear one.
const EIGHT_TAP_TABLES: &[usize] = &[0, 1, 2, 4];
const BILINEAR: &[usize] = &[3];

#[test]
fn every_shim_helper_matches_libvpx_c() {
    use Pass::*;
    let eight: &[(&str, Helper, usize, Pass)] = &[
        ("16_h8_ssse3", vpx_filter_block1d16_h8_ssse3, 16, H),
        ("16_v8_ssse3", vpx_filter_block1d16_v8_ssse3, 16, V),
        ("4_v8_ssse3", vpx_filter_block1d4_v8_ssse3, 4, V),
        ("16_h8_avg_ssse3", vpx_filter_block1d16_h8_avg_ssse3, 16, HAvg),
        ("16_v8_avg_ssse3", vpx_filter_block1d16_v8_avg_ssse3, 16, VAvg),
        ("8_h8_avg_ssse3", vpx_filter_block1d8_h8_avg_ssse3, 8, HAvg),
        ("8_v8_avg_ssse3", vpx_filter_block1d8_v8_avg_ssse3, 8, VAvg),
        ("4_h8_avg_ssse3", vpx_filter_block1d4_h8_avg_ssse3, 4, HAvg),
        ("4_v8_avg_ssse3", vpx_filter_block1d4_v8_avg_ssse3, 4, VAvg),
        ("16_h8_sse2", vpx_filter_block1d16_h8_sse2, 16, H),
        ("16_v8_sse2", vpx_filter_block1d16_v8_sse2, 16, V),
        ("8_h8_sse2", vpx_filter_block1d8_h8_sse2, 8, H),
        ("8_v8_sse2", vpx_filter_block1d8_v8_sse2, 8, V),
        ("4_h8_sse2", vpx_filter_block1d4_h8_sse2, 4, H),
        ("4_v8_sse2", vpx_filter_block1d4_v8_sse2, 4, V),
        ("16_h8_avg_sse2", vpx_filter_block1d16_h8_avg_sse2, 16, HAvg),
        ("16_v8_avg_sse2", vpx_filter_block1d16_v8_avg_sse2, 16, VAvg),
        ("8_h8_avg_sse2", vpx_filter_block1d8_h8_avg_sse2, 8, HAvg),
        ("8_v8_avg_sse2", vpx_filter_block1d8_v8_avg_sse2, 8, VAvg),
        ("4_h8_avg_sse2", vpx_filter_block1d4_h8_avg_sse2, 4, HAvg),
        ("4_v8_avg_sse2", vpx_filter_block1d4_v8_avg_sse2, 4, VAvg),
    ];
    for &(name, helper, w, pass) in eight {
        check_helper(name, helper, w, 8, pass, EIGHT_TAP_TABLES);
    }
    let two: &[(&str, Helper, usize, Pass)] = &[
        ("16_h2_ssse3", vpx_filter_block1d16_h2_ssse3, 16, H),
        ("16_v2_ssse3", vpx_filter_block1d16_v2_ssse3, 16, V),
        ("8_h2_ssse3", vpx_filter_block1d8_h2_ssse3, 8, H),
        ("8_v2_ssse3", vpx_filter_block1d8_v2_ssse3, 8, V),
        ("4_h2_ssse3", vpx_filter_block1d4_h2_ssse3, 4, H),
        ("4_v2_ssse3", vpx_filter_block1d4_v2_ssse3, 4, V),
        ("16_h2_avg_ssse3", vpx_filter_block1d16_h2_avg_ssse3, 16, HAvg),
        ("16_v2_avg_ssse3", vpx_filter_block1d16_v2_avg_ssse3, 16, VAvg),
        ("8_h2_avg_ssse3", vpx_filter_block1d8_h2_avg_ssse3, 8, HAvg),
        ("8_v2_avg_ssse3", vpx_filter_block1d8_v2_avg_ssse3, 8, VAvg),
        ("4_h2_avg_ssse3", vpx_filter_block1d4_h2_avg_ssse3, 4, HAvg),
        ("4_v2_avg_ssse3", vpx_filter_block1d4_v2_avg_ssse3, 4, VAvg),
        ("16_h2_sse2", vpx_filter_block1d16_h2_sse2, 16, H),
        ("16_v2_sse2", vpx_filter_block1d16_v2_sse2, 16, V),
        ("8_h2_sse2", vpx_filter_block1d8_h2_sse2, 8, H),
        ("8_v2_sse2", vpx_filter_block1d8_v2_sse2, 8, V),
        ("4_h2_sse2", vpx_filter_block1d4_h2_sse2, 4, H),
        ("4_v2_sse2", vpx_filter_block1d4_v2_sse2, 4, V),
        ("16_h2_avg_sse2", vpx_filter_block1d16_h2_avg_sse2, 16, HAvg),
        ("16_v2_avg_sse2", vpx_filter_block1d16_v2_avg_sse2, 16, VAvg),
        ("8_h2_avg_sse2", vpx_filter_block1d8_h2_avg_sse2, 8, HAvg),
        ("8_v2_avg_sse2", vpx_filter_block1d8_v2_avg_sse2, 8, VAvg),
        ("4_h2_avg_sse2", vpx_filter_block1d4_h2_avg_sse2, 4, HAvg),
        ("4_v2_avg_sse2", vpx_filter_block1d4_v2_avg_sse2, 4, VAvg),
    ];
    for &(name, helper, w, pass) in two {
        check_helper(name, helper, w, 2, pass, BILINEAR);
    }
}

fn has_avx2() -> bool {
    std::arch::is_x86_feature_detected!("avx2")
}

/// The convolutions the decoder dispatches to (each a FUN_CONV_1D/2D over the
/// helpers, shim ones included) against their C, every block size VP9 has.
#[test]
fn every_simd_convolution_matches_libvpx_c() {
    let mut sets: Vec<(&str, [Convolve; 6])> = vec![
        ("sse2", [vpx_convolve8_horiz_sse2, vpx_convolve8_vert_sse2, vpx_convolve8_avg_horiz_sse2, vpx_convolve8_avg_vert_sse2, vpx_convolve8_sse2, vpx_convolve8_avg_sse2]),
        ("ssse3", [vpx_convolve8_horiz_ssse3, vpx_convolve8_vert_ssse3, vpx_convolve8_avg_horiz_ssse3, vpx_convolve8_avg_vert_ssse3, vpx_convolve8_ssse3, vpx_convolve8_avg_ssse3]),
    ];
    if has_avx2() {
        sets.push(("avx2", [vpx_convolve8_horiz_avx2, vpx_convolve8_vert_avx2, vpx_convolve8_avg_horiz_avx2, vpx_convolve8_avg_vert_avx2, vpx_convolve8_avx2, vpx_convolve8_avg_avx2]));
    }
    let c: [Convolve; 6] = [vpx_convolve8_horiz_c, vpx_convolve8_vert_c, vpx_convolve8_avg_horiz_c, vpx_convolve8_avg_vert_c, vpx_convolve8_c, vpx_convolve8_avg_c];
    let names = ["horiz", "vert", "avg_horiz", "avg_vert", "2d", "avg_2d"];
    let mut noise = Noise(0xfeed_beef);
    for (isa, set) in &sets {
        for (k, (simd, c)) in set.iter().zip(c).enumerate() {
            for (table, &kernels) in unsafe { vp9_filter_kernels }.iter().enumerate() {
                for w in [4, 8, 16, 32, 64] {
                    for h in [4, 8, 16, 32, 64] {
                        // A 1-D pass gets a sub-pixel phase in its own direction
                        // (phase 0 is a copy, never sent to these); 2-D both.
                        let (px, py) = (1 + (noise.next() % 15) as c_int, 1 + (noise.next() % 15) as c_int);
                        let (x0, y0) = match k {
                            0 | 2 => (px, 0),
                            1 | 3 => (0, py),
                            _ => (px, py),
                        };
                        let planes = Planes::new(&mut noise);
                        let (mut expected, mut actual) = (planes.dst.clone(), planes.dst.clone());
                        let src = planes.src[ORIGIN + 16 * STRIDE..].as_ptr();
                        let s = STRIDE as isize;
                        unsafe {
                            c(src, s, expected[ORIGIN..].as_mut_ptr(), s, kernels, x0, 16, y0, 16, w, h);
                            simd(src, s, actual[ORIGIN..].as_mut_ptr(), s, kernels, x0, 16, y0, 16, w, h);
                        }
                        assert!(actual == expected, "vpx_convolve8_{}_{isa}: table {table}, {w}x{h}, phase {x0}/{y0}", names[k]);
                    }
                }
            }
        }
    }
}

/// libvpx's dispatched function pointers (vpx_dsp_rtcd.h's RTCD_EXTERN
/// globals; vp9_rtcd.h and vpx_scale_rtcd.h dispatch nothing on x86_64), and
/// what libvpx's set-up would store in each on a CPU with SSE2, SSSE3 or
/// AVX2. The intra predictors are left out: every SIMD one is asm-only, so
/// they are C at every level here.
macro_rules! dispatch_table {
    (variants: $($variant:ident),* ; table: $($slot:ident: $c:ident, $sse2:ident, $ssse3:ident, $avx2:ident;)*) => {
        extern "C" {
            $(
                #[link_name = concat!("vnvpx_", stringify!($slot))]
                static mut $slot: *const c_void;
            )*
        }
        #[allow(clashing_extern_declarations)] // only their addresses are taken
        mod variants {
            extern "C" {
                $(
                    #[link_name = concat!("vnvpx_", stringify!($variant))]
                    pub fn $variant();
                )*
            }
        }
        /// Points every dispatched function at its `level` variant.
        unsafe fn dispatch(level: &str) {
            $(
                let f: unsafe extern "C" fn() = match level {
                    "c" => variants::$c,
                    "sse2" => variants::$sse2,
                    "ssse3" => variants::$ssse3,
                    "avx2" => variants::$avx2,
                    other => panic!("unknown level {other}"),
                };
                *std::ptr::addr_of_mut!($slot) = f as *const c_void;
            )*
        }
    };
}

dispatch_table! {
    variants:
    vpx_convolve8_c, vpx_convolve8_sse2, vpx_convolve8_ssse3, vpx_convolve8_avx2,
    vpx_convolve8_avg_c, vpx_convolve8_avg_sse2, vpx_convolve8_avg_ssse3, vpx_convolve8_avg_avx2,
    vpx_convolve8_avg_horiz_c, vpx_convolve8_avg_horiz_sse2, vpx_convolve8_avg_horiz_ssse3, vpx_convolve8_avg_horiz_avx2,
    vpx_convolve8_avg_vert_c, vpx_convolve8_avg_vert_sse2, vpx_convolve8_avg_vert_ssse3, vpx_convolve8_avg_vert_avx2,
    vpx_convolve8_horiz_c, vpx_convolve8_horiz_sse2, vpx_convolve8_horiz_ssse3, vpx_convolve8_horiz_avx2,
    vpx_convolve8_vert_c, vpx_convolve8_vert_sse2, vpx_convolve8_vert_ssse3, vpx_convolve8_vert_avx2,
    vpx_idct16x16_256_add_c, vpx_idct16x16_256_add_sse2, vpx_idct16x16_256_add_avx2,
    vpx_idct32x32_1024_add_c, vpx_idct32x32_1024_add_sse2, vpx_idct32x32_1024_add_avx2,
    vpx_idct32x32_135_add_c, vpx_idct32x32_135_add_sse2, vpx_idct32x32_135_add_ssse3, vpx_idct32x32_135_add_avx2,
    vpx_idct32x32_34_add_c, vpx_idct32x32_34_add_sse2, vpx_idct32x32_34_add_ssse3,
    vpx_idct8x8_12_add_c, vpx_idct8x8_12_add_sse2, vpx_idct8x8_12_add_ssse3,
    vpx_lpf_horizontal_16_c, vpx_lpf_horizontal_16_sse2, vpx_lpf_horizontal_16_avx2,
    vpx_lpf_horizontal_16_dual_c, vpx_lpf_horizontal_16_dual_sse2, vpx_lpf_horizontal_16_dual_avx2,
    vpx_scaled_2d_c, vpx_scaled_2d_ssse3;
    table:
    vpx_convolve8: vpx_convolve8_c, vpx_convolve8_sse2, vpx_convolve8_ssse3, vpx_convolve8_avx2;
    vpx_convolve8_avg: vpx_convolve8_avg_c, vpx_convolve8_avg_sse2, vpx_convolve8_avg_ssse3, vpx_convolve8_avg_avx2;
    vpx_convolve8_avg_horiz: vpx_convolve8_avg_horiz_c, vpx_convolve8_avg_horiz_sse2, vpx_convolve8_avg_horiz_ssse3, vpx_convolve8_avg_horiz_avx2;
    vpx_convolve8_avg_vert: vpx_convolve8_avg_vert_c, vpx_convolve8_avg_vert_sse2, vpx_convolve8_avg_vert_ssse3, vpx_convolve8_avg_vert_avx2;
    vpx_convolve8_horiz: vpx_convolve8_horiz_c, vpx_convolve8_horiz_sse2, vpx_convolve8_horiz_ssse3, vpx_convolve8_horiz_avx2;
    vpx_convolve8_vert: vpx_convolve8_vert_c, vpx_convolve8_vert_sse2, vpx_convolve8_vert_ssse3, vpx_convolve8_vert_avx2;
    vpx_idct16x16_256_add: vpx_idct16x16_256_add_c, vpx_idct16x16_256_add_sse2, vpx_idct16x16_256_add_sse2, vpx_idct16x16_256_add_avx2;
    vpx_idct32x32_1024_add: vpx_idct32x32_1024_add_c, vpx_idct32x32_1024_add_sse2, vpx_idct32x32_1024_add_sse2, vpx_idct32x32_1024_add_avx2;
    vpx_idct32x32_135_add: vpx_idct32x32_135_add_c, vpx_idct32x32_135_add_sse2, vpx_idct32x32_135_add_ssse3, vpx_idct32x32_135_add_avx2;
    vpx_idct32x32_34_add: vpx_idct32x32_34_add_c, vpx_idct32x32_34_add_sse2, vpx_idct32x32_34_add_ssse3, vpx_idct32x32_34_add_ssse3;
    vpx_idct8x8_12_add: vpx_idct8x8_12_add_c, vpx_idct8x8_12_add_sse2, vpx_idct8x8_12_add_ssse3, vpx_idct8x8_12_add_ssse3;
    vpx_lpf_horizontal_16: vpx_lpf_horizontal_16_c, vpx_lpf_horizontal_16_sse2, vpx_lpf_horizontal_16_sse2, vpx_lpf_horizontal_16_avx2;
    vpx_lpf_horizontal_16_dual: vpx_lpf_horizontal_16_dual_c, vpx_lpf_horizontal_16_dual_sse2, vpx_lpf_horizontal_16_dual_sse2, vpx_lpf_horizontal_16_dual_avx2;
    vpx_scaled_2d: vpx_scaled_2d_c, vpx_scaled_2d_c, vpx_scaled_2d_ssse3, vpx_scaled_2d_ssse3;
}

/// The child half of [`every_simd_level_decodes_bit_exact`]: sets libvpx's
/// dispatch to `VN_VPX_LEVEL`, decodes the clip, prints the hash.
#[test]
#[ignore = "run by every_simd_level_decodes_bit_exact in a child process"]
fn simd_level_child() {
    let level = std::env::var("VN_VPX_LEVEL").expect("VN_VPX_LEVEL");
    if level == "avx2" && !has_avx2() {
        println!("HASH skipped: no AVX2 on this CPU");
        return;
    }
    init_once();
    // SAFETY: this process runs no other test; no decoder exists yet.
    unsafe { dispatch(&level) };
    let (shown, hash) = decode_all(&ivf(COMPOUND));
    #[cfg(feature = "shim-stats")]
    {
        let mut stats = [0i64; 4];
        unsafe { ffi::vn_vpx_shim_stats(stats.as_mut_ptr()) };
        println!("SHIM {} {} {} {}", stats[0], stats[1], stats[2], stats[3]);
    }
    println!("HASH {shown} {hash:#018x}");
}

/// The clip decoded with libvpx's dispatch set as a CPU with only SSE2, with
/// SSSE3, and with AVX2 would set it (and with the rtcd C fallbacks), each in
/// a child process because the dispatch is process-wide. libvpx's own
/// `VPX_SIMD_CAPS` override cannot do this: on a current Windows SDK its
/// `getenv` is compiled out (vpx_ports/x86.h tests
/// `WINAPI_FAMILY_PARTITION(WINAPI_FAMILY_APP)`, which is always true now).
#[test]
fn every_simd_level_decodes_bit_exact() {
    let exe = std::env::current_exe().unwrap();
    let mut hashes = Vec::new();
    for level in ["c", "sse2", "ssse3", "avx2"] {
        let mut cmd = std::process::Command::new(&exe);
        cmd.args(["--ignored", "--exact", "tests::x86::simd_level_child", "--nocapture", "--test-threads=1"]);
        cmd.env("VN_VPX_LEVEL", level);
        let out = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{level}: {stdout}\n{}", String::from_utf8_lossy(&out.stderr));
        // The harness prints "test <name> ... " before the test's own output.
        let tagged = |tag: &str| stdout.lines().find_map(|l| l.find(tag).map(|at| l[at..].to_string()));
        let line = tagged("HASH ").unwrap_or_else(|| panic!("{level}: no hash in {stdout}"));
        if let Some(shim) = tagged("SHIM ") {
            println!("{level}: {shim} (calls: ssse3 8-tap, ssse3 8-tap avg, bilinear, sse2 8-tap)");
        }
        println!("{level}: {line}");
        if !line.starts_with("HASH skipped") {
            hashes.push((level, line));
        }
    }
    let expected = format!("HASH {COMPOUND_FRAMES} {COMPOUND_HASH:#018x}");
    for (level, line) in &hashes {
        assert_eq!(line, &expected, "SIMD level {level}");
    }
}

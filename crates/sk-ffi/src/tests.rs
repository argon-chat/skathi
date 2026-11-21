//! Every call here is made twice: by the compiler, as an ordinary call, and through [`call`].

use super::*;

trait Arg: Copy {
    const TYPE: Type;
    fn sample(position: usize) -> Self;
    fn slot(self) -> u64;
}

macro_rules! int_arg {
    ($($ty:ty => $variant:ident, $seed:expr;)*) => {$(
        impl Arg for $ty {
            const TYPE: Type = Type::$variant;
            fn sample(position: usize) -> Self {
                ($seed as $ty).wrapping_mul(position as $ty + 3).wrapping_sub(position as $ty)
            }
            fn slot(self) -> u64 {
                self as i64 as u64
            }
        }
    )*};
}

int_arg! {
    i8 => I8, -0x5b;
    u8 => U8, 0xc7;
    i16 => I16, -0x6b3d;
    u16 => U16, 0xd1a7;
    i32 => I32, -0x5a17_c3d9;
    u32 => U32, 0xe3b1_95c7u32;
    i64 => I64, -0x5f3a_91c7_d2e4_b685i64;
    u64 => U64, 0xf1e2_d3c4_b5a6_9788u64;
    isize => Isize, -0x5f3a_91c7isize;
    usize => Usize, 0xf1e2_d3c4usize;
}

impl Arg for f32 {
    const TYPE: Type = Type::F32;
    fn sample(position: usize) -> Self {
        1.25 + position as f32 * 3.5
    }
    fn slot(self) -> u64 {
        self.to_bits() as u64
    }
}

impl Arg for f64 {
    const TYPE: Type = Type::F64;
    fn sample(position: usize) -> Self {
        -7.125 + position as f64 * 1e10
    }
    fn slot(self) -> u64 {
        self.to_bits()
    }
}

impl Arg for bool {
    const TYPE: Type = Type::Bool;
    fn sample(position: usize) -> Self {
        position.is_multiple_of(2)
    }
    fn slot(self) -> u64 {
        self as u64
    }
}

impl Arg for *const u8 {
    const TYPE: Type = Type::Pointer;
    fn sample(position: usize) -> Self {
        (0x7ff0_1234_5678_9000usize + position * 8) as *const u8
    }
    fn slot(self) -> u64 {
        self as usize as u64
    }
}

/// What the callee saw of one argument, so that a neighbour's bits read in its place change the result.
trait Seen {
    fn seen(self) -> u64;
}

macro_rules! seen {
    ($($ty:ty),*) => {$(impl Seen for $ty { fn seen(self) -> u64 { self.slot() } })*};
}
seen!(
    i8,
    u8,
    i16,
    u16,
    i32,
    u32,
    i64,
    u64,
    isize,
    usize,
    f32,
    f64,
    bool,
    *const u8
);

/// A C function of the given parameters that mixes them by position, and a test that calls it both ways.
macro_rules! mixes {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {$(
        #[test]
        #[allow(unused_mut, unused_assignments)]
        fn $name() {
            #[allow(unused_mut)]
            extern "C" fn callee($($arg: $ty),*) -> u64 {
                let mut mix = 17u64;
                $(mix = mix.wrapping_mul(1_000_003).wrapping_add($arg.seen());)*
                mix
            }
            let mut position = 0;
            $(
                let $arg = <$ty as Arg>::sample(position);
                position += 1;
            )*
            let _ = position;
            let signature = Signature {
                args: vec![$(<$ty as Arg>::TYPE),*],
                returns: Type::U64,
                fixed: None,
            };
            let slots = [$($arg.slot()),*];
            let through = unsafe { call(callee as *const (), &signature, &slots) };
            assert_eq!(through.value, callee($($arg),*));
        }
    )*};
}

mixes! {
    no_arguments();
    one_of_each_integer(a: i8, b: u8, c: i16, d: u16, e: i32, f: u32, g: i64, h: u64);
    floats_alone(a: f32, b: f64, c: f32, d: f64);
    integers_and_floats_in_turn(a: i32, b: f64, c: u64, d: f32, e: i8, f: f64, g: u16, h: f32);
    floats_past_the_fourth_position(a: i64, b: i64, c: i64, d: i64, e: f64, f: f32, g: f64);
    ten_integers_of_mixed_width(a: i32, b: u64, c: i32, d: u64, e: i32, f: u64, g: i32, h: u64, i: i32, j: u64);
    small_integers_on_the_stack(
        a: u64, b: u64, c: u64, d: u64, e: u64, f: u64, g: u64, h: u64,
        i: i8, j: i16, k: i8, l: i32, m: u8, n: u64, o: bool, p: u16
    );
    twelve_doubles(
        a: f64, b: f64, c: f64, d: f64, e: f64, f: f64,
        g: f64, h: f64, i: f64, j: f64, k: f64, l: f64
    );
    floats_of_both_widths_on_the_stack(
        a: f64, b: f64, c: f64, d: f64, e: f64, f: f64, g: f64, h: f64,
        i: f32, j: f64, k: f32, l: f32, m: f64
    );
    both_kinds_past_their_registers(
        a: i64, b: f64, c: i64, d: f64, e: i64, f: f64, g: i64, h: f64, i: i64, j: f64,
        k: i64, l: f64, m: i64, n: f64, o: i64, p: f64, q: i64, r: f64, s: i32, t: f32,
        u: i8, v: f32, w: u16, x: f64
    );
    pointers_and_sizes(a: *const u8, b: usize, c: isize, d: *const u8, e: bool, f: *const u8);
}

fn returning(function: *const (), returns: Type) -> u64 {
    let signature = Signature {
        args: vec![],
        returns,
        fixed: None,
    };
    unsafe { call(function, &signature, &[]) }.value
}

#[test]
fn a_narrow_return_is_extended_from_its_own_width() {
    extern "C" fn dirty() -> u64 {
        0xdead_beef_cafe_81ff
    }
    let f = dirty as *const ();
    assert_eq!(returning(f, Type::U8), 0xff);
    assert_eq!(returning(f, Type::I8) as i64, -1);
    assert_eq!(returning(f, Type::U16), 0x81ff);
    assert_eq!(returning(f, Type::I16) as i64, 0x81ffu16 as i16 as i64);
    assert_eq!(returning(f, Type::U32), 0xcafe_81ff);
    assert_eq!(returning(f, Type::I32) as i64, 0xcafe_81ffu32 as i32 as i64);
    assert_eq!(returning(f, Type::U64), 0xdead_beef_cafe_81ff);
    assert_eq!(returning(f, Type::Bool), 1);

    extern "C" fn false_with_noise() -> u64 {
        0xffff_ff00
    }
    assert_eq!(returning(false_with_noise as *const (), Type::Bool), 0);
}

#[test]
fn unsigned_32_bit_returns_stay_unsigned() {
    extern "C" fn below() -> u32 {
        0x7fff_ffff
    }
    extern "C" fn at() -> u32 {
        0x8000_0000
    }
    extern "C" fn top() -> u32 {
        0xffff_ffff
    }
    assert_eq!(returning(below as *const (), Type::U32), 0x7fff_ffff);
    assert_eq!(returning(at as *const (), Type::U32), 0x8000_0000);
    assert_eq!(returning(top as *const (), Type::U32), 0xffff_ffff);
}

#[test]
fn floats_and_pointers_come_back() {
    extern "C" fn single() -> f32 {
        -2.5
    }
    extern "C" fn double() -> f64 {
        1e300
    }
    extern "C" fn high() -> *const u8 {
        0xffff_8000_0000_1000usize as *const u8
    }
    extern "C" fn nothing() {}
    assert_eq!(
        f32::from_bits(returning(single as *const (), Type::F32) as u32),
        -2.5
    );
    assert_eq!(
        f64::from_bits(returning(double as *const (), Type::F64)),
        1e300
    );
    assert_eq!(
        returning(high as *const (), Type::Pointer),
        0xffff_8000_0000_1000
    );
    assert_eq!(returning(nothing as *const (), Type::Void), 0);
}

#[test]
fn an_argument_is_cut_to_its_declared_width() {
    extern "C" fn widen(a: i32, b: u8) -> i64 {
        a as i64 * 1000 + b as i64
    }
    let signature = Signature {
        args: vec![Type::I32, Type::U8],
        returns: Type::I64,
        fixed: None,
    };
    let slots = [0x1_ffff_fffe, 0x1_0000_0105];
    let through = unsafe { call(widen as *const (), &signature, &slots) };
    assert_eq!(through.value as i64, -2 * 1000 + 5);
}

#[cfg(windows)]
fn c_runtime(symbol: &str) -> *const () {
    let system = std::env::var_os("SystemRoot").expect("SystemRoot");
    let path = std::path::Path::new(&system)
        .join("System32")
        .join("msvcrt.dll");
    Library::open(&path).unwrap().symbol(symbol).unwrap()
}

#[cfg(windows)]
fn snprintf() -> *const () {
    c_runtime("_snprintf")
}

#[cfg(unix)]
fn snprintf() -> *const () {
    libc::snprintf as *const ()
}

#[test]
fn a_variadic_function_gets_integers_floats_and_pointers() {
    let mut out = [0u8; 64];
    let format = c"%d %.2f %s %.1f %d %d %d %d %.3f";
    let word = c"ok";
    let signature = Signature {
        args: vec![
            Type::Pointer,
            Type::Usize,
            Type::Pointer,
            Type::I32,
            Type::F64,
            Type::Pointer,
            Type::F64,
            Type::I32,
            Type::I32,
            Type::I32,
            Type::I32,
            Type::F64,
        ],
        returns: Type::I32,
        fixed: Some(3),
    };
    let slots = [
        out.as_mut_ptr() as u64,
        out.len() as u64,
        format.as_ptr() as u64,
        (-7i64) as u64,
        2.5f64.to_bits(),
        word.as_ptr() as u64,
        0.5f64.to_bits(),
        1,
        2,
        3,
        4,
        (-0.125f64).to_bits(),
    ];
    let written = unsafe { call(snprintf(), &signature, &slots) }.value as i32;
    let text = std::str::from_utf8(&out[..written as usize]).unwrap();
    assert_eq!(text, "-7 2.50 ok 0.5 1 2 3 4 -0.125");
}

#[cfg(windows)]
#[test]
fn the_last_error_is_read_after_the_call() {
    use windows_sys::Win32::Foundation::SetLastError;
    let signature = Signature {
        args: vec![Type::U32],
        returns: Type::Void,
        fixed: None,
    };
    let outcome = unsafe { call(SetLastError as *const (), &signature, &[1234]) };
    assert_eq!(outcome.os_error, 1234);
}

#[cfg(unix)]
#[test]
fn errno_is_read_after_the_call() {
    let signature = Signature {
        args: vec![Type::I32],
        returns: Type::I32,
        fixed: None,
    };
    let outcome = unsafe { call(libc::close as *const (), &signature, &[(-1i64) as u64]) };
    assert_eq!(outcome.value as i32, -1);
    assert_eq!(outcome.os_error, libc::EBADF);
}

#[test]
fn a_missing_library_and_a_missing_symbol_say_which() {
    let here = std::env::current_exe().unwrap();
    let missing = here.with_file_name("no-such-library.bin");
    let error = Library::open(&missing).unwrap_err();
    assert_eq!(error.symbol, None);
    assert!(error.library.ends_with("no-such-library.bin"));
    assert!(Library::open(std::path::Path::new("relative.dll")).is_err());
}

static POOL_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn pool_test() -> std::sync::MutexGuard<'static, ()> {
    POOL_TESTS.lock().unwrap_or_else(|e| e.into_inner())
}

fn mixing(types: Vec<Type>) -> Callback {
    let signature = Signature {
        args: types,
        returns: Type::U64,
        fixed: None,
    };
    let handler: Handler = std::sync::Arc::new(|slots: &[u64]| {
        slots.iter().fold(17u64, |mix, slot| {
            mix.wrapping_mul(1_000_003).wrapping_add(*slot)
        })
    });
    Callback::new(signature, handler).unwrap()
}

/// The compiler calls a thunk as a C function of the given parameters; the handler must see them all.
macro_rules! receives {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {$(
        #[test]
        #[allow(unused_mut, unused_assignments)]
        fn $name() {
            let _pool = pool_test();
            let callback = mixing(vec![$(<$ty as Arg>::TYPE),*]);
            let function: extern "C" fn($($ty),*) -> u64 =
                unsafe { std::mem::transmute(callback.pointer()) };
            let mut position = 0;
            let mut expected = 17u64;
            $(
                let $arg = <$ty as Arg>::sample(position);
                position += 1;
                expected = expected
                    .wrapping_mul(1_000_003)
                    .wrapping_add(<$ty as Arg>::TYPE.normalize($arg.slot()));
            )*
            let _ = position;
            assert_eq!(function($($arg),*), expected);
            callback.release();
        }
    )*};
}

receives! {
    a_callback_without_arguments();
    a_callback_of_each_integer(a: i8, b: u8, c: i16, d: u16, e: i32, f: u32, g: i64, h: u64);
    a_callback_of_integers_and_floats(a: i32, b: f64, c: u64, d: f32, e: i8, f: f64, g: u16, h: f32);
    a_callback_with_small_integers_on_the_stack(
        a: u64, b: u64, c: u64, d: u64, e: u64, f: u64, g: u64, h: u64,
        i: i8, j: i16, k: i8, l: i32, m: u8, n: u64, o: bool, p: u16
    );
    a_callback_with_both_kinds_past_their_registers(
        a: i64, b: f64, c: i64, d: f64, e: i64, f: f64, g: i64, h: f64, i: i64, j: f64,
        k: i64, l: f64, m: i64, n: f64, o: i64, p: f64, q: i64, r: f64, s: i32, t: f32,
        u: i8, v: f32, w: u16, x: f64
    );
    a_callback_of_pointers(a: *const u8, b: usize, c: isize, d: *const u8, e: bool, f: *const u8);
}

fn answering(returns: Type, value: u64) -> Callback {
    let signature = Signature {
        args: vec![],
        returns,
        fixed: None,
    };
    Callback::new(signature, std::sync::Arc::new(move |_: &[u64]| value)).unwrap()
}

#[test]
fn a_callback_returns_integers_and_floats() {
    let _pool = pool_test();
    let byte = answering(Type::U8, 0xdead_01ff);
    let double = answering(Type::F64, 2.5f64.to_bits());
    let single = answering(Type::F32, (-1.5f32).to_bits() as u64);
    unsafe {
        let f: extern "C" fn() -> u8 = std::mem::transmute(byte.pointer());
        assert_eq!(f(), 0xff);
        let f: extern "C" fn() -> f64 = std::mem::transmute(double.pointer());
        assert_eq!(f(), 2.5);
        let f: extern "C" fn() -> f32 = std::mem::transmute(single.pointer());
        assert_eq!(f(), -1.5);
    }
    byte.release();
    double.release();
    single.release();
}

#[test]
fn each_thunk_reaches_its_own_handler_and_none_after_release() {
    let _pool = pool_test();
    let callbacks: Vec<Callback> = (0..CALLBACK_CAPACITY as u64)
        .map(|n| answering(Type::U64, n * 7 + 1))
        .collect();
    assert_eq!(live_callbacks(), CALLBACK_CAPACITY);
    let full = Signature {
        args: vec![],
        returns: Type::Void,
        fixed: None,
    };
    assert!(Callback::new(full, std::sync::Arc::new(|_: &[u64]| 0)).is_none());
    let functions: Vec<extern "C" fn() -> u64> = callbacks
        .iter()
        .map(|c| unsafe { std::mem::transmute(c.pointer()) })
        .collect();
    let answers: Vec<u64> = functions.iter().map(|f| f()).collect();
    for callback in callbacks {
        callback.release();
    }
    assert_eq!(answers.len(), CALLBACK_CAPACITY);
    assert!(answers.iter().all(|a| a % 7 == 1));
    let mut distinct = answers.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), CALLBACK_CAPACITY);
    assert_eq!(live_callbacks(), 0);
    assert_eq!(functions[0](), 0);
}

#[test]
fn a_callback_runs_on_the_thread_that_calls_it() {
    let _pool = pool_test();
    let callback = mixing(vec![Type::I32, Type::F64]);
    let address = callback.pointer() as usize;
    let seen = std::thread::spawn(move || {
        let function: extern "C" fn(i32, f64) -> u64 = unsafe { std::mem::transmute(address) };
        function(-3, 0.5)
    })
    .join()
    .unwrap();
    let expected = [(-3i64) as u64, 0.5f64.to_bits()]
        .iter()
        .fold(17u64, |mix, slot| {
            mix.wrapping_mul(1_000_003).wrapping_add(*slot)
        });
    assert_eq!(seen, expected);
    callback.release();
}

extern "C" fn write_to(address: *mut u8) {
    unsafe { address.write_volatile(1) }
}

#[test]
fn a_fault_in_a_foreign_function_names_it() {
    const NAME: &str = "tests::a_fault_in_a_foreign_function_names_it";
    if std::env::var_os("SK_FFI_FAULT").is_some() {
        let signature = Signature {
            args: vec![Type::Pointer],
            returns: Type::Void,
            fixed: None,
        };
        in_call(c"probe!write_to", || unsafe {
            call(write_to as *const (), &signature, &[8])
        });
        return;
    }
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
        .env("SK_FFI_FAULT", "1")
        .output()
        .unwrap();
    assert!(!child.status.success());
    let said = String::from_utf8_lossy(&child.stderr);
    assert!(said.contains("[crash] ffi: probe!write_to\n"), "{said}");
}

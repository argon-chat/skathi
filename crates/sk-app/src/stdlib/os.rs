//! `skathi:os`: the operating system as it is, read once on first use.

use std::ffi::c_void;
use std::sync::OnceLock;

use sk_runtime::host_fn;
use sk_runtime::redjs::webapi::util::*;

const PLATFORM: &str = if cfg!(target_os = "windows") {
    "win32"
} else if cfg!(target_os = "macos") {
    "darwin"
} else {
    "linux"
};

const ARCH: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    "x64"
};

struct Distro {
    name: String,
    version: String,
    id: String,
}

struct Info {
    machine_arch: &'static str,
    release: String,
    distro: Option<Distro>,
    hostname: String,
    homedir: String,
    tmpdir: String,
}

fn info() -> &'static Info {
    static INFO: OnceLock<Info> = OnceLock::new();
    INFO.get_or_init(|| Info {
        machine_arch: platform::machine_arch().unwrap_or(ARCH),
        release: platform::release(),
        distro: platform::distro(),
        hostname: platform::hostname(),
        homedir: std::env::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        tmpdir: directory(&std::env::temp_dir().to_string_lossy()),
    })
}

/// Without the trailing separator `GetTempPath2W` and `$TMPDIR` carry, as Node's `os.tmpdir()`.
fn directory(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed.ends_with(':') {
        path.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(target_os = "linux")]
fn parse_os_release(text: &str) -> Option<Distro> {
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .map(|value| value.trim().trim_matches(['"', '\'']).to_string())
            .unwrap_or_default()
    };
    let distro = Distro {
        name: field("NAME"),
        version: field("VERSION_ID"),
        id: field("ID"),
    };
    (!distro.name.is_empty() || !distro.id.is_empty()).then_some(distro)
}

#[cfg(target_os = "windows")]
mod platform {
    use super::Distro;

    const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
    const IMAGE_FILE_MACHINE_ARM64: u16 = 0xAA64;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn IsWow64Process2(
            process: isize,
            process_machine: *mut u16,
            native_machine: *mut u16,
        ) -> i32;
    }

    /// The machine under an emulated process too: an x64 host under Prism answers arm64.
    pub(super) fn machine_arch() -> Option<&'static str> {
        let (mut process, mut native) = (0u16, 0u16);
        if unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) } == 0 {
            return None;
        }
        match native {
            IMAGE_FILE_MACHINE_ARM64 => Some("arm64"),
            IMAGE_FILE_MACHINE_AMD64 => Some("x64"),
            _ => None,
        }
    }

    pub(super) fn release() -> String {
        use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
        use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
        let mut version: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
        version.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
        // `RtlGetVersion`: `GetVersionEx` reports 6.2 to an unmanifested process.
        if unsafe { RtlGetVersion(&mut version) } < 0 {
            return String::new();
        }
        format!(
            "{}.{}.{}",
            version.dwMajorVersion, version.dwMinorVersion, version.dwBuildNumber
        )
    }

    pub(super) fn distro() -> Option<Distro> {
        None
    }

    pub(super) fn hostname() -> String {
        use windows_sys::Win32::System::SystemInformation::{
            ComputerNameDnsHostname, GetComputerNameExW,
        };
        let mut buffer = [0u16; 256];
        let mut len = buffer.len() as u32;
        if unsafe { GetComputerNameExW(ComputerNameDnsHostname, buffer.as_mut_ptr(), &mut len) }
            == 0
        {
            return String::new();
        }
        String::from_utf16_lossy(&buffer[..len as usize])
    }
}

#[cfg(unix)]
fn posix_hostname() -> String {
    let mut buffer = [0u8; 256];
    if unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0 {
        return String::new();
    }
    let len = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[..len]).into_owned()
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Distro;

    fn sysctl_flag(name: &std::ffi::CStr) -> Option<bool> {
        let mut value: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>();
        let read = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&mut value as *mut libc::c_int).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (read == 0).then_some(value != 0)
    }

    /// The machine under Rosetta too: a translated x64 host answers arm64.
    pub(super) fn machine_arch() -> Option<&'static str> {
        if sysctl_flag(c"sysctl.proc_translated") == Some(true)
            || sysctl_flag(c"hw.optional.arm64") == Some(true)
        {
            Some("arm64")
        } else {
            Some("x64")
        }
    }

    pub(super) fn release() -> String {
        let mut buffer = [0u8; 64];
        let mut len = buffer.len();
        let read = unsafe {
            libc::sysctlbyname(
                c"kern.osproductversion".as_ptr(),
                buffer.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if read != 0 {
            return String::new();
        }
        let len = buffer
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(len.min(buffer.len()));
        String::from_utf8_lossy(&buffer[..len]).into_owned()
    }

    pub(super) fn distro() -> Option<Distro> {
        None
    }

    pub(super) fn hostname() -> String {
        super::posix_hostname()
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::Distro;

    fn uname() -> Option<libc::utsname> {
        let mut name: libc::utsname = unsafe { std::mem::zeroed() };
        (unsafe { libc::uname(&mut name) } == 0).then_some(name)
    }

    fn text(field: &[libc::c_char]) -> String {
        let bytes: Vec<u8> = field
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    pub(super) fn machine_arch() -> Option<&'static str> {
        match text(&uname()?.machine).as_str() {
            "aarch64" | "arm64" => Some("arm64"),
            "x86_64" | "amd64" => Some("x64"),
            _ => None,
        }
    }

    pub(super) fn release() -> String {
        uname().map(|name| text(&name.release)).unwrap_or_default()
    }

    pub(super) fn distro() -> Option<Distro> {
        ["/etc/os-release", "/usr/lib/os-release"]
            .iter()
            .find_map(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| super::parse_os_release(&text))
    }

    pub(super) fn hostname() -> String {
        super::posix_hostname()
    }
}

host_fn!(fn platform_name(g, _args) { unsafe { js_string(g, PLATFORM) } });
host_fn!(fn arch(g, _args) { unsafe { js_string(g, ARCH) } });
host_fn!(fn machine_arch(g, _args) { unsafe { js_string(g, info().machine_arch) } });
host_fn!(fn release(g, _args) { unsafe { js_string(g, &info().release) } });
host_fn!(fn hostname(g, _args) { unsafe { js_string(g, &info().hostname) } });
host_fn!(fn homedir(g, _args) { unsafe { js_string(g, &info().homedir) } });
host_fn!(fn tmpdir(g, _args) { unsafe { js_string(g, &info().tmpdir) } });

host_fn!(fn distro(g, _args) {
    let Some(distro) = &info().distro else { return NULL };
    unsafe {
        let object = new_object(g, UNDEF);
        for (key, value) in [("name", &distro.name), ("version", &distro.version), ("id", &distro.id)] {
            define(g, object, key, js_string(g, value), 0);
        }
        super::frozen(g, object)
    }
});

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let class = super::support::class(
        g,
        &ClassSpec {
            name: "OS",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[],
            getters: &[
                ("platform", platform_name),
                ("arch", arch),
                ("machineArch", machine_arch),
                ("release", release),
                ("distro", distro),
                ("hostname", hostname),
                ("homedir", homedir),
                ("tmpdir", tmpdir),
            ],
            accessors: &[],
        },
    );
    define(g, std, "OS", class.ctor, 0);
    define(g, std, "os", new_object(g, class.proto), 0);
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_directory_loses_its_trailing_separator_but_a_root_keeps_it() {
        assert_eq!(
            super::directory("C:\\Users\\me\\Temp\\"),
            "C:\\Users\\me\\Temp"
        );
        assert_eq!(super::directory("/tmp/"), "/tmp");
        assert_eq!(super::directory("/"), "/");
        assert_eq!(super::directory("C:\\"), "C:\\");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn os_release_fields_are_unquoted() {
        let distro = super::parse_os_release(
            "NAME=\"Ubuntu\"\nVERSION_ID=\"26.04\"\nID=ubuntu\nID_LIKE=debian\n",
        )
        .unwrap();
        assert_eq!(
            (
                distro.name.as_str(),
                distro.version.as_str(),
                distro.id.as_str()
            ),
            ("Ubuntu", "26.04", "ubuntu")
        );
    }
}

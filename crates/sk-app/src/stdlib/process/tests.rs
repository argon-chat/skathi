use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::child::{self, Exit, Request, Signal, Spawned, Stdio};
use super::live::{self, Pipe, QUEUE_LIMIT, READ_SIZE};
use super::resolve::{self, Refusal};
use crate::permissions::{Permissions, ProcessArg, ProcessArgs, ProcessEntry};

/// This test binary is also the child: started with `SK_PROCESS_PROBE` set, it runs `probe`
/// before libtest's `main` and exits.
mod probe {
    #[used]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
    #[cfg_attr(
        target_vendor = "apple",
        unsafe(link_section = "__DATA,__mod_init_func")
    )]
    static PROBE: extern "C" fn() = run;

    fn args() -> Vec<String> {
        #[cfg(target_os = "linux")]
        {
            let raw = std::fs::read("/proc/self/cmdline").unwrap_or_default();
            let mut parts: Vec<String> = raw
                .split(|b| *b == 0)
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .collect();
            parts.pop();
            parts
        }
        #[cfg(not(target_os = "linux"))]
        std::env::args().collect()
    }

    extern "C" fn run() {
        let Some(mode) = std::env::var_os("SK_PROCESS_PROBE") else {
            return;
        };
        let args: Vec<String> = args().into_iter().skip(1).collect();
        let mut out = std::io::stdout().lock();
        use std::io::{Read, Write};
        let code = match mode.to_str().unwrap_or("") {
            "argv" => {
                for arg in &args {
                    let hex: String = arg.bytes().map(|b| format!("{b:02x}")).collect();
                    writeln!(out, "{hex}").unwrap();
                }
                0
            }
            "exit" => args[0].parse().unwrap(),
            "echo" => {
                let mut input = Vec::new();
                std::io::stdin().read_to_end(&mut input).unwrap();
                out.write_all(&input).unwrap();
                0
            }
            "big" => {
                let size: usize = args[0].parse().unwrap();
                let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
                out.write_all(&bytes).unwrap();
                eprint!("done");
                0
            }
            "env" => {
                for key in &args {
                    writeln!(
                        out,
                        "{key}={}",
                        std::env::var(key).unwrap_or_else(|_| "-".into())
                    )
                    .unwrap();
                }
                0
            }
            "handle" => i32::from(super::handle_open(args[0].parse().unwrap())),
            "tree" => {
                let grandchild = std::process::Command::new(std::env::current_exe().unwrap())
                    .env("SK_PROCESS_PROBE", "sleep")
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                writeln!(out, "{}", grandchild.id()).unwrap();
                out.flush().unwrap();
                std::thread::sleep(std::time::Duration::from_secs(60));
                0
            }
            _ => {
                std::thread::sleep(std::time::Duration::from_secs(60));
                0
            }
        };
        out.flush().unwrap();
        std::process::exit(code);
    }
}

#[cfg(windows)]
fn handle_open(value: usize) -> bool {
    let mut flags = 0u32;
    // SAFETY: only asks whether the value names a handle of this process.
    unsafe { windows_sys::Win32::Foundation::GetHandleInformation(value as _, &mut flags) != 0 }
}

#[cfg(unix)]
fn handle_open(value: usize) -> bool {
    // SAFETY: only asks whether the descriptor is open.
    unsafe { libc::fcntl(value as i32, libc::F_GETFD) >= 0 }
}

fn me() -> PathBuf {
    resolve::canonical(&std::env::current_exe().unwrap()).unwrap()
}

fn entry(name: &str, path: &str, args: ProcessArgs) -> ProcessEntry {
    ProcessEntry {
        name: name.into(),
        path: path.into(),
        args,
    }
}

fn allow(entries: Vec<ProcessEntry>, app_dir: &Path) -> Permissions {
    Permissions {
        process: entries,
        app_dir: app_dir.to_path_buf(),
        ..Permissions::default()
    }
}

fn start(mode: &str, args: &[&str], stdin: Stdio) -> Spawned {
    let program = me();
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    child::spawn(Request {
        program: &program,
        args: &args,
        env: resolve::child_env(
            std::env::vars_os(),
            &[("SK_PROCESS_PROBE".into(), mode.into())],
            None,
        ),
        cwd: None,
        stdin,
        stdout: Stdio::Pipe,
        stderr: Stdio::Pipe,
    })
    .unwrap()
}

fn output(spawned: &mut Spawned) -> Vec<u8> {
    let mut out = Vec::new();
    spawned
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut out)
        .unwrap();
    out
}

struct TempDir(PathBuf);

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(name: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!("sk-process-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TempDir(resolve::canonical(&dir).unwrap())
}

fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    // SAFETY: opens and closes a handle; asks for nothing but whether it still runs.
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return false;
        }
        let running = WaitForSingleObject(h, 0) == WAIT_TIMEOUT;
        CloseHandle(h);
        running
    }
    #[cfg(unix)]
    // SAFETY: signal 0 only checks that the process exists.
    unsafe {
        libc::kill(pid as i32, 0) == 0
    }
}

fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn nothing_is_allowed_without_an_entry() {
    let refused = resolve::resolve(&Permissions::default(), "tool", &[], None);
    assert!(matches!(refused, Err(Refusal::NotAllowed(_))));
    let me = me();
    let refused = resolve::resolve(&Permissions::default(), me.to_str().unwrap(), &[], None);
    assert!(matches!(refused, Err(Refusal::NotAllowed(_))));
}

#[test]
fn a_program_is_an_entry_name_or_an_absolute_path_the_list_resolves_to() {
    let me = me();
    let dir = me.parent().unwrap();
    let name = me.file_name().unwrap().to_str().unwrap();
    let p = allow(
        vec![entry("probe", &format!("$APP/{name}"), ProcessArgs::Any)],
        dir,
    );
    assert_eq!(resolve::resolve(&p, "probe", &[], None), Ok(me.clone()));
    assert_eq!(
        resolve::resolve(&p, me.to_str().unwrap(), &[], None),
        Ok(me.clone())
    );
    assert!(matches!(
        resolve::resolve(&p, "other", &[], None),
        Err(Refusal::NotAllowed(_))
    ));
    assert!(matches!(
        resolve::resolve(&p, name, &[], None),
        Err(Refusal::NotAllowed(_))
    ));
    let relative = format!(".{}{name}", std::path::MAIN_SEPARATOR);
    assert!(matches!(
        resolve::resolve(&p, &relative, &[], None),
        Err(Refusal::NotAllowed(_))
    ));

    let around = dir.join("..").join(dir.file_name().unwrap()).join(name);
    assert_eq!(
        resolve::resolve(&p, around.to_str().unwrap(), &[], None),
        Ok(me.clone())
    );
    if cfg!(windows) {
        let upper = me.to_str().unwrap().to_uppercase();
        assert_eq!(resolve::resolve(&p, &upper, &[], None), Ok(me.clone()));
        let dotted = format!("{}. ", me.display());
        assert_eq!(resolve::resolve(&p, &dotted, &[], None), Ok(me.clone()));
    }

    let elsewhere = temp_dir("elsewhere");
    let copy = elsewhere.join(name);
    std::fs::copy(&me, &copy).unwrap();
    assert!(matches!(
        resolve::resolve(&p, copy.to_str().unwrap(), &[], None),
        Err(Refusal::NotAllowed(_))
    ));
}

#[cfg(windows)]
#[test]
fn a_short_name_matches_the_entry_of_the_long_one() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let dir = temp_dir("short");
    let long = dir.join("a program with a long name.exe");
    std::fs::copy(me(), &long).unwrap();
    let wide: Vec<u16> = long.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buf = [0u16; 1024];
    // SAFETY: a NUL-terminated path and a buffer as long as said.
    let n = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetShortPathNameW(
            wide.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as u32,
        )
    } as usize;
    let short = PathBuf::from(OsString::from_wide(&buf[..n]));
    if n == 0 || short == long {
        return;
    }
    let p = allow(
        vec![entry("long", long.to_str().unwrap(), ProcessArgs::None)],
        &dir,
    );
    assert_eq!(
        resolve::resolve(&p, short.to_str().unwrap(), &[], None),
        Ok(long)
    );
}

#[cfg(unix)]
#[test]
fn a_link_is_matched_by_what_it_points_to() {
    let dir = temp_dir("link");
    let real = dir.join("real");
    std::fs::copy(me(), &real).unwrap();
    let link = dir.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let p = allow(
        vec![entry("real", real.to_str().unwrap(), ProcessArgs::None)],
        &dir,
    );
    assert_eq!(
        resolve::resolve(&p, link.to_str().unwrap(), &[], None),
        Ok(real.clone())
    );
    let p = allow(
        vec![entry("link", link.to_str().unwrap(), ProcessArgs::None)],
        &dir,
    );
    assert_eq!(resolve::resolve(&p, "link", &[], None), Ok(real));
}

#[test]
fn an_entry_cannot_leave_the_app_or_search_the_current_folder() {
    let dir = temp_dir("leave");
    std::fs::copy(
        me(),
        dir.join(if cfg!(windows) { "tool.exe" } else { "tool" }),
    )
    .unwrap();
    let app = dir.join("app");
    std::fs::create_dir_all(&app).unwrap();
    for path in [
        "$APP/../tool",
        "$APP/",
        "$SYSTEM/../tool",
        "$PATH/sub/tool",
        "tool",
        "./tool",
    ] {
        let p = allow(vec![entry("tool", path, ProcessArgs::None)], &app);
        assert!(resolve::resolve(&p, "tool", &[], None).is_err(), "{path}");
    }
    let p = allow(vec![entry("tool", "$PATH/tool", ProcessArgs::None)], &app);
    let relative = OsString::from(format!(".{}app", if cfg!(windows) { ";" } else { ":" }));
    assert!(resolve::resolve(&p, "tool", &[], Some(&relative)).is_err());
    assert!(resolve::resolve(&p, "tool", &[], Some(dir.as_os_str())).is_ok());
}

#[cfg(windows)]
#[test]
fn batch_files_and_files_that_are_not_executables_are_refused() {
    let dir = temp_dir("batch");
    std::fs::write(dir.join("run.bat"), "@echo off\r\necho %1\r\n").unwrap();
    std::fs::write(dir.join("run.cmd"), "@echo off\r\n").unwrap();
    for path in ["$APP/run.bat", "$APP/run.cmd", "$APP/run"] {
        let p = allow(vec![entry("run", path, ProcessArgs::Any)], &dir);
        assert!(
            resolve::resolve(&p, "run", &["&calc".into()], None).is_err(),
            "{path}"
        );
    }
    std::fs::write(dir.join("text.exe"), "@echo off\r\n").unwrap();
    let text = dir.join("text.exe");
    let refused = child::spawn(Request {
        program: &text,
        args: &[],
        env: Vec::new(),
        cwd: None,
        stdin: Stdio::Null,
        stdout: Stdio::Null,
        stderr: Stdio::Null,
    });
    assert!(refused.is_err());
}

#[cfg(unix)]
#[test]
fn a_file_without_a_known_header_is_not_handed_to_a_shell() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("header");
    let script = dir.join("script");
    std::fs::write(&script, "touch should-not-exist\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let refused = child::spawn(Request {
        program: &script,
        args: &[],
        env: Vec::new(),
        cwd: Some(&*dir),
        stdin: Stdio::Null,
        stdout: Stdio::Null,
        stderr: Stdio::Null,
    });
    assert!(refused.is_err());
    assert!(!dir.join("should-not-exist").exists());
}

#[test]
fn arguments_follow_the_rules_of_the_entry_position_by_position() {
    let git = ProcessArgs::Only(vec![
        ProcessArg::Is("status".into()),
        ProcessArg::Matches("--porcelain(=v[12])?".into()),
    ]);
    let ok = |args: &[&str]| {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        resolve::args_allowed(&git, &args).is_ok()
    };
    assert!(ok(&["status", "--porcelain"]));
    assert!(ok(&["status", "--porcelain=v2"]));
    assert!(!ok(&["-c", "core.sshCommand=evil", "fetch"]));
    assert!(!ok(&["status"]));
    assert!(!ok(&["status", "--porcelain", "-c"]));
    assert!(!ok(&["status", "x--porcelain"]));
    assert!(!ok(&["status", "--porcelain\n-c"]));
    assert!(!ok(&["Status", "--porcelain"]));
    assert!(resolve::args_allowed(&ProcessArgs::None, &[]).is_ok());
    assert!(resolve::args_allowed(&ProcessArgs::None, &["".into()]).is_err());
    let broken = ProcessArgs::Only(vec![ProcessArg::Matches("(".into())]);
    assert!(resolve::args_allowed(&broken, &["(".into()]).is_err());
}

#[test]
fn the_child_environment_leaves_out_skathi_and_appimage_and_refuses_loader_variables() {
    let host = [
        ("HOME", "/home/u"),
        ("SK_PAYLOAD", "/app.skpk"),
        ("sk_devtools", "1"),
        ("APPDIR", "/tmp/.mount_x"),
        ("APPIMAGE", "/a/X.AppImage"),
        ("PATH", "/tmp/.mount_x/usr/bin:/usr/bin"),
        ("LD_LIBRARY_PATH", "/tmp/.mount_x/usr/lib"),
        ("LANG", "C"),
    ]
    .map(|(k, v)| (OsString::from(k), OsString::from(v)));
    let env = resolve::child_env(
        host.clone(),
        &[("LANG".into(), "de_DE".into())],
        Some("/tmp/.mount_x"),
    );
    let get = |k: &str| {
        env.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.to_str().unwrap())
    };
    assert_eq!(get("HOME"), Some("/home/u"));
    assert_eq!(get("PATH"), Some("/usr/bin"));
    assert_eq!(get("LANG"), Some("de_DE"));
    for gone in [
        "SK_PAYLOAD",
        "sk_devtools",
        "APPDIR",
        "APPIMAGE",
        "LD_LIBRARY_PATH",
    ] {
        assert_eq!(get(gone), None, "{gone}");
    }
    let outside = resolve::child_env(host, &[], None);
    assert!(outside.iter().any(|(k, _)| k == "APPDIR"));
    for key in [
        "PATH",
        "Path",
        "LD_PRELOAD",
        "DYLD_INSERT_LIBRARIES",
        "BASH_ENV",
        "ComSpec",
        "BASH_FUNC_x%%",
    ] {
        assert!(resolve::forbidden_key(key), "{key}");
    }
    assert!(!resolve::forbidden_key("LANG"));
}

#[test]
fn arguments_arrive_byte_for_byte() {
    let tricky = [
        "",
        " ",
        "two words",
        "\"",
        "\\",
        "\\\\\"",
        "a\\\\b",
        "trailing\\",
        "\"quoted\" \\\"",
        "%PATH%",
        "!PATH!",
        "& calc | x > y < z ^ (a)",
        "$(echo x) `x` $HOME",
        "line\nbreak\r\n",
        "tab\there",
        "ünïcödé 🙂 ж",
        "-c",
    ];
    let mut spawned = start("argv", &tricky, Stdio::Null);
    let out = String::from_utf8(output(&mut spawned)).unwrap();
    let got: Vec<String> = out
        .lines()
        .map(|hex| {
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            String::from_utf8(bytes).unwrap()
        })
        .collect();
    assert_eq!(got, tricky);
    assert_eq!(spawned.child.wait(), Exit::Code(0));
}

#[test]
fn the_exit_code_and_the_signal_are_reported() {
    let spawned = start("exit", &["42"], Stdio::Null);
    assert_eq!(spawned.child.wait(), Exit::Code(42));
    let spawned = start("sleep", &[], Stdio::Null);
    spawned.child.kill(Signal::Term);
    assert_eq!(spawned.child.wait(), Exit::Signal("SIGTERM".into()));
    let spawned = start("sleep", &[], Stdio::Null);
    spawned.child.kill(Signal::Kill);
    assert_eq!(spawned.child.wait(), Exit::Signal("SIGKILL".into()));
    spawned.child.kill(Signal::Kill);
}

#[test]
fn stdin_comes_back_on_stdout_while_both_are_full() {
    let mut spawned = start("echo", &[], Stdio::Pipe);
    let input: Vec<u8> = (0..8 * 1024 * 1024u32)
        .map(|i| (i * 7 % 256) as u8)
        .collect();
    let mut stdin = spawned.stdin.take().unwrap();
    let sent = input.clone();
    let writer = std::thread::spawn(move || stdin.write_all(&sent));
    let out = output(&mut spawned);
    writer.join().unwrap().unwrap();
    assert!(
        out == input,
        "{} of {} bytes came back",
        out.len(),
        input.len()
    );
    assert_eq!(spawned.child.wait(), Exit::Code(0));
}

#[test]
fn a_reader_stops_at_its_limit_until_the_bytes_are_taken() {
    let size = 16 * 1024 * 1024;
    let mut spawned = start("big", &[&size.to_string()], Stdio::Null);
    let pipe = Arc::new(Pipe::default());
    let file = spawned.stdout.take().unwrap();
    let (p, c) = (pipe.clone(), spawned.child.clone());
    std::thread::spawn(move || live::reader(0, file, p, c));
    eventually("the reader filled its queue", || {
        pipe.state.lock().unwrap().bytes >= QUEUE_LIMIT
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(pipe.state.lock().unwrap().bytes < QUEUE_LIMIT + READ_SIZE);
    assert!(!spawned.child.exited());
    let mut got = Vec::with_capacity(size);
    loop {
        let mut s = pipe.state.lock().unwrap();
        while let Some(chunk) = s.chunks.pop_front() {
            got.extend_from_slice(&chunk);
        }
        s.bytes = 0;
        let done = s.done.is_some();
        drop(s);
        pipe.room.notify_all();
        if done {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(got.len(), size);
    assert!(got.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8));
    let mut err = String::new();
    spawned
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert_eq!(err, "done");
    assert_eq!(spawned.child.wait(), Exit::Code(0));
}

#[test]
fn killing_a_child_ends_what_it_started() {
    let mut spawned = start("tree", &[], Stdio::Null);
    let mut line = Vec::new();
    let mut stdout = spawned.stdout.take().unwrap();
    let mut byte = [0u8; 1];
    while stdout.read(&mut byte).unwrap() == 1 && byte[0] != b'\n' {
        line.push(byte[0]);
    }
    let grandchild: u32 = String::from_utf8(line).unwrap().trim().parse().unwrap();
    assert!(alive(grandchild));
    spawned.child.kill(Signal::Kill);
    spawned.child.wait();
    eventually("the grandchild ended", || !alive(grandchild));
}

#[test]
fn a_dropped_handle_ends_the_child() {
    let spawned = start("sleep", &[], Stdio::Null);
    let pid = spawned.child.pid;
    let child = spawned.child.clone();
    let waiter = std::thread::spawn(move || child.wait());
    spawned.child.end();
    assert!(waiter.join().unwrap() != Exit::Code(0));
    assert!(!alive(pid));
}

#[test]
fn only_stdio_reaches_the_child() {
    #[cfg(windows)]
    let leaked = {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let file = std::fs::File::open(me()).unwrap();
        let mut dup: HANDLE = std::ptr::null_mut();
        // SAFETY: an inheritable copy of a handle of this process, closed with the process.
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                file.as_raw_handle() as HANDLE,
                GetCurrentProcess(),
                &mut dup,
                0,
                1,
                DUPLICATE_SAME_ACCESS,
            );
        }
        dup as usize
    };
    #[cfg(unix)]
    let leaked = {
        let path = std::ffi::CString::new(me().to_str().unwrap()).unwrap();
        // SAFETY: a descriptor opened without O_CLOEXEC on purpose, closed with the process.
        unsafe { libc::open(path.as_ptr(), libc::O_RDONLY) as usize }
    };
    assert!(handle_open(leaked));
    let spawned = start("handle", &[&leaked.to_string()], Stdio::Null);
    assert_eq!(spawned.child.wait(), Exit::Code(0));
}

#[test]
fn from_js_streams_promises_limits_and_collection() {
    let me = me();
    let permissions: &'static Permissions = Box::leak(Box::new(allow(
        vec![entry("probe", me.to_str().unwrap(), ProcessArgs::Any)],
        me.parent().unwrap(),
    )));
    sk_runtime::testing::on_js_thread(move || {
        live::TEST_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions));
        sk_runtime::JsRuntime::with(|rt| {
            crate::stdlib::install(rt);
            let context = rt.ctx();
            let eval = |src: &str| -> String {
                let v = context.eval(src).unwrap();
                context.read_string(v).unwrap()
            };
            eval(
                r#"(() => {
                const { Process } = __sk_std.process;
                const probe = (mode, args, more) => ["probe", args, { env: { SK_PROCESS_PROBE: mode }, ...more }];
                const name = (run) => { try { run(); return "ok"; } catch (e) { return e.name; } };
                globalThis.__out = {
                  unlisted: name(() => Process.spawn("other")),
                  path: name(() => Process.spawn("probe", [], { env: { Path: "x" } })),
                  args: name(() => Process.spawn("probe", ["a", 1])),
                };
                (async () => {
                  const p = Process.spawn(...probe("echo", []));
                  const writer = p.stdin.getWriter();
                  await writer.write(new TextEncoder().encode("hello "));
                  await writer.write(new Uint8Array([0xc3, 0xa9]));
                  await writer.close();
                  let text = "";
                  const decoder = new TextDecoder();
                  for await (const chunk of p.stdout) text += decoder.decode(chunk, { stream: true });
                  __out.echo = [text, await p.exited, typeof p.pid];
                  const r = await Process.exec(...probe("exit", ["7"]));
                  __out.exec = [r.code, r.signal, r.stdout];
                  const b = await Process.exec(...probe("big", ["70000"]));
                  __out.big = [b.stdout.length, b.stderr];
                  const failed = (promise) => promise.then(() => "ok", (e) => e.name);
                  __out.over = await failed(Process.exec(...probe("big", ["100000"], { maxBuffer: 1000 })));
                  __out.timeout = await failed(Process.exec(...probe("sleep", [], { timeout: 50 })));
                  const abort = new AbortController();
                  const aborted = Process.exec(...probe("sleep", [], { signal: abort.signal }));
                  abort.abort();
                  __out.abort = await failed(aborted);
                  const k = Process.spawn(...probe("sleep", [], { stdin: "ignore", stdout: "ignore" }));
                  __out.ignored = [k.stdin, k.stdout];
                  k.kill();
                  __out.killed = await k.exited;
                  __out.done = true;
                })().catch((e) => { __out.failed = String(e) + e.stack; });
                return "started";
              })()"#,
            );
            let deadline = Instant::now() + Duration::from_secs(20);
            while eval("String(!!(__out.done || __out.failed))") != "true" {
                assert!(
                    Instant::now() < deadline,
                    "{}",
                    eval("JSON.stringify(__out)")
                );
                super::pump();
                context.drain_microtasks();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                eval("JSON.stringify(__out)"),
                r#"{"unlisted":"NotAllowedError","path":"NotAllowedError","args":"TypeError","echo":["hello é",0,"number"],"exec":[7,null,""],"big":[70000,"done"],"over":"RangeError","timeout":"TimeoutError","abort":"AbortError","ignored":[null,null],"killed":null,"done":true}"#
            );

            let pid: u32 = eval(
                r#"(() => {
                const p = __sk_std.process.Process.spawn("probe", [], { env: { SK_PROCESS_PROBE: "sleep" } });
                return String(p.pid);
              })()"#,
            )
            .parse()
            .unwrap();
            assert!(alive(pid));
            let deadline = Instant::now() + Duration::from_secs(20);
            while alive(pid) {
                assert!(
                    Instant::now() < deadline,
                    "the collected process still runs"
                );
                eval("(() => { for (let i = 0; i < 1000; i++) new Array(1000); return ''; })()");
                context.gc();
                std::thread::sleep(Duration::from_millis(20));
            }
        });
    });
}

#[test]
fn the_environment_given_reaches_the_child() {
    let program = me();
    let mut spawned = child::spawn(Request {
        program: &program,
        args: &["GIVEN".into(), "SK_PAYLOAD".into()],
        env: resolve::child_env(
            std::env::vars_os().chain([(OsString::from("SK_PAYLOAD"), OsString::from("x"))]),
            &[
                ("SK_PROCESS_PROBE".into(), "env".into()),
                ("GIVEN".into(), "a b=c".into()),
            ],
            None,
        ),
        cwd: Some(&std::env::temp_dir()),
        stdin: Stdio::Null,
        stdout: Stdio::Pipe,
        stderr: Stdio::Null,
    })
    .unwrap();
    let out = String::from_utf8(output(&mut spawned)).unwrap();
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        ["GIVEN=a b=c", "SK_PAYLOAD=-"]
    );
}

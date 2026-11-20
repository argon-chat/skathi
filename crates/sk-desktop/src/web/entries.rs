//! Read-only File and Directory Entries for an OS drop. A virtual root includes exactly
//! the dropped items. Directory walks and file snapshots run on the desktop file pool.
use super::{awaiting, fsa, global};
use crate::{
    fs::{self, Access, Kind},
    pool, Done, Fail,
};
use sk_runtime::{
    host, host_fn,
    redjs::{
        capi::JSClassRef,
        glue,
        webapi::{file, uievents, util::*},
    },
};
use std::{
    cell::Cell,
    ffi::c_void,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Roots = Arc<Vec<(PathBuf, Kind)>>;
#[derive(Clone)]
struct Entry {
    roots: Roots,
    root: Option<usize>,
    below: PathBuf,
    kind: Kind,
}

impl Entry {
    fn path(&self) -> Option<PathBuf> {
        self.root.map(|i| {
            if self.below.as_os_str().is_empty() {
                self.roots[i].0.clone()
            } else {
                self.roots[i].0.join(&self.below)
            }
        })
    }
    fn virtual_path(&self) -> String {
        let Some(i) = self.root else {
            return "/".into();
        };
        let mut result = format!("/{}", fs::name_of(&self.roots[i].0));
        for part in self.below.iter() {
            result.push('/');
            result.push_str(&part.to_string_lossy());
        }
        result
    }
    fn virtual_root(roots: Roots) -> Self {
        Self {
            roots,
            root: None,
            below: PathBuf::new(),
            kind: Kind::Directory,
        }
    }
    fn parent(&self) -> Self {
        let mut next = self.clone();
        if !next.below.pop() {
            next.root = None;
        }
        next.kind = Kind::Directory;
        next
    }
    // A symlink anywhere below a dropped directory can only resolve inside that directory.
    fn checked_path(&self) -> Result<PathBuf, Fail> {
        let root = self.root.ok_or_else(missing)?;
        let top = &self.roots[root].0;
        let physical = self.path().ok_or_else(missing)?;
        if !self.below.as_os_str().is_empty() {
            let canonical = std::fs::canonicalize(&physical).map_err(io)?;
            if !canonical.starts_with(std::fs::canonicalize(top).map_err(io)?) {
                return Err(Fail::new(
                    "SecurityError",
                    "The entry is outside the dropped directory.",
                ));
            }
        }
        Ok(physical)
    }
    fn child(&self, name: &str, kind: Kind) -> Result<Self, Fail> {
        let mut e = if name.starts_with('/') {
            Self::virtual_root(self.roots.clone())
        } else {
            self.clone()
        };
        for part in name.split('/').filter(|p| !p.is_empty() && *p != ".") {
            if part == ".." {
                e = e.parent();
                continue;
            }
            if Path::new(part).components().count() != 1
                || part.contains(['\\', '\0'])
                || (cfg!(windows) && part.contains(':'))
            {
                return Err(Fail::new("EncodingError", "Invalid entry path."));
            }
            if e.root.is_some() {
                e.below.push(part);
            } else {
                e.root = Some(
                    e.roots
                        .iter()
                        .position(|(p, _)| fs::name_of(p) == part)
                        .ok_or_else(missing)?,
                );
            }
        }
        if e.root.is_some() {
            let meta = std::fs::metadata(e.checked_path()?).map_err(io)?;
            e.kind = if meta.is_dir() {
                Kind::Directory
            } else {
                Kind::File
            };
        }
        if e.kind != kind {
            return Err(Fail::new("TypeMismatchError", fs::TYPE_MISMATCH));
        }
        Ok(e)
    }
}

fn missing() -> Fail {
    Fail::new("NotFoundError", fs::NOT_FOUND)
}
fn io(e: std::io::Error) -> Fail {
    fs::fail_io(&e, Access::Read)
}

struct Reader {
    entry: Entry,
    cursor: Arc<Mutex<Cursor>>,
}
#[derive(Default)]
struct Cursor {
    physical: Option<fs::Cursor>,
    root_index: usize,
    done: bool,
    busy: bool,
}

#[derive(Clone, Copy, Default)]
struct Classes {
    entry: usize,
    reader: usize,
    fs: usize,
    file_proto: u64,
    dir_proto: u64,
    reader_proto: u64,
    fs_proto: u64,
    fs_slot: usize,
    root_slot: usize,
}
thread_local! { static C: Cell<Classes> = Cell::new(Classes::default()); }
unsafe fn raw_entry(g: *mut c_void, value: u64) -> Option<&'static mut Entry> {
    host_data::<Entry>(g, C.get().entry as JSClassRef, value)
}
pub(super) unsafe fn path_of(g: *mut c_void, value: u64) -> Option<PathBuf> {
    raw_entry(g, value)?.path()
}
unsafe fn make_entry(g: *mut c_void, e: Entry) -> u64 {
    let c = C.get();
    host_create(
        g,
        c.entry as JSClassRef,
        if e.kind == Kind::Directory {
            c.dir_proto
        } else {
            c.file_proto
        },
        e,
    )
}
unsafe fn attach(g: *mut c_void, obj: u64, slot: usize, value: u64) {
    glue::rjs_define(g, obj, slot as *mut c_void, value, 0);
}
unsafe fn filesystem_of(g: *mut c_void, obj: u64) -> u64 {
    get_a(g, obj, C.get().fs_slot as *mut c_void)
}

host_fn!(fn from_item(g, args) {
    unsafe {
        let Some((store, path, _)) = uievents::dropped_entry(g, args.this().0) else { return NULL };
        let c = C.get();
        let mut fs = filesystem_of(g, store);
        if !is_cell(fs) {
            let roots: Roots = Arc::new(uievents::dropped_roots(g, store).into_iter().map(|(p, d)| (p, if d { Kind::Directory } else { Kind::File })).collect());
            let root = make_entry(g, Entry::virtual_root(roots.clone()));
            fs = host_create(g, c.fs as JSClassRef, c.fs_proto, roots);
            attach(g, fs, c.root_slot, root);
            attach(g, root, c.fs_slot, fs);
            attach(g, store, c.fs_slot, fs);
        }
        let roots = host_data::<Roots>(g, c.fs as JSClassRef, fs).unwrap().clone();
        let Some(i) = roots.iter().position(|(p, _)| *p == path) else { return NULL };
        let e = make_entry(g, Entry { kind: roots[i].1, roots, root: Some(i), below: PathBuf::new() });
        attach(g, e, c.fs_slot, fs);
        e
    }
});

host_fn!(fn is_file(g, args) { unsafe { enc_bool(raw_entry(g, args.this().0).is_some_and(|e| e.kind == Kind::File)) } });
host_fn!(fn is_directory(g, args) { unsafe { enc_bool(raw_entry(g, args.this().0).is_some_and(|e| e.kind == Kind::Directory)) } });
host_fn!(fn name(g, args) { unsafe {
    let Some(e) = raw_entry(g, args.this().0) else { return throw_type_error(g, "Illegal invocation") };
    js_string(g, &e.path().map_or_else(String::new, |p| fs::name_of(&p)))
} });
host_fn!(fn full_path(g, args) { unsafe {
    let Some(e) = raw_entry(g, args.this().0) else { return throw_type_error(g, "Illegal invocation") };
    js_string(g, &e.virtual_path())
} });
host_fn!(fn filesystem(g, args) { unsafe { filesystem_of(g, args.this().0) } });
host_fn!(fn root(g, args) { unsafe { get_a(g, args.this().0, C.get().root_slot as *mut c_void) } });
host_fn!(fn fs_name(g, _args) { unsafe { js_string(g, "Dropped files") } });

unsafe fn callback(g: *mut c_void, v: u64) -> bool {
    is_cell(v) && glue::rjs_kind(g, v) == 9
}
host_fn!(fn ignore(_g, _args) { UNDEF });
host_fn!(fn deliver(g, args) { unsafe {
    let self_ = args.this().0;
    let fs = filesystem_of(g, self_);
    let result = args.arg(0).0;
    let kind = get(g, self_, "entries");
    if kind == TRUE {
        for i in 0..array_len(g, result) { attach(g, get_i(g, result, i), C.get().fs_slot, fs); }
    } else if raw_entry(g, result).is_some() {
        attach(g, result, C.get().fs_slot, fs);
    }
    let cb = get(g, self_, "callback");
    if callback(g, cb) {
        if let Err(e) = call(g, cb, UNDEF, &[result]) { glue::rjs_throw(g, e); }
    }
    UNDEF
} });

unsafe fn complete(
    g: *mut c_void,
    owner: u64,
    ok: u64,
    err: u64,
    entries: bool,
    job: impl FnOnce() -> Result<Done, Fail> + Send + 'static,
) -> u64 {
    if (ok != UNDEF && ok != NULL && !callback(g, ok))
        || (err != UNDEF && err != NULL && !callback(g, err))
    {
        return throw_type_error(g, "Entry callback must be a function");
    }
    let token = crate::next_token();
    let promise = awaiting(g, Ok(token), fsa::to_made);
    let context = new_object(g, UNDEF);
    attach(g, context, C.get().fs_slot, filesystem_of(g, owner));
    define(g, context, "callback", ok, 0);
    define(g, context, "entries", enc_bool(entries), 0);
    let on_success = make_fn(g, "", 1, deliver);
    let on_success = call(g, get(g, on_success, "bind"), on_success, &[context]).unwrap_or(UNDEF);
    let on_error = if callback(g, err) {
        err
    } else {
        make_fn(g, "", 0, ignore)
    };
    if let Err(e) = call(g, get(g, promise, "then"), promise, &[on_success, on_error]) {
        glue::rjs_throw(g, e);
    }
    pool::submit(token, job);
    UNDEF
}
fn one(e: Entry) -> Done {
    Done::with_value("null".into(), move |g| unsafe { make_entry(g, e) })
}

host_fn!(fn parent(g, args) { unsafe {
    let Some(e) = raw_entry(g, args.this().0).cloned() else { return throw_type_error(g, "Illegal invocation") };
    complete(g, args.this().0, args.arg(0).0, args.arg(1).0, false, move || Ok(one(e.parent())))
} });
host_fn!(fn entry_file(g, args) { unsafe {
    let Some(e) = raw_entry(g, args.this().0).cloned().filter(|e| e.kind == Kind::File) else { return throw_type_error(g, "Illegal invocation") };
    complete(g, args.this().0, args.arg(0).0, args.arg(1).0, false, move || {
        let p = e.checked_path()?;
        let (size, mtime) = fs::op_stat_file(&p)?;
        Ok(Done::with_value("null".into(), move |g| file::create_file(g, &p, &fs::name_of(&p), None, size, mtime, "")))
    })
} });
unsafe fn child(
    g: *mut c_void,
    this: u64,
    path: u64,
    options: u64,
    ok: u64,
    err: u64,
    kind: Kind,
) -> u64 {
    let Some(e) = raw_entry(g, this)
        .cloned()
        .filter(|e| e.kind == Kind::Directory)
    else {
        return throw_type_error(g, "Illegal invocation");
    };
    let path = host::string_arg(g, path).unwrap_or_default();
    let create = is_cell(options) && get(g, options, "create") == TRUE;
    complete(g, this, ok, err, false, move || {
        if create {
            return Err(Fail::new(
                "NoModificationAllowedError",
                "Dropped entries are read-only.",
            ));
        }
        Ok(one(e.child(&path, kind)?))
    })
}
host_fn!(fn get_file(g, args) { unsafe { child(g, args.this().0, args.arg(0).0, args.arg(1).0, args.arg(2).0, args.arg(3).0, Kind::File) } });
host_fn!(fn get_directory(g, args) { unsafe { child(g, args.this().0, args.arg(0).0, args.arg(1).0, args.arg(2).0, args.arg(3).0, Kind::Directory) } });

host_fn!(fn create_reader(g, args) { unsafe {
    let this = args.this().0;
    let Some(entry) = raw_entry(g, this).cloned().filter(|e| e.kind == Kind::Directory) else { return throw_type_error(g, "Illegal invocation") };
    let c = C.get();
    let reader = host_create(g, c.reader as JSClassRef, c.reader_proto, Reader { entry, cursor: Default::default() });
    attach(g, reader, c.fs_slot, filesystem_of(g, this));
    reader
} });
fn next_batch(e: &Entry, cursor: &mut Cursor) -> Result<Vec<Entry>, Fail> {
    const BATCH: usize = 100;
    if cursor.done {
        return Ok(vec![]);
    }
    if e.root.is_none() {
        let end = (cursor.root_index + BATCH).min(e.roots.len());
        let entries = (cursor.root_index..end)
            .map(|i| Entry {
                roots: e.roots.clone(),
                root: Some(i),
                below: PathBuf::new(),
                kind: e.roots[i].1,
            })
            .collect();
        cursor.root_index = end;
        cursor.done = end == e.roots.len();
        return Ok(entries);
    }
    let p = e.checked_path()?;
    if cursor.physical.is_none() {
        cursor.physical = Some(fs::Cursor::open(&p)?);
    }
    // An empty result means end-of-directory to the caller. Skip inaccessible
    // children without prematurely terminating a walk that still has more entries.
    loop {
        let (paths, done) = cursor.physical.as_mut().unwrap().next(BATCH)?;
        cursor.done = done;
        let mut children = Vec::new();
        for (p, kind) in paths {
            let Some(name) = p.file_name() else { continue };
            let child = Entry {
                roots: e.roots.clone(),
                root: e.root,
                below: e.below.join(name),
                kind,
            };
            if child.checked_path().is_ok() {
                children.push(child);
            }
        }
        if done || !children.is_empty() {
            return Ok(children);
        }
    }
}
host_fn!(fn read_entries(g, args) { unsafe {
    let this = args.this().0;
    let Some(r) = host_data::<Reader>(g, C.get().reader as JSClassRef, this) else { return throw_type_error(g, "Illegal invocation") };
    let entry = r.entry.clone();
    let cursor = r.cursor.clone();
    let ok = args.arg(0).0;
    let err = args.arg(1).0;
    if !callback(g, ok) || (err != UNDEF && err != NULL && !callback(g, err)) {
        return throw_type_error(g, "readEntries requires callback functions");
    }
    // try_lock keeps a second JS call from waiting on a worker's filesystem operation.
    let busy = match cursor.try_lock() { Ok(mut c) if !c.busy => { c.busy = true; false }, _ => true };
    complete(g, this, ok, err, true, move || {
        if busy { return Err(Fail::new("InvalidStateError", "A directory read is already pending.")); }
        let mut c = cursor.lock().unwrap_or_else(|p| p.into_inner());
        let result = next_batch(&entry, &mut c);
        c.busy = false;
        let entries = result?;
        Ok(Done::with_value("null".into(), move |g| {
            let out = new_array(g);
            for (i, e) in entries.into_iter().enumerate() { set_i(g, out, i as u32, make_entry(g,e)); }
            out
        }))
    })
} });

pub(super) unsafe fn install(g: *mut c_void) {
    let class = |name: &'static str,
                 parent: u64,
                 methods: &[(&'static str, u32, HostFn)],
                 getters: &[(&'static str, HostFn)]| {
        define_class(
            g,
            &ClassSpec {
                name,
                arity: 0,
                construct: None,
                parent_proto: parent,
                methods,
                getters,
                accessors: &[],
            },
        )
    };
    let base = class(
        "FileSystemEntry",
        0,
        &[("getParent", 1, parent)],
        &[
            ("name", name),
            ("fullPath", full_path),
            ("filesystem", filesystem),
            ("isFile", is_file),
            ("isDirectory", is_directory),
        ],
    );
    let f = class(
        "FileSystemFileEntry",
        base.proto,
        &[("file", 1, entry_file)],
        &[],
    );
    let d = class(
        "FileSystemDirectoryEntry",
        base.proto,
        &[
            ("createReader", 0, create_reader),
            ("getFile", 1, get_file),
            ("getDirectory", 1, get_directory),
        ],
        &[],
    );
    let r = class(
        "FileSystemDirectoryReader",
        0,
        &[("readEntries", 1, read_entries)],
        &[],
    );
    let fs = class("FileSystem", 0, &[], &[("name", fs_name), ("root", root)]);
    C.set(Classes {
        entry: make_host_class::<Entry>("FileSystemEntry") as usize,
        reader: make_host_class::<Reader>("FileSystemDirectoryReader") as usize,
        fs: make_host_class::<Roots>("FileSystem") as usize,
        file_proto: f.proto,
        dir_proto: d.proto,
        reader_proto: r.proto,
        fs_proto: fs.proto,
        fs_slot: atom_priv(g, "skathi#dropFilesystem") as usize,
        root_slot: atom_priv(g, "skathi#dropRoot") as usize,
    });
    let proto = get(g, get(g, global(g), "DataTransferItem"), "prototype");
    host::method(g, proto, "webkitGetAsEntry", 0, from_item);
}

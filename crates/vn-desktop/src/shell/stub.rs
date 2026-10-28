//! Non-Windows: the shell calls exist and refuse, so the crate builds and the
//! validation tests run everywhere. vue-native is Windows-first (ARCHITECTURE).

use std::path::Path;

use crate::Fail;

pub fn thread_init() {}

fn unsupported(verb: &str) -> Fail {
    Fail::new("NotSupportedError", format!("{verb} is not implemented on this platform yet")).code("ENOSYS")
}

pub fn open_external(_url: &str) -> Result<(), Fail> {
    Err(unsupported("openExternal"))
}

pub fn open_path(_path: &Path) -> Result<(), Fail> {
    Err(unsupported("openPath"))
}

pub fn show_item(_path: &Path) -> Result<(), Fail> {
    Err(unsupported("showItemInFolder"))
}

pub fn trash(_path: &Path) -> Result<(), Fail> {
    Err(unsupported("trashItem"))
}

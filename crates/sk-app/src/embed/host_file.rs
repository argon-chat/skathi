use serde::de::{self, Deserialize, Visitor};
use std::{ffi::OsString, fmt, path::PathBuf};

/// A disk-backed DOM `File` received by a native module. Reading this argument
/// copies the native path only. Script-created files and plain objects are rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFile(pub PathBuf);

pub(super) const TOKEN: &str = "$skathi::HostFile";

pub(super) fn path_bytes(path: PathBuf) -> Vec<u8> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect()
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        path.into_os_string().into_vec()
    }
}

impl<'de> Deserialize<'de> for HostFile {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct DiskPath;
        impl Visitor<'_> for DiskPath {
            type Value = HostFile;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a File backed by a native path")
            }
            fn visit_byte_buf<E: de::Error>(self, bytes: Vec<u8>) -> Result<Self::Value, E> {
                #[cfg(windows)]
                let path = {
                    use std::os::windows::ffi::OsStringExt;
                    if bytes.len() % 2 != 0 {
                        return Err(E::custom("invalid native path"));
                    }
                    let wide: Vec<_> = bytes
                        .chunks_exact(2)
                        .map(|n| u16::from_le_bytes([n[0], n[1]]))
                        .collect();
                    OsString::from_wide(&wide)
                };
                #[cfg(unix)]
                let path = {
                    use std::os::unix::ffi::OsStringExt;
                    OsString::from_vec(bytes)
                };
                Ok(HostFile(PathBuf::from(path)))
            }
        }
        deserializer.deserialize_newtype_struct(TOKEN, DiskPath)
    }
}

#[cfg(feature = "types")]
impl specta::Type for HostFile {
    fn definition(_: &mut specta::Types) -> specta::datatype::DataType {
        specta_typescript::define("File").into()
    }
}

use std::borrow::Cow;

use ::image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};

use super::Error;

pub(super) enum ImageInput {
    Asset(String),
    Png(Vec<u8>),
}

pub(super) struct PreparedImage {
    pub(super) png: Vec<u8>,
}

pub(super) fn prepare(input: ImageInput) -> Result<PreparedImage, Error> {
    let bytes = match input {
        ImageInput::Asset(source) => {
            let key = sk_payload::content_key(&source)
                .ok_or_else(|| Error::invalid("Expected a packaged notification image path"))?;
            let bytes = crate::payload::lookup(&key).ok_or_else(|| {
                Error::invalid("Notification image is not in the app package. Pass PNG bytes for a development asset")
            })?;
            // A packaged raster is self-contained; resolving files linked from an SVG would escape the package.
            ::image::guess_format(bytes)
                .map_err(|_| Error::invalid("Expected a packaged raster image"))?;
            Cow::Borrowed(bytes)
        }
        ImageInput::Png(bytes) => {
            if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err(Error::invalid("Expected a PNG notification image"));
            }
            Cow::Owned(bytes)
        }
    };
    let bitmap = redjs::webapi::image::decode_thumbnail(&bytes, None, 512).ok_or_else(|| {
        Error::invalid(
            "Invalid notification image (2 MiB input, maximum raster dimensions 4096 by 4096)",
        )
    })?;
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &bitmap.rgba,
            bitmap.width,
            bitmap.height,
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| {
            Error::operation(format!("Could not encode notification image: {error}"))
        })?;
    Ok(PreparedImage { png })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_png_is_bounded_and_reencoded_without_distorting_its_aspect_ratio() {
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(
                &vec![255; 1024 * 64 * 4],
                1024,
                64,
                ExtendedColorType::Rgba8,
            )
            .unwrap();
        let prepared = prepare(ImageInput::Png(png)).unwrap();
        let bitmap = ::image::load_from_memory(&prepared.png).unwrap();
        assert_eq!((bitmap.width(), bitmap.height()), (512, 32));
        assert!(prepare(ImageInput::Png(b"\x89PNG\r\n\x1a\n".to_vec())).is_err());
        assert!(prepare(ImageInput::Asset("../outside.png".into())).is_err());
    }
}

#[cfg(any(target_os = "macos", windows))]
mod cache {
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};
    use uuid::Uuid;

    use super::{Error, PreparedImage};

    const MAX_FILES: usize = 256;
    const RETENTION: Duration = Duration::from_secs(35 * 24 * 3600);

    fn cache_root() -> io::Result<PathBuf> {
        #[cfg(windows)]
        if sk_permissions::package::has_identity() {
            let local = windows::Storage::ApplicationData::Current()
                .and_then(|app| app.LocalFolder())
                .and_then(|folder| folder.Path())
                .map_err(io::Error::other)?;
            return Ok(PathBuf::from(local.to_os_string()).join("cache"));
        }
        if let Some(profile) = crate::embed::data_dir() {
            return Ok(profile.join("cache"));
        }
        #[cfg(windows)]
        let parent = env::var_os("LOCALAPPDATA").map(PathBuf::from);
        #[cfg(target_os = "macos")]
        let parent = env::var_os("HOME").map(|p| PathBuf::from(p).join("Library").join("Caches"));
        let parent = parent
            .filter(|p| p.is_absolute())
            .ok_or_else(|| io::Error::other("no user cache directory"))?;
        Ok(parent.join(crate::payload::slot_name()).join("cache"))
    }

    fn directory() -> io::Result<PathBuf> {
        let path = cache_root()?.join(format!(
            "notifications-{}",
            super::super::native_id("image-cache-v1")
        ));
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path)?;
        if !fs::symlink_metadata(&path)?.is_dir() {
            return Err(io::Error::other(
                "invalid notification image cache directory",
            ));
        }
        Ok(path)
    }

    fn make_room(directory: &Path) -> io::Result<()> {
        let mut retained = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("png")
                || path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| Uuid::try_parse(s).ok())
                    .is_none()
                || !entry.file_type()?.is_file()
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if !modified.elapsed().is_ok_and(|age| age >= RETENTION)
                || fs::remove_file(&path).is_err()
            {
                retained.push((modified, path));
            }
        }
        if retained.len() >= MAX_FILES {
            retained.sort_unstable_by_key(|(modified, _)| *modified);
            for (_, path) in retained.iter().take(retained.len() - MAX_FILES + 1) {
                if let Err(error) = fs::remove_file(path) {
                    if error.kind() != io::ErrorKind::NotFound {
                        return Err(error);
                    }
                }
            }
        }
        Ok(())
    }

    impl PreparedImage {
        pub(in super::super) fn stage(&self) -> Result<PathBuf, Error> {
            let write = || -> io::Result<PathBuf> {
                let dir = directory()?;
                make_room(&dir)?;
                // The Apple notification service may move the original attachment.
                let path = dir.join(format!("{}.png", Uuid::new_v4()));
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(&path)?;
                if let Err(error) = file.write_all(&self.png) {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(error);
                }
                Ok(path)
            };
            write().map_err(|error| {
                Error::operation(format!("Could not stage notification image: {error}"))
            })
        }
    }
}

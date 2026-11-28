//! Run with `cargo run --manifest-path tools/windows-bindings/Cargo.toml`.
//! An optional directory supplies the two .winmd files for offline generation.

use sha2::{Digest, Sha256};
use std::{env, error::Error, fs, path::Path, process::Command};

const REVISION: &str = "d468916ac27a36fb8a12bafc1bf5c0ec2fe92238";
// These SDK/WDK metadata retain Win32 flag types and the published ABI's const annotations.
const METADATA: [(&str, &str); 2] = [
    (
        "Windows.Win32.winmd",
        "97d24cf1a9dc3e50782bbf1dba0952bf6a025fa583d8b3ae6c5ef713b463c869",
    ),
    (
        "Windows.Wdk.winmd",
        "54b8e48866b0a7e066981b63bb6f82fc6947fe8c57e2b5cb4513b1537aa836c2",
    ),
];

fn main() -> Result<(), Box<dyn Error>> {
    let directory = env::args_os().nth(1);
    let mut metadata = Vec::new();
    for (name, expected) in METADATA {
        let bytes = if let Some(directory) = &directory {
            fs::read(Path::new(directory).join(name))?
        } else {
            let url = format!("https://raw.githubusercontent.com/microsoft/windows-rs/{REVISION}/crates/libs/bindgen/default/{name}");
            let result = Command::new("curl")
                .args(["--fail", "--location", "--silent", "--show-error", &url])
                .output()?;
            if !result.status.success() {
                return Err(format!(
                    "downloading {name} failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                )
                .into());
            }
            result.stdout
        };
        if hex::encode(Sha256::digest(&bytes)) != expected {
            return Err(
                format!("unexpected SHA-256 for {name} at windows-rs revision {REVISION}").into(),
            );
        }
        metadata.push(bytes);
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/sk-windows");
    // Bindgen feeds rustfmt through stdin, so it discovers rustfmt.toml from its working directory.
    env::set_current_dir(&root)?;
    for name in ["raw", "win32", "runtime"] {
        let mut projection = windows_bindgen::builder();
        if matches!(name, "raw" | "win32") {
            projection.input_byte_sets(&metadata);
        }
        if name == "raw" {
            projection.sys();
        }
        projection
            .flat()
            .filter_file(root.join(format!("{name}.txt")))
            .output(root.join("src").join(format!("{name}.rs")))
            .write();
    }
    Ok(())
}

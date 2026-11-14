//! `--types`: the host's declarations in place of Bun's. The front end checks a file under the
//! nearest tsconfig.json and reads bun-types unless the project has installed `@types/bun`, so the
//! entry is checked in a folder of its own: a tsconfig.json whose root files are the given
//! declarations, with the ES libraries and without lib.dom, and an empty `@types/bun`.

use std::path::{Path, PathBuf};

fn json_string(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    format!("\"{}\"", text.replace('"', "\\\""))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().unwrap())
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    std::fs::write(path, text).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// Returns the folder to build in and the entry's name there.
pub fn stage(input: &str, ir: &str, types: &[String]) -> Result<(PathBuf, String), String> {
    let absolute =
        |path: &str| std::path::absolute(path).map_err(|error| format!("{path}: {error}"));
    let folder = absolute(&format!("{ir}.input"))?;
    let _ = std::fs::remove_dir_all(&folder);
    let input = absolute(input)?;
    let name = input
        .file_name()
        .ok_or("the entry has no file name")?
        .to_string_lossy()
        .into_owned();
    let source = std::fs::read(&input)
        .map_err(|error| format!("cannot read {}: {error}", input.display()))?;
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    std::fs::write(folder.join(&name), source)
        .map_err(|error| format!("cannot write the entry: {error}"))?;
    let mut files = vec![format!("\"{name}\"")];
    for declarations in types {
        let path = absolute(declarations)?;
        if !path.is_file() {
            return Err(format!("{declarations} is not a file"));
        }
        files.push(json_string(&path));
    }
    write(
        &folder.join("tsconfig.json"),
        &format!(
            "{{\"compilerOptions\":{{\"strict\":true,\"skipLibCheck\":true,\"types\":[],\"lib\":[\"ESNext\"]}},\"files\":[{}]}}\n",
            files.join(",")
        ),
    )?;
    let bun_types = folder.join("node_modules/@types/bun");
    write(
        &bun_types.join("package.json"),
        "{\"name\":\"@types/bun\",\"version\":\"0.0.0\",\"types\":\"index.d.ts\"}\n",
    )?;
    write(&bun_types.join("index.d.ts"), "export {};\n")?;
    Ok((folder, name))
}

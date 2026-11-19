extern crate bun_sema_standalone;

mod frontend;
mod probe;
mod types;
mod unlinked;
mod uses;

use std::path::Path;
use std::process::ExitCode;

const PIN_FRONTEND: &str = env!("SK_AOT_PIN_FRONTEND");
const PIN_ENGINE: &str = env!("SK_AOT_PIN_ENGINE");
const PIN_TABLE: &str = env!("SK_AOT_PIN_TABLE");

const USAGE: &str = "usage: sk-aot info
       sk-aot ir INPUT OUT.ir [--types FILE.d.ts]... [--define NAME=VALUE]... [--option WORD]...
       sk-aot image IN.ir OUT.image [--option WORD]...
       sk-aot compile INPUT OUT.ir OUT.image [--types FILE.d.ts]... [--define NAME=VALUE]... [--option WORD]...
       sk-aot run IMAGE [--prelude FILE.js]...";

fn unlinked(name: &str) -> ! {
    eprintln!("sk-aot: the front end called {name}, which this build does not provide");
    if std::env::var_os("SK_AOT_BACKTRACE").is_some() {
        eprintln!("{}", std::backtrace::Backtrace::force_capture());
    }
    std::process::abort()
}

/// The provenance an image's manifest records, refused when the linked engine or table is not the pinned one.
fn info() -> Result<String, String> {
    let linked = [
        ("frontend", redjs::aot::FRONTEND_REVISION, PIN_FRONTEND),
        ("engine", redjs::aot::ENGINE_VERSION, PIN_ENGINE),
        ("tableId", redjs::aot::BUILD_TABLE_ID, PIN_TABLE),
    ];
    for (name, actual, pinned) in linked {
        if actual != pinned {
            return Err(format!(
                "this sk-aot was built with {name} {actual}, the toolchain pin names {pinned}; rebuild it"
            ));
        }
    }
    Ok(format!(
        "{{\"target\":\"{}-{}\",\"frontend\":\"{PIN_FRONTEND}\",\"engine\":\"{PIN_ENGINE}\",\"tableId\":\"{PIN_TABLE}\",\"compileOptions\":\"{}\"}}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        redjs::aot::COMPILE_OPTIONS
    ))
}

fn front_end(input: &str, ir: &str, flags: &Flags) -> Result<(), String> {
    let Flags {
        types,
        options,
        defines,
        ..
    } = flags;
    // The front end's own engine run answers under the admitted code that redjs compiles the image with.
    let pinned = redjs::aot::COMPILE_OPTIONS
        .split(';')
        .filter(|word| word.starts_with("admit="))
        .map(str::to_owned);
    let option_words: &Vec<String> = &pinned.chain(options.iter().cloned()).collect();
    let (cwd, entry, ir_path) = match types.as_slice() {
        [] => (
            std::env::current_dir().map_err(|error| format!("no working directory: {error}"))?,
            input.to_owned(),
            ir.to_owned(),
        ),
        _ => {
            let (folder, entry) = types::stage(input, ir, types)?;
            let ir_path = std::path::absolute(ir).map_err(|error| format!("{ir}: {error}"))?;
            (folder, entry, ir_path.to_string_lossy().into_owned())
        }
    };
    let cwd = cwd.to_string_lossy().into_owned();
    let mut log = bun_ast::Log::init();
    let request = frontend::Request {
        working_directory: cwd.as_bytes(),
        entry_point: entry.as_bytes(),
        ir_path: ir_path.as_bytes(),
        option_words,
        defines,
    };
    let built = frontend::compile(&request, &mut log);
    if std::env::var_os("SK_AOT_TRACE").is_some() {
        eprintln!("sk-aot: front end returned {}", built.is_some());
    }
    let mut text = String::new();
    let _ = log.print(&mut text);
    eprint!("{text}");
    let Some(built) = built.filter(|_| Path::new(ir).is_file()) else {
        return Err(format!("the front end did not compile {input}"));
    };
    let path = format!("{ir}.uses.json");
    let engine = built.engine.0.get::<bun_aot::engine::EngineResult>();
    std::fs::write(&path, uses::json(engine))
        .map_err(|error| format!("cannot write {path}: {error}"))
}

/// What `aot-image compile` writes: the image, `.identity` and `.entries` beside it.
fn image(ir: &str, out: &str, options: &[String]) -> Result<(), String> {
    let program = std::fs::read(ir).map_err(|error| format!("cannot read {ir}: {error}"))?;
    let image = match options {
        // SAFETY: the IR comes from this binary's front end, of the pinned schema.
        [] => unsafe { redjs::aot::compile_ir(&program) }?,
        _ => probe::compile(&program, options)?,
    };
    let write = |path: String, bytes: &[u8]| {
        std::fs::write(&path, bytes).map_err(|error| format!("cannot write {path}: {error}"))
    };
    write(out.to_owned(), &image.bytes)?;
    write(format!("{out}.identity"), &image.identity)?;
    let entries: Vec<String> = image.entries.iter().map(u32::to_string).collect();
    write(format!("{out}.entries"), entries.join("\n").as_bytes())?;
    println!(
        "image_bytes={} entries={:?} diagnostics={}",
        image.bytes.len(),
        image.entries,
        image.diagnostics
    );
    Ok(())
}

#[derive(Default)]
struct Flags {
    types: Vec<String>,
    options: Vec<String>,
    preludes: Vec<String>,
    defines: Vec<String>,
}

fn split_flags(args: &[String]) -> Result<(Vec<&String>, Flags), String> {
    let mut positional = Vec::new();
    let mut flags = Flags::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let list = match arg.as_str() {
            "--types" => &mut flags.types,
            "--option" => &mut flags.options,
            "--prelude" => &mut flags.preludes,
            "--define" => &mut flags.defines,
            _ => {
                positional.push(arg);
                continue;
            }
        };
        list.push(rest.next().ok_or(format!("{arg} needs a value"))?.clone());
    }
    Ok((positional, flags))
}

fn run(args: &[String]) -> Result<(), String> {
    let info = info()?;
    let (positional, flags) = split_flags(args.get(1..).unwrap_or_default())?;
    let Flags {
        types,
        options,
        preludes,
        ..
    } = &flags;
    match (args.first().map(String::as_str), positional.as_slice()) {
        (Some("info"), []) => {
            println!("{info}");
            Ok(())
        }
        (Some("ir"), [input, ir]) => front_end(input, ir, &flags),
        (Some("image"), [ir, out]) if types.is_empty() => image(ir, out, options),
        (Some("compile"), [input, ir, out]) => {
            front_end(input, ir, &flags)?;
            image(ir, out, options)
        }
        (Some("run"), [image]) => probe::run(image, preludes),
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("sk-aot: {message}");
            ExitCode::FAILURE
        }
    }
}

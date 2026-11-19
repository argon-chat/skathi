use bun_alloc::Arena;
use bun_aot::engine::ExecutableIdentity;
use bun_ast::Log;
use bun_bundler::Transpiler;
use bun_bundler::aot::{AotBuild, AotBuildRequest, build};
use bun_bundler::options::{AllowUnresolved, CompileMode, CompileTargetBuiltins};
use bun_dotenv::Loader;
use bun_options_types::aot_options::AotOptions;
use bun_options_types::schema::api;
use bun_resolver::fs::{DirnameStore, FileSystem};
use bun_sema_driver::Libs as CheckerLibs;
use core::num::NonZeroU32;
use core::ptr::NonNull;

#[path = "../.cache/bun/src/runtime/cli/typescript_libs.rs"]
#[allow(dead_code)]
mod typescript_libs;

/// What `bun compile` names the folder of an executable's modules on Windows.
const MODULE_PREFIX: &[u8] = b"B:/~BUN/root/";

pub struct Request<'r> {
    pub working_directory: &'r [u8],
    pub entry_point: &'r [u8],
    pub ir_path: &'r [u8],
    pub option_words: &'r [String],
    pub include_builtins: bool,
    /// `NAME=VALUE`, after the platform's.
    pub defines: &'r [String],
}

/// A single-threaded IR build. Strict images must carry the library that ordinary hosts can load.
fn aot_options(ir_path: &[u8], option_words: &[String], include_builtins: bool) -> AotOptions {
    AotOptions {
        option_words: option_words
            .iter()
            .map(|word| Box::from(word.as_bytes()))
            .collect(),
        omits_builtin_modules: !include_builtins,
        ir_output_path: Some(Box::from(ir_path)),
        thread_count: Some(NonZeroU32::MIN),
        writes_no_executable: true,
        ..AotOptions::default()
    }
}

fn set_up_process(directory: &'static [u8]) -> bool {
    bun_core::output::stdio::init();
    bun_dotenv::set_instance(bun_core::heap::into_raw(Box::new(Loader::init())));
    FileSystem::init(Some(directory)).is_ok()
}

fn transform_options(
    working_directory: &[u8],
    entry_point: &[u8],
    defines: &[String],
) -> api::TransformOptions {
    let mut define = api::StringMap {
        keys: vec![
            Box::from(&b"process.platform"[..]),
            Box::from(&b"process.arch"[..]),
            Box::from(&b"process.versions.bun"[..]),
        ],
        values: vec![
            Box::from(&b"\"win32\""[..]),
            Box::from(&b"\"x64\""[..]),
            Box::from(
                const_format::concatcp!("\"", bun_core::Global::package_json_version, "\"")
                    .as_bytes(),
            ),
        ],
    };
    for pair in defines {
        let (name, value) = pair.split_once('=').unwrap_or((pair, "true"));
        define.keys.push(Box::from(name.as_bytes()));
        define.values.push(Box::from(value.as_bytes()));
    }
    api::TransformOptions {
        absolute_working_dir: Some(Box::from(working_directory)),
        target: Some(api::Target::Bun),
        entry_points: vec![Box::from(entry_point)],
        define: Some(define),
        ..Default::default()
    }
}

/// Runs the front end. `None`: the build failed, and `log` says why.
pub fn compile(request: &Request<'_>, log: &mut Log) -> Option<AotBuild> {
    let Ok(directory) = DirnameStore::instance().append_slice(request.working_directory) else {
        return None;
    };
    if !set_up_process(directory) {
        return None;
    }
    FileSystem::get().set_top_level_dir(directory);
    let arena: &'static Arena = Box::leak(Box::new(Arena::new()));
    let options = transform_options(
        request.working_directory,
        request.entry_point,
        request.defines,
    );
    let Ok(transpiler) = Transpiler::init(arena, &raw mut *log, options, None) else {
        return None;
    };
    let transpiler: &'static mut Transpiler<'static> = arena.alloc(transpiler);
    let opts = &mut transpiler.options;
    opts.compile_mode = CompileMode::Executable;
    opts.supports_multiple_outputs = false;
    opts.allow_unresolved = AllowUnresolved::All;
    opts.bytecode = false;
    opts.public_path = Box::from(MODULE_PREFIX);
    opts.compile_entry_point_name = Box::from(&b"unused.exe"[..]);
    opts.root_dir = Box::default();
    if transpiler.configure_defines().is_err() {
        return None;
    }
    transpiler.configure_linker();
    transpiler.options.compile_target_builtins = CompileTargetBuiltins::Host;
    if !transpiler.options.production
        && transpiler
            .options
            .conditions
            .append_slice(&[b"development" as &[u8]])
            .is_err()
    {
        return None;
    }
    transpiler.sync_resolver_opts();
    transpiler.resolver.env_loader = NonNull::new(transpiler.env);

    let aot = aot_options(
        request.ir_path,
        request.option_words,
        request.include_builtins,
    );
    let entry_points: [&[u8]; 1] = [request.entry_point];
    let built = build(AotBuildRequest {
        transpiler,
        entry_points: &entry_points,
        assets: &[],
        options: &aot,
        executable_identity: ExecutableIdentity([0; 16]),
        image_form: None,
        module_prefix: MODULE_PREFIX,
        outfile: None,
        replaced_file: None,
        log,
        arena,
        event_loop: None,
        worker_pool: None,
        plugins: None,
        file_map: None,
        typescript_libs: CheckerLibs::Bundled(typescript_libs::BUNDLED),
    });
    built.ok()
}

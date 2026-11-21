import { mkdtempSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { appendMachOImage } from "../../packages/cli/cli/image-macho";

if (process.platform !== "darwin" || process.arch !== "arm64") {
  throw new Error("the Mach-O image verification needs Apple Silicon macOS");
}

const identity = process.env.SK_MACHO_SIGNING_IDENTITY ?? "-";
const verifyHardened = process.argv.includes("--verify-hardened");
const hardened = identity !== "-" || verifyHardened || process.argv.includes("--hardened");
const signing = ["--force", "--sign", identity, "--timestamp=none"];
if (hardened) signing.push("--options", "runtime");

function run(command: string, ...args: string[]) {
  const process = Bun.spawnSync([command, ...args], {
    stdout: "inherit",
    stderr: "inherit",
  });
  if (process.exitCode !== 0) throw new Error(`${command} exited with ${process.exitCode}`);
}

function verifyRuntime(executable: string, library: string) {
  if (hardened) {
    const signature = Bun.spawnSync(["codesign", "-d", "--verbose=4", executable]);
    const details = signature.stderr.toString();
    if (signature.exitCode !== 0 || !/^CodeDirectory .*flags=0x[0-9a-f]+\([^\n]*\bruntime\b[^\n]*\)/im.test(details)) {
      throw new Error(`the test host signature is missing the hardened runtime flag\n${details}`);
    }
    if (verifyHardened && identity === "-" && !/^TeamIdentifier=not set$/m.test(details)) {
      throw new Error(`expected an ad hoc host signature without a Team ID\n${details}`);
    }
  }
  if (!verifyHardened || identity !== "-") {
    run(executable, library);
    return;
  }
  const probe = Bun.spawnSync([executable, library], { stdout: "inherit", stderr: "pipe" });
  if (probe.exitCode === 0) return;
  const details = probe.stderr.toString();
  if (
    probe.exitCode === 2 &&
    /mapped file has no Team ID and is not a platform binary|mapping process and mapped file \(non-platform\) have different Team IDs/i.test(
      details,
    )
  ) {
    console.log(`Ad hoc hardened probe rejected by macOS library validation: ${details.trim()}`);
    return;
  }
  throw new Error(`hardened probe exited with ${probe.exitCode}\n${details}`);
}

const contents = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.skathi.AotUnwindProbe</string>
<key>CFBundleName</key><string>AotUnwindProbe</string>
<key>CFBundleExecutable</key><string>probe</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
</dict></plist>`;

const source = `
#include <dlfcn.h>
#include <libunwind.h>
#include <mach-o/getsect.h>
#include <mach-o/loader.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

typedef int (*Callback)(void);
typedef int (*NativeFunction)(Callback);
static uintptr_t codeAddress;
int invoke(NativeFunction);

__attribute__((noinline))
static int checkUnwind(void) {
    unw_context_t context;
    unw_cursor_t cursor;
    bool sawImage = false;
    if (unw_getcontext(&context) || unw_init_local(&cursor, &context))
        return 1;
    while (unw_step(&cursor) > 0) {
        unw_word_t pc;
        if (unw_get_reg(&cursor, UNW_REG_IP, &pc)) return 2;
        if (!sawImage && pc >= codeAddress && pc < codeAddress + 20) {
            unw_proc_info_t info;
            if (unw_get_proc_info(&cursor, &info) || info.start_ip != codeAddress + 8)
                return 3;
            sawImage = true;
            continue;
        }
        if (sawImage) {
            Dl_info info;
            if (dladdr((void *)pc, &info) && info.dli_saddr == (void *)invoke)
                return 42;
            return 4;
        }
    }
    return 5;
}

__attribute__((noinline, disable_tail_calls, visibility("default")))
int invoke(NativeFunction function) {
    return function(checkUnwind);
}

int main(int argc, char **argv) {
    if (argc != 2) return 1;
    void *handle = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL | RTLD_FIRST);
    if (!handle) { fprintf(stderr, "%s\\n", dlerror()); return 2; }
    void *anchor = dlsym(handle, "skathi_image_anchor");
    Dl_info info;
    if (!anchor || !dladdr(anchor, &info)) return 3;
    const struct mach_header_64 *base = info.dli_fbase;
    unsigned long size = 0;
    const uint8_t *code = getsectiondata(base, "__SKCODE", "__aot_code", &size);
    if (!code || size != 16384) return 4;
    codeAddress = (uintptr_t)code;
    int result = invoke((NativeFunction)codeAddress);
    if (result != 42) {
        fprintf(stderr, "Mach-O image unwind failed at check %d\\n", result);
        return 5;
    }
    puts("Executed signed ARM64 image code and unwound through its compact unwind record");
    return 0;
}
`;

function image() {
  const page = 16384;
  const bytes = new Uint8Array(page * 3);
  const fields = new DataView(bytes.buffer);
  fields.setBigUint64(0, 0x3130544f414e5542n, true);
  for (const [at, value] of [
    [32, bytes.length],
    [40, page],
    [64, page],
    [72, 5],
    [120, 1024],
    [124, 8],
    [328, 1],
    [page, 0xa9bf7bfd],
    [page + 4, 0x910003fd],
    [page + 8, 0xd63f0000],
    [page + 12, 0xa8c17bfd],
    [page + 16, 0xd65f03c0],
  ] as const) {
    fields.setUint32(at, value, true);
  }
  fields.setUint8(93, 1);
  return bytes;
}

const work = mkdtempSync(join(tmpdir(), "sk-macho-verify-"));
try {
  const app = join(work, "AotUnwindProbe.app");
  const directory = join(app, "Contents");
  mkdirSync(join(directory, "MacOS"), { recursive: true });
  mkdirSync(join(directory, "Frameworks"));
  const executable = join(directory, "MacOS", "probe");
  const library = join(directory, "Frameworks", "app.image.dylib");
  const template = process.env.SK_MACHO_TEMPLATE
    ? resolve(process.env.SK_MACHO_TEMPLATE)
    : join(work, "libskimage.dylib");
  await Bun.write(join(directory, "Info.plist"), contents);
  if (!process.env.SK_MACHO_TEMPLATE) {
    const anchor = join(work, "anchor.c");
    await Bun.write(anchor, "const unsigned char skathi_image_anchor = 0;\n");
    run(
      "clang",
      "-dynamiclib",
      "-nostdlib",
      "-lSystem",
      "-Wl,-headerpad,0x800",
      "-Wl,-install_name,@rpath/libskimage.dylib",
      "-Wl,-no_fixup_chains",
      "-Wl,-no_adhoc_codesign",
      anchor,
      "-o",
      template,
    );
  }
  const compiled = appendMachOImage(new Uint8Array(await Bun.file(template).arrayBuffer()), image());
  await Bun.write(library, compiled);
  const probe = join(work, "probe.c");
  await Bun.write(probe, source);
  run("clang", "-O1", "-fno-omit-frame-pointer", "-Wl,-export_dynamic", probe, "-o", executable);
  run("codesign", ...signing, library);
  if (identity === "-") {
    const signed = readFileSync(library);
    await Bun.write(library, compiled);
    run("codesign", ...signing, library);
    if (!signed.equals(readFileSync(library))) throw new Error("the ad hoc-signed image dylib was not reproducible");
  }
  run("codesign", ...signing, app);
  run("codesign", "--verify", "--deep", "--strict", app);
  verifyRuntime(executable, library);
  if (process.argv.includes("--redjs")) {
    process.env.SK_MACHO_TEST_CONTAINER = library;
    run(
      "cargo",
      "test",
      "--manifest-path",
      "vendor/redjs/redjs/Cargo.toml",
      "--target",
      "aarch64-apple-darwin",
      "--features",
      "native-aot",
      "--lib",
      "signed_macho_image_is_mapped_in_place",
      "--",
      "--ignored",
    );
  }
} finally {
  rmSync(work, { recursive: true, force: true });
}

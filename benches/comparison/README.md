# Windows desktop comparison

This is the fixture for the [Skathi, Electron and Tauri comparison](https://skathi.land/comparison/).
Each app opens a 760 × 520 window with a Vue counter and 1,000 rows, signals its second
animation frame through its window title and scrolls the list after 20 seconds.

## Build

Use Windows x64, PowerShell 7, Python 3.9+, Node and npm, Bun, and the SDK's Rust/MSVC
toolchain. Initialize the SDK submodules. From the **Skathi repository root**:

```powershell
bun install --frozen-lockfile
pwsh -File vendor/redjs/setup.ps1
bun tools/sk-aot/setup.ts
python benches/comparison/build.py
```

For the three-framework comparison alone, build `jit electron tauri`. The optional
`standalone`, `native`, `full`, `aot` and `trimmed` groups prepare the other suites. Both AOT builds need
the compiler setup step. An existing Electron ZIP can be used with
`--electron-zip-dir <directory>`. The builds use the SDK sources in this checkout and
the fixture lockfiles. The packaged apps go to `target/comparison/apps`.

## Measure

Run from an interactive Windows desktop with a hardware GPU and English performance
counter names. Finish building first. Keep the power plan, display configuration and
foreground workload the same across a series.

PowerShell loads `tools/Measure.csx` once before launching the apps. The median and
size reports use the .NET 10 SDK and `dotnet-script` (install once with
`dotnet tool install --global dotnet-script --version 2.0.0`).

```powershell
$measure = 'benches/comparison/tools/measure.ps1'
& $measure -Suite comparison -Runs 5 -Out target/comparison/running.jsonl
& $measure -Suite profiles -Runs 3 -Out target/comparison/profiles.jsonl
& $measure -Suite memory -Runs 3 -Out target/comparison/memory.jsonl
& $measure -Suite native -Runs 5 -Out target/comparison/native.jsonl
bun benches/comparison/power.ts --runs 3 --out target/comparison/power.jsonl
$medians = 'benches/comparison/tools/medians.csx'
dotnet script $medians -- target/comparison/running.jsonl --runs 5
dotnet script $medians -- target/comparison/profiles.jsonl --runs 3
dotnet script $medians -- target/comparison/memory.jsonl --runs 3
dotnet script $medians -- target/comparison/native.jsonl --runs 5
dotnet script $medians -- target/comparison/power.jsonl --runs 3
dotnet script benches/comparison/tools/sizes.csx
```

Each output path must be new. Omitting `-Out` creates a timestamped JSONL file.
The runner cycles through all apps each round. The first launch warms each app for
the window and profile suites. It polls the launched PID for the ready title, samples
memory at five seconds and measures CPU over the next ten seconds. The comparison
suite repeats both samples after scrolling. CPU is a percentage of one logical core.
Private working set, total working set and private commit are separate process-tree
sums in MiB. A missing GPU counter is recorded as null. Before launch the machine
must be below 10% CPU. The runner records average system CPU during the sample
and repeats it at 20% or above.

The profile suite measures idle JIT with the default allocator, low memory, and idle
frame pacing disabled. The AOT cases run one native image with engine-default or
compact capacities and with compact capacities plus low memory. The trimmed host
also removes the JIT compiler and WebAssembly from its engine archive. The full JIT host
uses the SDK's default Cargo features. All apps use the current OS power plan.

The memory control runs the SDK's standalone `sk-app` binary with the same minimal
features and JIT payload, supplied through `SK_PAYLOAD`. It compares default and
low memory through the standard executable entry point. In revision `2a884cbe`,
only this entry point calls `memory::apply()`. In the embedded profile suite the
low-memory rows record the requested setting.

The power suite opens the full JIT host with devtools, waits for scrolling to finish
and applies one animated noise shader to the list. It sets the host's power-scheme
and CPU-pressure inputs, checks the quality slot in the painted frame, and records
frame counts and process CPU over ten seconds per tier. It rotates the order of
the tiers each round. The readings describe rendering cadence and CPU use.

The native-call suite checks the returned sums for 10,000 additions and 50 transfers
of 1 MiB. Skathi's fixture includes separate copied and borrowed byte arguments, and
direct native API calls. The Electron and Tauri fixtures await each IPC call.

The size script counts deployable files, zips them with .NET's `CompressionLevel.Optimal`, and measures
the installed Evergreen WebView2 directory and the Tauri NSIS installer. Supply
`--fixed <unpacked-runtime> --cab <archive>` or `--installer <standalone-x64-exe>`
to include the corresponding WebView2 distribution in the output. Use the versioned
directory that contains `msedgewebview2.exe` after extracting the fixed CAB.
[Microsoft WebView2 downloads](https://developer.microsoft.com/en-us/microsoft-edge/webview2/#download-section),
[Evergreen standalone installer for x64](https://go.microsoft.com/fwlink/p/?LinkId=2124701).

## Recorded runs

- [November 17, 2026](results/2026-11-17/) contains the original samples for the
  0.4 comparison published in commit `437368c4`. Its full-feature host had devtools
  enabled in the payload.
- [November 22, 2026](results/2026-11-22/) contains the 0.7 results at
  `2a884cbe`, including the machine configuration, allocator controls and power
  samples. Its full-feature host embeds the same application payload as the minimal
  JIT host.

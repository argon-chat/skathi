//! `cargo bench -p sk-payload --bench bank -- [app.skpk ...]`
//! or pass `--entry <entry-name> <original-file> <bank.hb>...` for a decoded range comparison.
use sk_payload::{bank::crc64, kind, Entry, Payload, SkBank};
use std::ffi::OsString;
use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

const MIB: usize = 1024 * 1024;
const SAMPLES: usize = 7;
const LOOKUPS: usize = 4096;

fn median(mut operation: impl FnMut()) -> f64 {
    let batch = |count, operation: &mut dyn FnMut()| {
        let start = Instant::now();
        for _ in 0..count {
            operation();
        }
        start.elapsed()
    };
    let mut iterations = 1;
    while batch(iterations, &mut operation) < Duration::from_millis(20) {
        iterations *= 2;
    }
    let mut samples = [0.0; SAMPLES];
    for sample in &mut samples {
        *sample = batch(iterations, &mut operation).as_secs_f64() / iterations as f64;
    }
    samples.sort_by(f64::total_cmp);
    samples[SAMPLES / 2]
}

fn measure(label: &str, bytes: usize, operation: impl FnMut()) {
    let seconds = median(operation);
    print!("{label:24} {:11.4} ms", seconds * 1000.);
    if bytes > 0 {
        print!(" {:10.1} MiB/s", bytes as f64 / MIB as f64 / seconds);
    }
    println!();
}

fn synthetic(count: usize, file_size: usize) {
    let size = count * file_size;
    let mut names = Vec::with_capacity(count);
    let mut input = vec![0; size];
    for (index, data) in input.chunks_exact_mut(file_size).enumerate() {
        let id = (index as u32).wrapping_mul(2_654_435_761) & (count as u32 - 1);
        names.push(format!("assets/item-{id:06}.bin"));
        let mut state = id + 1;
        for byte in data {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *byte = (state >> 24) as u8;
        }
    }
    let entries: Vec<_> = names
        .iter()
        .zip(input.chunks_exact(file_size))
        .map(|(name, data)| Entry {
            kind: kind::ASSET,
            name,
            data,
        })
        .collect();
    run(&format!("{count} × {file_size} bytes"), &entries);
}

fn run(name: &str, entries: &[Entry<'_>]) {
    let size = entries.iter().map(|entry| entry.data.len()).sum();
    let bytes = Payload::write_bank(entries, 0).unwrap();
    let bank = SkBank::parse(&bytes).unwrap();
    let payload = Payload::parse(&bytes).unwrap();
    assert_eq!(payload.entries().len(), entries.len());
    let mut sorted = entries.to_vec();
    sorted.sort_unstable_by_key(|entry| (entry.kind, entry.name));
    for (input, decoded) in sorted.iter().zip(payload.entries()) {
        assert_eq!(
            (input.kind, input.name, input.data),
            (decoded.kind, decoded.name, decoded.data)
        );
    }
    let mut output = vec![0; size];

    println!(
        "\n{name}: {} entries, {size} data bytes, {} serialized bytes",
        entries.len(),
        bytes.len()
    );
    measure("Payload::write_bank", size, || {
        black_box(Payload::write_bank(black_box(entries), 0).unwrap());
    });
    measure("SkBank::parse", 0, || {
        black_box(SkBank::parse(black_box(&bytes)).unwrap());
    });
    measure("Payload::parse", 0, || {
        black_box(Payload::parse(black_box(&bytes)).unwrap());
    });
    if !payload.assets().is_empty() {
        let queries: Vec<_> = (0..LOOKUPS)
            .map(|i| payload.assets()[i * 2531 % payload.asset_count()].name)
            .collect();
        let lookup_seconds = median(|| {
            for name in black_box(&queries) {
                black_box(payload.asset(name).unwrap());
            }
        });
        println!(
            "{:24} {:11.1} ns/lookup",
            "Payload::asset",
            lookup_seconds * 1e9 / LOOKUPS as f64
        );
    }
    measure("copy all Store entries", size, || {
        let mut cursor = 0;
        for entry in payload.entries() {
            let end = cursor + entry.data.len();
            output[cursor..end].copy_from_slice(entry.data);
            cursor = end;
        }
        black_box(&output);
    });
    measure("verify all Store entries", size, || {
        for entry in black_box(bank.entries()) {
            entry.verify(size).unwrap();
        }
    });
    measure("CRC64 of input", size, || {
        for entry in black_box(entries) {
            black_box(crc64(entry.data));
        }
    });
}

fn read_entry(name: &str, original: &[u8], path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    let bank = SkBank::parse(&bytes).unwrap();
    let file = bank.entry(name).expect("entry missing from bank");
    assert_eq!(file.read(original.len()).unwrap().as_ref(), original);
    file.verify(original.len()).unwrap();

    println!(
        "\n{}: {name}, {:?}, chunked={}, {} bank bytes",
        path.display(),
        file.codec(),
        file.is_chunked(),
        bytes.len()
    );
    measure("decode full entry", original.len(), || {
        black_box(file.read(original.len()).unwrap());
    });
    const BLOCK: usize = 64 * 1024;
    let start = original.len().saturating_sub(BLOCK) / BLOCK * BLOCK;
    for len in [4096, BLOCK] {
        let range = start..(start + len).min(original.len());
        assert_eq!(
            file.read_range(range.clone()).unwrap().as_ref(),
            &original[range.clone()]
        );
        measure(
            &format!("read {} at {start}", range.len()),
            range.len(),
            || {
                black_box(file.read_range(range.clone()).unwrap());
            },
        );
    }
}

fn main() {
    let args: Vec<OsString> = std::env::args_os()
        .skip(1)
        .filter(|arg| arg != "--bench")
        .collect();
    if args.first().is_some_and(|arg| arg == "--entry") {
        assert!(
            args.len() >= 4,
            "usage: --entry <entry-name> <original-file> <bank.hb>..."
        );
        let name = args[1].to_str().expect("entry name must be UTF-8");
        let original = std::fs::read(&args[2]).unwrap();
        for path in &args[3..] {
            read_entry(name, &original, Path::new(path));
        }
        return;
    }
    println!(
        "In-memory Store banks, median of {SAMPLES} warmed samples (at least 20 ms per batch)."
    );
    synthetic(4096, 1024);
    synthetic(4, 8 * MIB);
    for path in args {
        let bytes = std::fs::read(&path).unwrap();
        let payload = Payload::parse(&bytes).unwrap();
        run(&Path::new(&path).display().to_string(), payload.entries());
        if bytes.starts_with(sk_payload::MAGIC) {
            measure("source SKPK v1 parse", 0, || {
                black_box(Payload::parse(black_box(&bytes)).unwrap());
            });
        }
    }
}

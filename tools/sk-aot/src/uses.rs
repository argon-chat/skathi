//! `<ir>.uses.json`: what the front end's engine run found the program to use. `sk build` reads it
//! to refuse a workload whose engine leaves out a feature the image needs.
//!
//! - `features`: each removable engine feature the program uses (`id` as in AOTBuiltinFeatures.h),
//!   how (`Named`, `Unnamed`, `NamedAtRunTime`, `FoundByName`), through which built-in, and where
//!   (`module`, `position`).
//! - `unseen`: code the image does not hold and why (`EvaluatesText` for eval and `new Function`).
//! - `entryModules`: the engine's ids of the entry modules, which `module` refers to.
//! - `statistics`: the engine's nonzero counters.

use bun_aot::builtins::table::text_of;
use bun_aot::engine::{EngineResult, builtin_of, feature_of};
use bun_aot::statistics::StatisticId;
use std::collections::BTreeSet;
use std::fmt::Write;

fn builtin(word: u32) -> String {
    builtin_of(word).map_or_else(|| "null".into(), |id| quote(&text_of(id)))
}

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn json(engine: &EngineResult) -> String {
    let mut features = BTreeSet::new();
    for record in engine.features_used() {
        let name = feature_of(record.feature).map_or("unknown", |feature| feature.name());
        features.insert(format!(
            "{{\"feature\":{},\"id\":{},\"kind\":{},\"builtin\":{},\"module\":{},\"position\":{}}}",
            quote(name),
            record.feature,
            quote(record.kind.name()),
            builtin(record.builtin),
            record.witness.location.module,
            record.witness.location.position
        ));
    }
    let mut unseen = BTreeSet::new();
    for record in engine.unseen_code() {
        unseen.insert(format!(
            "{{\"cause\":{},\"builtin\":{},\"module\":{},\"position\":{}}}",
            quote(record.cause.name()),
            builtin(record.builtin),
            record.witness.location.module,
            record.witness.location.position
        ));
    }
    let entries: Vec<String> = engine
        .entry_module_ids()
        .iter()
        .map(|id| id.0.to_string())
        .collect();
    let statistics: Vec<String> = engine
        .statistics()
        .iter()
        .enumerate()
        .filter(|(_, value)| **value != 0)
        .filter_map(|(index, value)| {
            let id = StatisticId::from_wire(u32::try_from(index).ok()?)?;
            Some(format!("{}:{value}", quote(id.name())))
        })
        .collect();
    let list = |items: Vec<String>| items.join(",\n    ");
    format!(
        "{{\n  \"schema\": \"skathi.aot-uses\",\n  \"version\": 1,\n  \"entryModules\": [{}],\n  \"features\": [\n    {}\n  ],\n  \"unseen\": [\n    {}\n  ],\n  \"statistics\": {{\n    {}\n  }}\n}}\n",
        entries.join(","),
        list(features.into_iter().collect()),
        list(unseen.into_iter().collect()),
        list(statistics)
    )
}

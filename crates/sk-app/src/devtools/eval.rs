//! Privileged devtools source evaluation, through the embedding API.
//! The JS formatter receives values; it never calls eval or Function.
use std::{cell::Cell, ffi::c_void};
use sk_runtime::redjs::{
    api::{Context, Error, Value},
    ffi,
};

const FORMATTER: &str = r#"
(function (value, thrown) {
  try {
    if (thrown) throw value;
    var json;
    try { json = JSON.stringify(value); } catch (_) { json = undefined; }
    if (json === undefined) json = JSON.stringify(value === undefined ? null : String(value));
    return '{"value":' + json + ',"type":' + JSON.stringify(typeof value) + '}';
  } catch (error) {
    return JSON.stringify({
      exception: (error && error.name ? error.name + ': ' + (error.message || '') : String(error)),
      stack: (error && error.stack) ? String(error.stack) : ''
    });
  }
})
"#;
#[derive(Clone, Copy)]
struct Formatter {
    context: *mut c_void,
    context_id: u64,
    function: Value,
}

thread_local! {
    // A single formatter for this thread's current VM. No global JS name can
    // be overwritten by the source being evaluated. The context id fences reuse.
    static FORMATTER_CACHE: Cell<Option<Formatter>> = const { Cell::new(None) };
}

struct Rooted<'a> {
    context: &'a Context,
    value: Value,
}
impl<'a> Rooted<'a> {
    fn new(context: &'a Context, value: Value) -> Self {
        context.protect(value);
        Self { context, value }
    }
}
impl Drop for Rooted<'_> {
    fn drop(&mut self) {
        self.context.unprotect(self.value);
    }
}

fn error_text(context: &Context, error: Error) -> String {
    match error {
        Error::Exception(value) => {
            let rooted = Rooted::new(context, value);
            context
                .to_rust_string(rooted.value)
                .unwrap_or_else(|_| "JS exception while formatting eval result".into())
        }
        error => error.to_string(),
    }
}

fn formatter(context: &Context) -> Result<Value, String> {
    let context_id = unsafe { ffi::redjs_context_id(context.raw()) };
    if context_id == 0 {
        return Err("devtools eval requires a registered runtime context".into());
    }
    if let Some(cached) = FORMATTER_CACHE.with(Cell::get) {
        if cached.context_id == context_id && cached.context == context.raw() {
            return Ok(cached.function);
        }
    }
    let function = context
        .eval_named(FORMATTER, "sk://devtools-eval-formatter.js")
        .map_err(|error| error_text(context, error))?;
    context.protect(function);
    let old = FORMATTER_CACHE.with(|cache| {
        cache.replace(Some(Formatter {
            context: context.raw(),
            context_id,
            function,
        }))
    });
    if let Some(old) = old {
        // A released/recycled context's heap is gone: forget its stale word,
        // just as the runtime's context registries do, without dereferencing it.
        if unsafe { ffi::redjs_context_id(old.context) } == old.context_id {
            unsafe { ffi::redjs_unprotect(old.context, old.function.0) };
        }
    }
    Ok(function)
}

pub(super) fn evaluate(context: &Context, source: &str) -> Result<String, String> {
    let formatter = formatter(context)?;
    let (value, thrown) = match context.eval_named(source, "sk://devtools-eval.js") {
        Ok(value) => (value, false),
        Err(Error::Exception(value)) => (value, true),
        Err(error) => return Err(error_text(context, error)),
    };
    // Both successful completion values and thrown objects are initially raw
    // JS values. Keep them rooted across call-site profiling, helper invocation
    // and any toJSON/getter that runs (and may itself trigger a collection).
    let value = Rooted::new(context, value);
    let formatted = context
        .call(
            formatter,
            Value::undefined(),
            &[value.value, Value::boolean(thrown)],
        )
        .map_err(|error| error_text(context, error))?;
    let formatted = Rooted::new(context, formatted);
    context
        .to_rust_string(formatted.value)
        .map_err(|error| error_text(context, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sk_runtime::JsRuntime;

    struct RestoreEval<'a>(Rooted<'a>);
    impl Drop for RestoreEval<'_> {
        fn drop(&mut self) {
            let _ = self.0.context.global_set("eval", self.0.value);
        }
    }
    fn run(context: &Context, source: &str) -> serde_json::Value {
        serde_json::from_str(
            &evaluate(context, source).expect("host evaluation/serialization failed"),
        )
        .expect("invalid protocol JSON")
    }

    #[test]
    fn completion_values_preserve_the_eval_wire_shape() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                assert_eq!(
                    run(context, "20 + 22"),
                    serde_json::json!({"value":42,"type":"number"})
                );
                assert_eq!(
                    run(context, "undefined"),
                    serde_json::json!({"value":null,"type":"undefined"})
                );
                assert_eq!(
                    run(context, "null"),
                    serde_json::json!({"value":null,"type":"object"})
                );
                assert_eq!(
                    run(context, "123n"),
                    serde_json::json!({"value":"123","type":"bigint"})
                );
                assert_eq!(
                    run(
                        context,
                        "(() => { const cycle = {}; cycle.self = cycle; return cycle; })()"
                    ),
                    serde_json::json!({"value":"[object Object]","type":"object"})
                );
                assert_eq!(
                    run(
                        context,
                        "var __sk_eval_statement_probe = 41; ++__sk_eval_statement_probe"
                    ),
                    serde_json::json!({"value":42,"type":"number"})
                );
            })
        });
    }

    #[test]
    fn thrown_values_and_syntax_errors_keep_exception_and_stack_fields() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                let error = run(context, "throw new TypeError('probe')");
                assert_eq!(error["exception"], "TypeError: probe");
                assert!(error["stack"].as_str().is_some());
                let syntax = run(context, "const = invalid");
                assert!(syntax["exception"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("SyntaxError:")));
                assert_eq!(
                    run(context, "throw null"),
                    serde_json::json!({"exception":"null","stack":""})
                );
            })
        });
    }

    #[test]
    fn formatting_keeps_values_and_cached_function_rooted_through_gc() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                let collect = context
                    .function("__sk_eval_collect", 0, |global, _frame| unsafe {
                        ffi::redjs_gc(global, 0);
                        Value::undefined().0
                    })
                    .expect("cannot create collection fixture");
                context
                    .global_set("__sk_eval_collect", collect)
                    .expect("cannot publish collection fixture");
                let value = run(context, "__sk_eval_collect(); ({ toJSON() { __sk_eval_collect(); return { answer: 42 }; } })");
                context
                    .global_set("__sk_eval_collect", Value::undefined())
                    .expect("cannot clear collection fixture");
                assert_eq!(
                    value,
                    serde_json::json!({"value":{"answer":42},"type":"object"})
                );
                let failed_fallback = run(context, "({ toJSON() { throw new Error('stringify'); }, toString() { throw new TypeError('fallback'); } })");
                assert_eq!(failed_fallback["exception"], "TypeError: fallback");
            })
        });
    }
    #[test]
    fn host_evaluation_does_not_depend_on_the_js_indirect_eval_function() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                let original = context.global_get("eval").expect("eval missing");
                let _restore = RestoreEval(Rooted::new(context, original));
                context.eval_named("globalThis.eval = function () { throw new EvalError('disabled eval probe'); };", "eval-test-setup").expect("cannot replace JIT eval");
                let actual = run(context, "({ answer: 42 })");

                assert_eq!(
                    actual,
                    serde_json::json!({"value":{"answer":42},"type":"object"})
                );
            })
        });
    }
}

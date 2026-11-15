use super::*;

const BASE: &str = r#"const base = { delay: 0, endDelay: 0, fill: 'auto', iterationStart: 0,
    iterations: 1, duration: 'auto', direction: 'normal', easing: 'linear' };
const c = __sk_waapi_core;
function check(ok, message) { if (!ok) throw new Error(message || 'native WAAPI assertion failed'); }
function same(actual, expected) { check(JSON.stringify(actual) === JSON.stringify(expected),
    JSON.stringify(actual) + ' != ' + JSON.stringify(expected)); }
"#;

fn evaluate(body: &str) {
    let source = format!("(() => {{ {BASE}\n{body}\nreturn 'passed'; }})()");
    JsRuntime::with(|runtime| {
        super::install(runtime);
        match runtime.ctx().eval(&source) {
            Ok(value) => assert_eq!(runtime.ctx().read_string(value).unwrap(), "passed"),
            Err(Error::Exception(value)) => {
                let (message, stack) = runtime.ctx().error_info(value);
                panic!("{message}\n{stack}\n{source}");
            }
            Err(error) => panic!("{error}: {source}"),
        }
    });
}

#[test]
fn keyframe_offsets_merge_and_empty_inputs() {
    sk_runtime::testing::on_js_thread(|| {
        evaluate(
            r#"
        same(c.keyframes(null, 'test'), []);
        same(c.keyframes({opacity: []}, 'test'), []);
        const list = c.keyframes([{opacity: 0}, {opacity: .3, offset: .4}, {}, {opacity: 1}], 'test');
        same(list.map(f => f.computed), [0, .4, .7, 1]);
        same(list.map(f => f.offset), [null, .4, null, null]);
        same(list[0].props, [['opacity', '0']]);
        same(c.keyframes([{opacity: 1}], 'test').map(f => f.computed), [1]);
        const merged = c.keyframes({opacity: [0, 1], transform: ['scale(1)', 'scale(2)', 'scale(3)']}, 'test');
        same(merged.map(f => f.computed), [0, .5, 1]);
        same(merged[0].props, [['opacity', '0'], ['transform', 'scale(1)']]);
        same(merged[1].props, [['transform', 'scale(2)']]);
        same(c.keyframes({opacity: 1}, 'test')[0].props, [['opacity', '1']]);
        let rejected = false;
        try { c.keyframes([{offset: .8}, {offset: .2}], 'test'); } catch (e) { rejected = e instanceof TypeError; }
        check(rejected, 'decreasing offsets must fail');
    "#,
        )
    });
}

#[test]
fn conversions_use_js_number_and_string_without_losing_utf16() {
    sk_runtime::testing::on_js_thread(|| {
        evaluate(
            r#"
        const t = c.timing(base, {duration: 12n, iterations: 2n}, 'test');
        same([t.duration, t.iterations], [12, 2]);
        const frames = c.keyframes([{opacity: Symbol('x'), ['\ud800']: '\udfff'}], 'test');
        same(frames[0].props[0], ['opacity', 'Symbol(x)']);
        check(frames[0].props[1][0].charCodeAt(0) === 0xd800, 'key UTF-16 preserved');
        check(frames[0].props[1][1].charCodeAt(0) === 0xdfff, 'value UTF-16 preserved');
        const sentinel = {};
        try { c.timing(base, {duration: {valueOf() {throw sentinel;}}}, 'test'); check(false); }
        catch (error) { check(error === sentinel, 'coercion exception identity'); }
        let rejected = false;
        try { c.timing(base, {duration: Symbol('x')}, 'test'); } catch (error) { rejected = error instanceof TypeError; }
        check(rejected, 'Number(Symbol) throws');
        check(c.timing(base, {easing: '\ufeff linear \u00a0'}, 'test').easing === 'linear', 'JS whitespace trim');
    "#,
        )
    });
}

#[test]
fn timing_getter_order_and_object_spread_are_observable() {
    sk_runtime::testing::on_js_thread(|| {
        evaluate(
            r#"
        const log = [];
        const input = {
            get delay() { log.push('delay'); return {valueOf() {log.push('number'); return 3;}}; },
            get duration() { log.push('duration'); return 5; }
        };
        const result = c.timing(base, input, 'test');
        same(log, ['delay', 'delay', 'number', 'duration', 'duration', 'duration']);
        same([result.delay, result.duration], [3, 5]);
        const symbol = Symbol();
        const copied = c.timing({...base, [symbol]: 23, ['__proto__']: 'own'}, null, 'test');
        check(copied[symbol] === 23 && Object.hasOwn(copied, '__proto__') && copied.__proto__ === 'own');
        let count = 0;
        const changing = c.timing(base, {get duration() {return ++count < 3 ? 'anything' : 'auto';}}, 'test');
        check(changing.duration === 'auto' && count === 3);
    "#,
        )
    });
}

#[test]
fn frame_property_getters_run_before_metadata_and_next_array_index() {
    sk_runtime::testing::on_js_thread(|| {
        evaluate(
            r#"
        const log = [];
        c.keyframes([{
            get opacity() {log.push('property'); return {toString() {log.push('string'); return '1';}};},
            get offset() {log.push('offset'); return 1;},
            get easing() {log.push('easing'); return 'linear';},
            get composite() {log.push('composite'); return 'auto';}
        }], 'test');
        same(log, ['property', 'string', 'offset', 'easing', 'composite']);
        const values = [{toString() {values[1] = 'changed'; return 'first';}}, 'old'];
        const frames = c.keyframes({opacity: values}, 'test');
        same(frames.map(f => f.props[0][1]), ['first', 'changed']);
        const sparse = []; sparse.length = 3; sparse[1] = .5;
        const holes = c.keyframes({opacity: sparse}, 'test');
        same(holes.map(f => f.computed), [.5]);
        const inherited = Object.create(Array.prototype); inherited[0] = 'inherited';
        const withInherited = [, 'own']; Object.setPrototypeOf(withInherited, inherited);
        same(c.keyframes({opacity: withInherited}, 'test').map(f => f.props[0][1]), ['inherited', 'own']);
        const enormous = new Proxy([], {get(target, key) {return key === 'length' ? 2 ** 32 : target[key];}});
        let rangeError = false;
        try {c.keyframes({opacity: enormous}, 'test');} catch (error) {rangeError = error instanceof RangeError;}
        check(rangeError, 'mapped array length must be rejected before indexed reads');
        const sparseMetadata = []; sparseMetadata.length = 2; sparseMetadata[1] = 'ease';
        same(c.keyframes({opacity: [0, .5, 1], easing: sparseMetadata}, 'test').map(f => f.easing), [undefined, 'ease', undefined]);
    "#,
        )
    });
}

#[test]
fn iterators_close_on_body_errors_and_preserve_original_exception() {
    sk_runtime::testing::on_js_thread(|| {
        evaluate(
            r#"
        let methodReads = 0, nextReads = 0, closed = 0;
        const sentinel = {};
        const iterator = {
            get next() {nextReads++; return () => ({done: false, value: {get opacity() {throw sentinel;}}});},
            return() {closed++; throw new Error('must not replace original');}
        };
        const source = {get [Symbol.iterator]() {methodReads++; return () => iterator;}};
        try {c.keyframes(source, 'test'); check(false);} catch (error) {check(error === sentinel);}
        same([methodReads, nextReads, closed], [2, 1, 1]);
        closed = 0;
        const abrupt = {[Symbol.iterator]() {return {
            next() {throw sentinel;}, return() {closed++; return {};}
        };}};
        try {c.keyframes(abrupt, 'test'); check(false);} catch (error) {check(error === sentinel);}
        check(closed === 0, 'next failure is not a loop-body abrupt completion');
        let index = 0;
        const ok = {[Symbol.iterator]() {return {next() {return index++ ? {done: true} : {done: false, value: null};}};}};
        check(c.keyframes(ok, 'test').length === 1);
    "#,
        )
    });
}

host_fn!(fn test_gc(_g, _args) {
    JsRuntime::with(|runtime| runtime.ctx().gc());
    Value::undefined().0
});

#[test]
fn nested_same_method_and_gc_during_getters_keep_values_rooted() {
    sk_runtime::testing::on_js_thread(|| {
        JsRuntime::with(|runtime| {
            let gc = unsafe {
                runtime
                    .ctx()
                    .function_sysv("__waapi_test_gc", 0, test_gc as *const c_void)
            }
            .unwrap();
            runtime.ctx().global_set("__waapi_test_gc", gc).unwrap();
        });
        evaluate(
            r#"
            const frames = c.keyframes([{
                get opacity() {return Array.from({length: 2048}, (_, i) => i).join('x');},
                get transform() {
                    const nested = c.keyframes([{opacity: .75}], 'nested');
                    check(nested[0].props[0][1] === '0.75');
                    __waapi_test_gc();
                    return 'scale(2)';
                }
            }], 'outer');
            check(frames[0].props[0][1].startsWith('0x1x2x3') && frames[0].props[0][1].endsWith('2047'));
            const t = c.timing(base, {get delay() {
                check(c.timing(base, 7, 'nested').duration === 7);
                __waapi_test_gc(); return 2;
            }}, 'outer');
            check(t.delay === 2);
            const sparse = [];
            Object.defineProperty(sparse, 0, {get() {return 'first-' + Array.from({length: 256}, (_, i) => i).join('x');}});
            Object.defineProperty(sparse, 2048, {get() {__waapi_test_gc(); return 'last';}});
            const sparseFrames = c.keyframes({opacity: sparse}, 'sparse-gc');
            same(sparseFrames.map(f => f.computed), [0, 1]);
            check(sparseFrames[0].props[0][1].startsWith('first-0x1x2') && sparseFrames[0].props[0][1].endsWith('255'));
            check(sparseFrames[1].props[0][1] === 'last');
        "#,
        );
    });
}

#[test]
fn computed_timing_uses_full_double_precision_and_boundary_fill() {
    sk_runtime::testing::on_js_thread(|| {
        evaluate(
            r#"
        const t = c.timing(base, {duration: 100, delay: 10, endDelay: 20, iterations: 2, direction: 'alternate'}, 'test');
        same(c.computed(t, null, false), {fill: 'none', duration: 100, endTime: 230, activeDuration: 200,
            localTime: null, progress: null, currentIteration: null});
        const middle = c.computed(t, 135, false);
        same([middle.progress, middle.currentIteration], [.75, 1]);
        check(!c.inEffect(t, 10, true) && c.inEffect(t, 10, false));
        check(c.inEffect(t, 210, true) && !c.inEffect(t, 210, false));
        const filled = c.timing(t, {fill: 'both'}, 'test');
        same([c.computed(filled, -5, false).progress, c.computed(filled, 300, false).progress], [0, 0]);
        check(c.endTime(t) === 230);
        const precise = c.computed(c.timing(base, 1, 'test'), .123456789123, false).progress;
        check(precise === .123456789123, 'linear progress must not narrow to f32');
        const zero = c.timing(base, {duration: 0, iterations: Infinity, fill: 'both'}, 'test');
        const z = c.computed(zero, 0, false);
        check(z.activeDuration === 0 && z.endTime === 0 && !Number.isNaN(z.progress));
        check(c.validEasing('steps(4, end)') && !c.validEasing('not-an-easing'));
        check(c.evaluateEasing('steps(4, end)', .49) === .25);
        check(c.evaluateEasing('cubic-bezier(.2, 2, .8, 2)', .5) > 1, 'overshoot preserved');
    "#,
        )
    });
}

#[test]
fn typed_painting_exchanges_pairs_without_delimiters() {
    sk_runtime::testing::on_js_thread(|| {
        let node = crate::tree::with_tree(|tree| tree.create_root());
        evaluate(&format!(
            r#"
            const id = 389000001;
            const t = c.timing(base, {{duration: 100, fill: 'both'}}, 'test');
            const frames = c.keyframes([{{opacity: 0, 'margin-left': '1px', '--odd': 'a\x1fb\x1ec'}}, {{opacity: 1, 'margin-left': '10px'}}], 'test');
            const notes = c.setEffect(id, {node}, frames, t);
            check(notes.accepted && notes.error === null);
            same(notes.hyphenated, ['margin-left']);
            check(notes.ignored.includes('--odd'));
            c.clock(id, NaN, 50, 1);
            const values = c.values(id);
            check(Array.isArray(values), 'real typed pairs');
            const opacity = values.find(pair => pair[0] === 'opacity');
            check(opacity && Math.abs(Number(opacity[1]) - .5) < 1e-6, 'native painter sampled held clock');
            c.drop(id);
            check(c.values(id) === null);
            const bad = c.setEffect(id, {node}, [{{computed: 2, easing: 'linear', props: []}}], t);
            check(!bad.accepted && bad.error !== null);
        "#
        ));
    });
}

#[test]
fn internal_property_operations_ignore_mutated_global_helpers_but_keep_proxy_traps() {
    sk_runtime::testing::on_js_thread(|| {
        evaluate(
            r#"
        const saved = [Reflect.ownKeys, Reflect.has, Object.getOwnPropertyDescriptor];
        try {
            Reflect.ownKeys = () => {throw new Error('unexpected global ownKeys call');};
            Reflect.has = () => false;
            Object.getOwnPropertyDescriptor = () => {throw new Error('unexpected global descriptor call');};
            const t = c.timing(base, {delay: 3}, 'test');
            check(t.delay === 3 && t.duration === 'auto');
            check(c.keyframes({opacity: [0, 1]}, 'test').length === 2);
            const sentinel = {};
            const proxy = new Proxy([], {has() {throw sentinel;}});
            proxy.length = 1;
            try {c.keyframes({opacity: proxy}, 'test'); check(false);}
            catch (error) {check(error === sentinel, 'intrinsic HasProperty retains user trap exception');}
        } finally {
            [Reflect.ownKeys, Reflect.has, Object.getOwnPropertyDescriptor] = saved;
        }
    "#,
        )
    });
}

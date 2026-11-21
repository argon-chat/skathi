//! `skathi:toml`: `TOML.parse` and `TOML.stringify`, in the shape of `JSON`.

use std::str::FromStr;

use toml::value::Datetime;
use toml::{Table, Value as Toml};

use crate::js::{self, Context, Error, Result, Value};

const KIND_UNDEFINED: u32 = 0;
const KIND_NULL: u32 = 1;
const KIND_BOOL: u32 = 2;
const KIND_INT32: u32 = 3;
const KIND_DOUBLE: u32 = 4;
const KIND_STRING: u32 = 5;
const KIND_SYMBOL: u32 = 6;
const KIND_BIGINT: u32 = 7;
const KIND_FUNCTION: u32 = 9;
const KIND_ARRAY: u32 = 10;

const TYPE_ERROR: u32 = 1;
const RANGE_ERROR: u32 = 2;
const SYNTAX_ERROR: u32 = 3;

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const MAX_DEPTH: usize = 128;

fn fail<T>(context: &Context, kind: u32, message: &str) -> Result<T> {
    Err(Error::Exception(context.error(kind, message)))
}

fn member(context: &Context, object: Value, name: &str) -> Result<Value> {
    context.get(object, &context.atom(name))
}

fn line_and_column(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

fn datetime_to_js(context: &Context, datetime: &Datetime) -> Result<Value> {
    let text = context.string(&datetime.to_string());
    if datetime.offset.is_some() {
        return context.construct(context.global_get("Date")?, &[text]);
    }
    let class = match (datetime.date.is_some(), datetime.time.is_some()) {
        (true, true) => "PlainDateTime",
        (true, false) => "PlainDate",
        _ => "PlainTime",
    };
    let temporal = context.global_get("Temporal")?;
    if !temporal.is_cell() {
        return Ok(text);
    }
    let class = member(context, temporal, class)?;
    context.call(member(context, class, "from")?, class, &[text])
}

fn to_js(context: &Context, value: &Toml) -> Result<Value> {
    Ok(match value {
        Toml::String(s) => context.string(s),
        Toml::Integer(i) if i.unsigned_abs() <= MAX_SAFE_INTEGER as u64 => Value::number(*i as f64),
        Toml::Integer(i) => context.call(
            context.global_get("BigInt")?,
            Value::undefined(),
            &[context.string(&i.to_string())],
        )?,
        Toml::Float(f) => Value::number(*f),
        Toml::Boolean(b) => Value::boolean(*b),
        Toml::Datetime(datetime) => datetime_to_js(context, datetime)?,
        Toml::Array(items) => {
            let array = context.array(0)?;
            for (i, item) in items.iter().enumerate() {
                context.set_index(array, i as u32, to_js(context, item)?)?;
            }
            array
        }
        Toml::Table(table) => table_to_js(context, table)?,
    })
}

fn table_to_js(context: &Context, table: &Table) -> Result<Value> {
    let object = context.object();
    for (key, value) in table {
        context.define(object, &context.atom(key), to_js(context, value)?, 0)?;
    }
    Ok(object)
}

fn parse(context: &Context, text: &str) -> Result<Value> {
    match Table::from_str(text) {
        Ok(table) => table_to_js(context, &table),
        Err(error) => {
            let (line, column) = line_and_column(text, error.span().map_or(0, |span| span.start));
            fail(
                context,
                SYNTAX_ERROR,
                &format!(
                    "TOML.parse: line {line}, column {column}: {}",
                    error.message()
                ),
            )
        }
    }
}

struct Writer<'a> {
    context: &'a Context,
    tag: Value,
    keys: Value,
}

impl Writer<'_> {
    fn unsupported<T>(&self, path: &str, what: &str) -> Result<T> {
        fail(
            self.context,
            TYPE_ERROR,
            &format!("TOML.stringify: {path} is {what}, which TOML cannot represent"),
        )
    }

    fn datetime(&self, object: Value, method: &str, path: &str) -> Result<Toml> {
        let context = self.context;
        let text = context.call(member(context, object, method)?, object, &[])?;
        let text = context.read_string(text)?;
        match Datetime::from_str(&text) {
            Ok(datetime) => Ok(Toml::Datetime(datetime)),
            Err(_) => self.unsupported(path, &format!("the date {text}")),
        }
    }

    /// `None`: a value `JSON.stringify` skips in an object too (undefined, a function, a symbol).
    fn value(&self, value: Value, path: &str, depth: usize) -> Result<Option<Toml>> {
        let context = self.context;
        Ok(Some(match context.kind(value) {
            KIND_UNDEFINED | KIND_FUNCTION | KIND_SYMBOL => return Ok(None),
            KIND_NULL => return self.unsupported(path, "null"),
            KIND_BOOL => Toml::Boolean(value.as_bool().unwrap_or(false)),
            KIND_INT32 | KIND_DOUBLE => {
                let number = value.as_number().unwrap_or(f64::NAN);
                if number.fract() == 0.0 && number.abs() <= MAX_SAFE_INTEGER as f64 {
                    Toml::Integer(number as i64)
                } else {
                    Toml::Float(number)
                }
            }
            KIND_STRING => Toml::String(context.read_string(value)?),
            KIND_BIGINT => match context.to_rust_string(value)?.parse::<i64>() {
                Ok(integer) => Toml::Integer(integer),
                Err(_) => {
                    return fail(
                        context,
                        RANGE_ERROR,
                        &format!("TOML.stringify: {path} does not fit TOML's 64-bit integers"),
                    )
                }
            },
            _ if depth >= MAX_DEPTH => {
                return self.unsupported(path, "nested too deeply or part of a cycle");
            }
            KIND_ARRAY => {
                let mut items = Vec::new();
                for i in 0..context.array_length(value)? {
                    let at = format!("{path}[{i}]");
                    match self.value(context.get_index(value, i)?, &at, depth + 1)? {
                        Some(item) => items.push(item),
                        None => return self.unsupported(&at, "undefined or a function"),
                    }
                }
                Toml::Array(items)
            }
            _ => {
                let tag = context.read_string(context.call(self.tag, value, &[])?)?;
                match tag.as_str() {
                    "[object Date]" => self.datetime(value, "toISOString", path)?,
                    "[object Temporal.PlainDateTime]"
                    | "[object Temporal.PlainDate]"
                    | "[object Temporal.PlainTime]" => self.datetime(value, "toString", path)?,
                    _ => Toml::Table(self.table(value, path, depth + 1)?),
                }
            }
        }))
    }

    fn table(&self, object: Value, path: &str, depth: usize) -> Result<Table> {
        let context = self.context;
        let keys = context.call(self.keys, Value::undefined(), &[object])?;
        let mut table = Table::new();
        for i in 0..context.array_length(keys)? {
            let key = context.read_string(context.get_index(keys, i)?)?;
            let at = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            let value = context.get(object, &context.atom(&key))?;
            if let Some(value) = self.value(value, &at, depth)? {
                table.insert(key, value);
            }
        }
        Ok(table)
    }
}

fn stringify(context: &Context, value: Value, options: Value) -> Result<Value> {
    let object = context.global_get("Object")?;
    let writer = Writer {
        context,
        tag: member(context, member(context, object, "prototype")?, "toString")?,
        keys: member(context, object, "keys")?,
    };
    if !value.is_cell()
        || matches!(
            context.kind(value),
            KIND_STRING | KIND_SYMBOL | KIND_BIGINT | KIND_ARRAY
        )
    {
        return fail(
            context,
            TYPE_ERROR,
            "TOML.stringify: a TOML document is a table, so the value must be an object",
        );
    }
    let table = writer.table(value, "", 0)?;
    let pretty = options.is_cell() && member(context, options, "pretty")?.as_bool() == Some(true);
    let text = if pretty {
        toml::to_string_pretty(&table)
    } else {
        toml::to_string(&table)
    };
    match text {
        Ok(text) => Ok(context.string(&text)),
        Err(error) => fail(context, TYPE_ERROR, &format!("TOML.stringify: {error}")),
    }
}

pub(super) fn install(context: &Context, std: Value) -> Result<()> {
    let toml = context.object();
    let parse = js::function(context, "parse", 1, |context, args| {
        let text = args.arg(0);
        if context.kind(text) != KIND_STRING {
            return fail(context, TYPE_ERROR, "TOML.parse: the text must be a string");
        }
        parse(context, &context.read_string(text)?)
    })?;
    let stringify = js::function(context, "stringify", 1, |context, args| {
        stringify(context, args.arg(0), args.arg(1))
    })?;
    context.define(toml, &context.atom("parse"), parse, 0)?;
    context.define(toml, &context.atom("stringify"), stringify, 0)?;
    context.define(std, &context.atom("TOML"), toml, 0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_offset_is_a_line_and_a_column_counted_from_one() {
        let text = "a = 1\nbé = \n";
        assert_eq!(super::line_and_column(text, 0), (1, 1));
        assert_eq!(
            super::line_and_column(text, text.find("= \n").unwrap() + 2),
            (2, 6)
        );
    }
}

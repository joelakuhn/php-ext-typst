#![cfg_attr(windows, feature(abi_vectorcall))]
use std::collections::HashMap;
use std::path::{ Path };
use std::fs;

use ext_php_rs::flags::DataType;
use typst::eval::{ Library, Datetime };
use typst::diag::{ FileResult, FileError, SourceError };
use typst::syntax::{ Source, SourceId };
use typst::font::{ Font, FontBook };
use typst::util::Buffer;
use typst::eval::Value;
use typst::World;

use comemo::Prehashed;

use ext_php_rs::{prelude::*};
use ext_php_rs::binary::Binary;
use ext_php_rs::types::{Zval, ZendHashTable};

mod fonts;
use fonts::FontSearcher;
use fonts::FontSlot;

// WORLD

struct PHPWorld {
    library: Prehashed<Library>,
    source: Box<Source>,
    book: Prehashed<FontBook>,
    fonts: Vec<FontSlot>,
}

impl PHPWorld {
    fn new(builder: &Typst) -> Self {
        let mut fontsearcher = FontSearcher::new();
        fontsearcher.search_system();

        Self {
            library: Prehashed::new(make_library(builder)),
            source: Box::new(Source::new(SourceId::from_u16(0u16), Path::new(""), builder.body.to_owned())),
            book: Prehashed::new(fontsearcher.book),
            fonts: fontsearcher.fonts,
        }
    }
}

impl World for PHPWorld {
    fn root(&self) -> &Path {
        Path::new("")
    }

    fn library(&self) -> &Prehashed<Library> {
        &self.library
    }

    fn main(&self) -> &Source {
        &self.source
    }

    fn resolve(&self, _path: &Path) -> FileResult<SourceId> {
        FileResult::Ok(SourceId::from_u16(0u16))
    }

    fn source(&self, _id: SourceId) -> &Source {
        &self.source
    }

    fn book(&self) -> &Prehashed<FontBook> {
        &self.book
    }

    fn font(&self, id: usize) -> Option<Font> {
        let slot = &self.fonts[id];
        slot.font
            .get_or_init(|| {
                let data = self.file(&slot.path).ok()?;
                Font::new(data, slot.index)
            })
            .clone()
    }

    fn file(&self, path: &Path) -> FileResult<Buffer> {
        read(path).map(Buffer::from).clone()
    }

    fn today(&self, _offset:Option<i64>) -> Option<typst::eval::Datetime> {
        Some(Datetime::from_ymd(1970, 1, 1).unwrap())
    }
}

// HELPERS

fn make_library(builder: &Typst) -> Library {
    let mut lib = typst_library::build();
    let scope = lib.global.scope_mut();

    for (k, v) in builder.json.to_owned().into_iter() {
        let serde_value: Result<serde_json::Value, _> = serde_json::from_slice(v.as_bytes());
        if serde_value.is_ok() {
            let typst_val = json_to_typst(serde_value.unwrap());
            scope.define(k, typst_val);
        }
    }

    for (k, v) in builder.vars.to_owned().into_iter() {
        scope.define(k, v);
    }

    return lib;
}

fn read(path: &Path) -> FileResult<Vec<u8>> {
    let f = |e| FileError::from_io(e, path);
    if fs::metadata(path).map_err(f)?.is_dir() {
        Err(FileError::IsDirectory)
    } else {
        fs::read(path).map_err(f)
    }
}

// CONVERTERS

fn json_to_typst(value: serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::None,
        serde_json::Value::Bool(v) => Value::Bool(v),
        serde_json::Value::Number(v) => match v.as_i64() {
            Some(int) => Value::Int(int),
            None => Value::Float(v.as_f64().unwrap_or(f64::NAN)),
        },
        serde_json::Value::String(v) => Value::Str(v.into()),
        serde_json::Value::Array(v) => {
            Value::Array(v.into_iter().map(json_to_typst).collect())
        }
        serde_json::Value::Object(v) => Value::Dict(
            v.into_iter()
                .map(|(key, value)| (key.into(), json_to_typst(value)))
                .collect(),
        ),
    }
}

fn ztable_to_typst(arr: &ZendHashTable) -> Value {
    Value::Dict(
        arr.iter().map(|(n, s, v)| {
            if s.is_some() { (s.unwrap(), v) }
            else { (n.to_string(), v) }
        }).map(|(s, v)| (s.into(), zval_to_typst(v))).collect()
    )
}

fn zval_to_typst(value: &Zval) -> Value {
    match value.get_type() {
        DataType::Undef => Value::None,
        DataType::Null => Value::None,
        DataType::False => Value::Bool(false),
        DataType::True => Value::Bool(true),
        DataType::Long => Value::Int(value.long().unwrap()),
        DataType::Double => Value::Float(value.double().unwrap()),
        DataType::String => Value::Str(value.string().unwrap().into()),
        DataType::Array => {
            let arr = value.array().unwrap();
            if arr.has_numerical_keys() {
                Value::Array(arr.iter().map(|(_, _, v)| v).map(zval_to_typst).collect())
            }
            else {
                ztable_to_typst(arr)
            }
        }
        DataType::Object(_) => {
            let obj = value.object().unwrap();
            match obj.get_properties() {
                Ok(props) => ztable_to_typst(props),
                _ => Value::None
            }
        },
        DataType::Void => Value::None,
        DataType::Bool => Value::Bool(value.bool().unwrap()),
        // Unsupported
        // DataType::Resource => {},
        // DataType::Reference => {},
        // DataType::Callable => {},
        // DataType::ConstantExpression => {},
        // DataType::Mixed => {},
        // DataType::Ptr => {},
        _ => Value::None,
    }
}

// DIAGNOSTICS

fn get_error_message(world: &dyn World, body: &str, errors: &Vec<SourceError>) -> String {
    let mut full_message = String::from("");
    let mut first = true;
    for error in errors {
        if first { first = false }
        else { full_message.push_str("\n"); }

        let range = error.range(world);
        let body_bytes = body.as_bytes();

        let mut line_number = 1;
        for b in body_bytes[0..range.start].iter() {
            if *b == 0x0A {
                line_number += 1
            }
        }

        full_message.push_str(&format!("Typst error on line {}: ", line_number));
        full_message.push_str(&String::from(error.message.to_owned()));

        let mut start = range.start;
        let mut end = range.end;
        if start > 0 && body_bytes[start] == 0x0A {
            start -= 1
        }
        while body_bytes[start] != 0x0A {
            if start == 0 { break; }
            start -= 1;
        }
        if start == 0x0A { start += 1 }
        if end < body_bytes.len() && body_bytes[end] == 0x0A {
            end += 1;
        }
        while end < body_bytes.len() && body_bytes[end] != 0x0A {
            end += 1;
        }
        if end == 0x0A { end -= 1 }


        match String::from_utf8(body_bytes[start..end].into()) {
            Ok(code) => {
                full_message.push_str("\n");
                full_message.push_str(&code);

            }
            _ => {},
        }
    }
    return full_message;
}


// MODULE

#[php_class]
pub struct Typst {
    body: String,
    json: HashMap<String, String>,
    vars: HashMap<String, Value>,
}

#[php_impl(rename_methods = "none")]
impl TypstBuilder {
    fn __construct(body: String) -> Self {
        Self {
            body: body,
            json: HashMap::new(),
            vars: HashMap::new(),
        }
    }

    fn json(&mut self, key: String, value: String) {
        self.json.insert(key, value);
    }

    fn var(&mut self, key: String, value: &Zval) {
        self.vars.insert(key, zval_to_typst(value));
    }

    fn compile(&mut self) -> PhpResult<Binary<u8>> {
        let world = PHPWorld::new(self);

        match typst::compile(&world) {
            Ok(document) => {
                let buffer = typst::export::pdf(&document);
                Ok(buffer.into_iter().collect::<Binary<_>>())
            }
            Err(errors) => {
                Err(PhpException::new(
                    get_error_message(&world, &self.body, &errors),
                    8,
                    ext_php_rs::zend::ce::exception(),
                ))
            }
        }
    }
}

#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
    module
}
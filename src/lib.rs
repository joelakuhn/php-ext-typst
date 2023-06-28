#![cfg_attr(windows, feature(abi_vectorcall))]
use std::collections::HashMap;
use std::path::{ Path };
use std::fs;

use ext_php_rs::flags::DataType;
use typst::eval::{ Library, Datetime };
use typst::diag::{ FileResult, FileError, SourceError };
use typst::geom::{RgbaColor, LumaColor};
use typst::syntax::{ Source, SourceId };
use typst::font::{ Font, FontBook };
use typst::util::Buffer;
use typst::eval::Value;
use typst::World;

use typst_library::prelude::CmykColor;

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

        let body = match builder.body.as_ref() {
            Some(body) => body,
            None => "",
        };

        Self {
            library: Prehashed::new(make_library(builder)),
            source: Box::new(Source::new(SourceId::from_u16(0u16), Path::new(""), body.to_owned())),
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
        FileResult::Err(FileError::AccessDenied)
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
            match obj.get_class_name().unwrap_or(String::from("")).as_str() {
                "TypstCMYK" => Value::Color(CmykColor::new(
                    obj.get_property::<u8>("c").unwrap(),
                    obj.get_property::<u8>("m").unwrap(),
                    obj.get_property::<u8>("y").unwrap(),
                    obj.get_property::<u8>("k").unwrap(),
                ).into()),
                "TypstRGBA" => Value::Color(RgbaColor::new(
                    obj.get_property::<u8>("r").unwrap(),
                    obj.get_property::<u8>("g").unwrap(),
                    obj.get_property::<u8>("b").unwrap(),
                    obj.get_property::<u8>("a").unwrap(),
                ).into()),
                "TypstLuma" => Value::Color(LumaColor::new(
                    obj.get_property::<u8>("luma").unwrap(),
                ).into()),
                _ => match obj.get_properties() {
                    Ok(props) => ztable_to_typst(props),
                    _ => Value::None
                }
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

fn csv_to_typst(csv: &String, delimiter: u8, use_headers: bool) -> Value {
    let mut builder = csv::ReaderBuilder::new();
    builder.has_headers(use_headers);
    builder.delimiter(delimiter);

    let mut reader = builder.from_reader(csv.as_bytes());

    if use_headers {
        let mut array = typst::eval::Array::new();
        let headers = match reader.headers() {
            Ok(header_record) => header_record.into_iter().map(|r|
                match String::from_utf8(r.as_bytes().into()) {
                    Ok(h) => h,
                    _ => String::from(""),
                }
            ).collect(),
            _ => vec![String::from("")],
        };

        for (_line, result) in reader.records().enumerate() {
            match result {
                Ok(row) => {
                    let cells : Vec<Value> = row.into_iter().map(|f| Value::Str(f.into())).collect();
                    let dict = Value::Dict(cells.into_iter().zip(&headers).into_iter().map(|(cell, header)| {
                        (header.to_owned().into(), cell)
                    }).collect());
                    array.push(dict);
                }
                _ => {}
            }
        }
        return Value::Array(array);
    }
    else {
        let mut array = typst::eval::Array::new();

        for (_line, result) in reader.records().enumerate() {
            match result {
                Ok(row) => {
                    let cells = row.into_iter().map(|f| Value::Str(f.into())).collect();
                    array.push(Value::Array(cells));
                }
                _ => {}
            }
        }
        return Value::Array(array);
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
#[allow(dead_code)]
pub struct TypstCMYK {
    #[prop]
    pub c: u8,
    #[prop]
    pub m: u8,
    #[prop]
    pub y: u8,
    #[prop]
    pub k: u8,
}

#[php_class]
#[allow(dead_code)]
pub struct TypstRGBA {
    #[prop]
    pub r: u8,
    #[prop]
    pub g: u8,
    #[prop]
    pub b: u8,
    #[prop]
    pub a: u8,
}

#[php_class]
#[allow(dead_code)]
pub struct TypstLuma {
    #[prop]
    pub luma: u8,
}



#[php_class]
pub struct Typst {
    body: Option<String>,
    json: HashMap<String, String>,
    vars: HashMap<String, Value>,
}

#[php_impl(rename_methods = "none")]
impl Typst {
    fn __construct(body: Option<String>) -> Self {
        Self {
            body: body,
            json: HashMap::new(),
            vars: HashMap::new(),
        }
    }

    fn body(&mut self, body: String) {
        self.body = Some(body);
    }

    fn json(&mut self, key: String, value: String) {
        self.json.insert(key, value);
    }

    fn csv(&mut self, key: String, value: String, delimiter: Option<String>, use_headers: Option<bool>) {
        let real_delimiter = match delimiter {
            Some(d) => match d.as_bytes().get(0) {
                Some(b) => b.to_owned(),
                None => 0x2cu8,
            }
            None => 0x2cu8,
        };

        let real_use_headers = match use_headers {
            Some(u) => u,
            None => false,
        };

        self.vars.insert(key, csv_to_typst(&value, real_delimiter, real_use_headers));
    }

    fn var(&mut self, key: String, value: &Zval) {
        self.vars.insert(key, zval_to_typst(value));
    }

    fn compile(&mut self) -> PhpResult<Binary<u8>> {
        let world = PHPWorld::new(self);

        if !self.body.is_some() {
            return Err(PhpException::default(String::from("No body for typst compiler")));
        }

        match typst::compile(&world) {
            Ok(document) => {
                let buffer = typst::export::pdf(&document);
                Ok(buffer.into_iter().collect::<Binary<_>>())
            }
            Err(errors) => {
                Err(PhpException::new(
                    get_error_message(&world, &self.body.as_ref().unwrap(), &errors),
                    8,
                    ext_php_rs::zend::ce::exception(),
                ))
            }
        }
    }

    fn cmyk(c: u8, m: u8, y: u8, k: u8) -> TypstCMYK {
        TypstCMYK { c, m, y, k }
    }

    fn rgba(r: u8, g: u8, b: u8, a: Option<u8>) -> TypstRGBA {
        TypstRGBA { r, g, b, a: a.unwrap_or(255) }
    }

    fn luma(luma: u8) -> TypstLuma {
        TypstLuma { luma }
    }

    fn system_fonts(&self) -> Vec<String> {
        PHPWorld::new(self).book().families().map(|(family, _info)|
            String::from(family)
        ).collect()
    }
}

#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
    module
}

#![cfg_attr(windows, feature(abi_vectorcall))]
use std::collections::HashMap;
use std::path::Path;
use std::fs;

use ext_php_rs::flags::DataType;
use typst::ecow::EcoVec;
use typst::Library;
// use typst::eval::{ Library, Datetime };
use typst::diag::{ FileError, FileResult, SourceDiagnostic, Warned };
use typst::visualize::{Luma, Rgb};
use typst::syntax::{ FileId, Source, Span, VirtualPath };
use typst::text::{ Font, FontBook };
use typst::World;
use typst::foundations::{Binding, Datetime, Value, Bytes};

use typst::utils::LazyHash;

use ext_php_rs::{prelude::*};
use ext_php_rs::binary::Binary;
use ext_php_rs::types::{Zval, ZendHashTable};

mod fonts;
use fonts::FontSearcher;
use fonts::FontSlot;
use typst_pdf::PdfOptions;

// WORLD

struct PHPWorld {
    library: LazyHash<Library>,
    main: Source,
    book: LazyHash<FontBook>,
    fonts: Vec<FontSlot>,
}

impl PHPWorld {
    fn new(builder: &Typst) -> Self {
        let mut fontsearcher = FontSearcher::new();
        fontsearcher.search_system();

        for font_path in &builder.fonts {
            let path = Path::new(&font_path);
            if path.is_dir() { fontsearcher.search_dir(&path); }
            else if path.is_file() { fontsearcher.search_file(&path); }
        }

        let body = match builder.body.as_ref() {
            Some(body) => body,
            None => "",
        };

        let file_id = FileId::new(None, VirtualPath::new("./::php_source::"));

        Self {
            library: LazyHash::new(make_library(builder)),
            main: Source::new(file_id, body.to_owned()),
            book: LazyHash::new(fontsearcher.book),
            fonts: fontsearcher.fonts,
        }
    }
}

impl World for PHPWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, _id: FileId) -> FileResult<Source> {
        Ok(self.main.clone())
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn font(&self, id: usize) -> Option<Font> {
        let slot = &self.fonts[id];
        let data = read(&slot.path).unwrap();
        let bytes : Bytes = Bytes::new(data);
        Font::new(bytes, slot.index)
    }

    fn file(&self, path: FileId) -> FileResult<Bytes> {
        // if path.components().any(|c| c.as_os_str() == "..") {
        //     Err(FileError::AccessDenied)
        // }
        // else if !path.is_relative() {
        //     Err(FileError::AccessDenied)
        // }
        // else {
        
            let data = read(path.vpath().as_rooted_path()).unwrap();
            let bytes : Bytes = Bytes::new(data);
            Ok(bytes)
        // }
    }

    fn today(&self, _offset:Option<i64>) -> Option<Datetime> {
        Some(Datetime::from_ymd(1970, 1, 1).unwrap())
    }
}

// HELPERS

fn make_library(builder: &Typst) -> Library {
    let mut lib = Library::builder().build();
    let scope = lib.global.scope_mut();

    for (k, v) in builder.json.to_owned().into_iter() {
        let serde_value: Result<serde_json::Value, _> = serde_json::from_slice(v.as_bytes());
        if serde_value.is_ok() {
            let typst_val = json_to_typst(serde_value.unwrap());
            scope.bind(k.into(), Binding::new(typst_val, Span::detached()));
        }
    }

    for (k, v) in builder.vars.to_owned().into_iter() {
        scope.bind(k.into(), Binding::new(v, Span::detached()));
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
        arr.iter()
        .map(|(key, v)| (key.to_string(), v))
        .map(|(s, v)| (s.into(), zval_to_typst(v))).collect()
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
                Value::Array(arr.iter().map(|(_, v)| v).map(zval_to_typst).collect())
            }
            else {
                ztable_to_typst(arr)
            }
        }
        DataType::Object(_) => {
            let obj = value.object().unwrap();
            match obj.get_class_name().unwrap_or(String::from("")).as_str() {
                // "TypstCMYK" => Value::Color(Cmyk::new(
                //     obj.get_property::<u8>("c").unwrap() as f32,
                //     obj.get_property::<u8>("m").unwrap() as f32,
                //     obj.get_property::<u8>("y").unwrap() as f32,
                //     obj.get_property::<u8>("k").unwrap() as f32,
                // ).into()),
                "TypstRGBA" => Value::Color(Rgb::new(
                    obj.get_property::<u8>("r").unwrap() as f32,
                    obj.get_property::<u8>("g").unwrap() as f32,
                    obj.get_property::<u8>("b").unwrap() as f32,
                    obj.get_property::<u8>("a").unwrap() as f32,
                ).into()),
                "TypstLuma" => Value::Color(Luma::new(
                    obj.get_property::<u8>("luma").unwrap() as f32,
                    1.0,
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
        let mut array = typst::foundations::Array::new();
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
        let mut array = typst::foundations::Array::new();

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

fn get_error_message(_world: &dyn World, _body: &str, errors: &EcoVec<SourceDiagnostic>) -> String {
    let mut full_message = String::from("");
    let mut first = true;
    for error in errors {
        if first { first = false }
        else { full_message.push_str("\n"); }

        full_message.push_str(&error.message);

        // let range = error.(world);
        // let body_bytes = body.as_bytes();

        // let mut line_number = 1;
        // for b in body_bytes[0..range.start].iter() {
        //     if *b == 0x0A {
        //         line_number += 1
        //     }
        // }

        // full_message.push_str(&format!("Typst error on line {}: ", line_number));
        // full_message.push_str(&String::from(error.message.to_owned()));

        // let mut start = range.start;
        // let mut end = range.end;
        // if start > 0 && body_bytes[start] == 0x0A {
        //     start -= 1
        // }
        // while body_bytes[start] != 0x0A {
        //     if start == 0 { break; }
        //     start -= 1;
        // }
        // if start == 0x0A { start += 1 }
        // if end < body_bytes.len() && body_bytes[end] == 0x0A {
        //     end += 1;
        // }
        // while end < body_bytes.len() && body_bytes[end] != 0x0A {
        //     end += 1;
        // }
        // if end == 0x0A { end -= 1 }


        // match String::from_utf8(body_bytes[start..end].into()) {
        //     Ok(code) => {
        //         full_message.push_str("\n");
        //         full_message.push_str(&code);

        //     }
        //     _ => {},
        // }
    }
    return full_message;
}


// MODULE

#[php_class]
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
pub struct TypstLuma {
    #[prop]
    pub luma: u8,
}



#[php_class]
pub struct Typst {
    body: Option<String>,
    json: HashMap<String, String>,
    vars: HashMap<String, Value>,
    fonts: Vec<String>,
}

#[php_impl(rename_methods = "none")]
impl Typst {
    fn __construct(body: Option<String>) -> Self {
        Self {
            body: body,
            json: HashMap::new(),
            vars: HashMap::new(),
            fonts: vec![],
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

        let Warned { output, warnings } = typst::compile(&world);
        match output {
            Ok(document) => {
                match typst_pdf::pdf(&document, &PdfOptions::default()) {
                    Ok(buffer) => Ok(buffer.into_iter().collect::<Binary<_>>()),
                    Err(errors) => {
                        println!("{:?}", errors);
                        Err(PhpException::new(
                            get_error_message(&world, &self.body.as_ref().unwrap(), &warnings),
                            8,
                            ext_php_rs::zend::ce::exception(),
                        ))
                    }
                }
            }
            Err(errors) => {
                println!("{:?}", errors);
                Err(PhpException::new(
                    get_error_message(&world, &self.body.as_ref().unwrap(), &warnings),
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

    fn register_font(&mut self, path: String) -> PhpResult<()> {
        if !path.starts_with("./") {
            Err(PhpException::default(String::from("Path must be relative.")))
        }
        else if path.contains("..") {
            Err(PhpException::default(String::from("Path attempts to traverse parent.")))
        }
        else {
            self.fonts.push(path);
            Ok(())
            // Err(PhpException::default(String::from("sdf")))
        }
    }
}

#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
    module
}

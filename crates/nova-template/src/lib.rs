//! NOVA template front end.
//!
//! `JSON text -> bind data -> typed DSL -> validate + compile -> Scene`.
//! Everything that can be wrong with a template is reported here, before any
//! rasterisation starts.

pub mod assets;
pub mod binding;
pub mod compile;
pub mod dsl;

use std::path::{Path, PathBuf};

use nova_core::{Error, ErrorCode, Limits};
use serde_json::Value;

pub use compile::{CompileOptions, Compiled, Warning};

/// Largest template (or data) file accepted.
pub const MAX_TEMPLATE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// Directory asset paths resolve against. Default: the template's folder.
    pub asset_root: Option<PathBuf>,
    pub limits: Limits,
}

/// Parse JSON text, reporting syntax errors with line and column.
pub fn parse_json(text: &str, what: &str) -> Result<Value, Error> {
    serde_json::from_str(text).map_err(|e| {
        Error::new(
            ErrorCode::InvalidTemplate,
            format!("{what} is not valid JSON: {e}"),
        )
    })
}

/// Read a JSON file with a size cap.
pub fn read_json_file(path: &Path, what: &str) -> Result<Value, Error> {
    let meta = std::fs::metadata(path)
        .map_err(|e| Error::io(format!("cannot read {what} `{}`", path.display()), e))?;
    if meta.len() > MAX_TEMPLATE_BYTES {
        return Err(Error::new(
            ErrorCode::Limit,
            format!(
                "{what} `{}` is {} bytes; the limit is {MAX_TEMPLATE_BYTES}",
                path.display(),
                meta.len()
            ),
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::io(format!("cannot read {what} `{}`", path.display()), e))?;
    parse_json(&text, what)
}

/// Turn bound JSON into the typed DSL. Errors carry the path of the offending
/// field, e.g. `layers[3].text.font_size`.
pub fn parse_template(value: Value) -> Result<dsl::Template, Error> {
    serde_path_to_error::deserialize(value).map_err(|e| {
        let path = e.path().to_string();
        let message = e.inner().to_string();
        if path.is_empty() || path == "." {
            Error::new(ErrorCode::InvalidTemplate, message)
        } else {
            Error::at(ErrorCode::InvalidTemplate, path, message)
        }
    })
}

/// Bind `data` into a template value and compile it.
pub fn load_value(
    template: &Value,
    data: Option<&Value>,
    asset_root: &Path,
    limits: &Limits,
) -> Result<Compiled, Vec<Error>> {
    let empty = Value::Object(Default::default());
    let bound = binding::bind(template, data.unwrap_or(&empty))?;
    let parsed = parse_template(bound).map_err(|e| vec![e])?;
    compile::compile(
        &parsed,
        &CompileOptions {
            asset_root: asset_root.to_owned(),
            limits: limits.clone(),
        },
    )
}

/// Load, bind and compile a template file. This is what `nova render` uses.
pub fn load_file(
    template_path: &Path,
    data: Option<&Value>,
    opts: &LoadOptions,
) -> Result<Compiled, Vec<Error>> {
    let value = read_json_file(template_path, "template").map_err(|e| vec![e])?;
    let root = match &opts.asset_root {
        Some(root) => root.clone(),
        None => match template_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_owned(),
            _ => PathBuf::from("."),
        },
    };
    load_value(&value, data, &root, &opts.limits)
}

/// JSON Schema (draft 2020-12) of the template format, generated from the
/// same types the parser uses.
pub fn json_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(dsl::Template)).expect("schema serialises")
}

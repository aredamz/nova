//! Data binding: `{{slot.path}}` placeholders in template strings.
//!
//! This is substitution only. There are no expressions, no function calls,
//! no environment or filesystem access, and substituted data is never
//! scanned for further placeholders.

use nova_core::{Error, ErrorCode};
use serde_json::Value;

/// One `{{...}}` occurrence inside a string.
struct Slot<'a> {
    /// Byte range of the whole `{{ ... }}` in the source string.
    start: usize,
    end: usize,
    path: &'a str,
}

/// Find placeholders. Returns `Err(position)` for a malformed one
/// (unterminated, or a path with characters outside `[A-Za-z0-9_.-]`).
fn find_slots(s: &str) -> Result<Vec<Slot<'_>>, usize> {
    let mut slots = Vec::new();
    let mut from = 0;
    while let Some(rel) = s[from..].find("{{") {
        let start = from + rel;
        let close = s[start + 2..].find("}}").ok_or(start)?;
        let end = start + 2 + close + 2;
        let path = s[start + 2..end - 2].trim();
        let valid = !path.is_empty()
            && path.split('.').all(|seg| {
                !seg.is_empty()
                    && seg
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            });
        if !valid {
            return Err(start);
        }
        slots.push(Slot { start, end, path });
        from = end;
    }
    Ok(slots)
}

fn lookup<'a>(data: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = data;
    for seg in path.split('.') {
        cur = match cur {
            Value::Object(map) => map.get(seg)?,
            Value::Array(items) => items.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn scalar_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Resolve every placeholder in `template` from `data`.
///
/// A string that is exactly one placeholder takes the data value with its
/// JSON type (so a slot can supply a number or a whole object). A placeholder
/// inside a longer string is replaced by the value's text and must be a
/// string, number or boolean. All problems are reported, not just the first.
pub fn bind(template: &Value, data: &Value) -> Result<Value, Vec<Error>> {
    let mut errors = Vec::new();
    let out = bind_value(template, data, &mut String::new(), &mut errors);
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

fn bind_value(v: &Value, data: &Value, path: &mut String, errors: &mut Vec<Error>) -> Value {
    match v {
        Value::String(s) => bind_string(s, data, path, errors),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let len = path.len();
                    path.push_str(&format!("[{i}]"));
                    let out = bind_value(item, data, path, errors);
                    path.truncate(len);
                    out
                })
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, item)| {
                    let len = path.len();
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(k);
                    let out = bind_value(item, data, path, errors);
                    path.truncate(len);
                    (k.clone(), out)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn bind_string(s: &str, data: &Value, path: &str, errors: &mut Vec<Error>) -> Value {
    let slots = match find_slots(s) {
        Ok(slots) => slots,
        Err(pos) => {
            errors.push(Error::at(
                ErrorCode::InvalidTemplate,
                path,
                format!(
                    "malformed placeholder at character {pos}: use `{{{{name}}}}` or `{{{{a.b.c}}}}` \
                     (letters, digits, `_`, `-` and `.` only)"
                ),
            ));
            return Value::String(s.to_owned());
        }
    };
    if slots.is_empty() {
        return Value::String(s.to_owned());
    }
    // Whole-string placeholder keeps the JSON type of the data.
    if slots.len() == 1 && slots[0].start == 0 && slots[0].end == s.len() {
        return match lookup(data, slots[0].path) {
            Some(v) => v.clone(),
            None => {
                errors.push(missing(path, slots[0].path));
                Value::String(s.to_owned())
            }
        };
    }
    let mut out = String::with_capacity(s.len());
    let mut cursor = 0;
    for slot in &slots {
        out.push_str(&s[cursor..slot.start]);
        match lookup(data, slot.path) {
            Some(v) => match scalar_to_string(v) {
                Some(text) => out.push_str(&text),
                None => errors.push(Error::at(
                    ErrorCode::MissingData,
                    path,
                    format!(
                        "data slot `{}` must be a string, number or boolean to be used inside text",
                        slot.path
                    ),
                )),
            },
            None => errors.push(missing(path, slot.path)),
        }
        cursor = slot.end;
    }
    out.push_str(&s[cursor..]);
    Value::String(out)
}

fn missing(path: &str, slot: &str) -> Error {
    Error::at(
        ErrorCode::MissingData,
        path,
        format!("no value for data slot `{slot}`"),
    )
}

/// Every distinct slot path a template asks for, sorted.
pub fn list_slots(template: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => {
                if let Ok(slots) = find_slots(s) {
                    out.extend(slots.iter().map(|s| s.path.to_owned()));
                }
            }
            Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            Value::Object(map) => map.values().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(template, &mut out);
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn substitutes_inside_text() {
        let t = json!({ "text": { "value": "Hi {{ user.name }}, {{count}} new" } });
        let d = json!({ "user": { "name": "Aina" }, "count": 3 });
        assert_eq!(
            bind(&t, &d).unwrap(),
            json!({ "text": { "value": "Hi Aina, 3 new" } })
        );
    }

    #[test]
    fn whole_string_slot_keeps_json_type() {
        let t = json!({ "font_size": "{{size}}", "stops": "{{palette}}" });
        let d = json!({ "size": 92, "palette": [{ "offset": 0, "color": "#fff" }] });
        let out = bind(&t, &d).unwrap();
        assert_eq!(out["font_size"], json!(92));
        assert_eq!(out["stops"][0]["color"], json!("#fff"));
    }

    #[test]
    fn array_index_paths() {
        let t = json!("{{items.1.title}}");
        let d = json!({ "items": [{ "title": "a" }, { "title": "b" }] });
        assert_eq!(bind(&t, &d).unwrap(), json!("b"));
    }

    #[test]
    fn reports_every_missing_slot_with_its_location() {
        let t = json!({ "layers": [{ "text": { "value": "{{a}} and {{b}}" } }] });
        let errs = bind(&t, &json!({})).unwrap_err();
        assert_eq!(errs.len(), 2);
        assert!(errs.iter().all(|e| e.code == ErrorCode::MissingData));
        assert_eq!(errs[0].path.as_deref(), Some("layers[0].text.value"));
        assert!(errs[0].message.contains("`a`"));
    }

    #[test]
    fn substituted_data_is_not_rescanned() {
        let t = json!("{{a}}!");
        let d = json!({ "a": "{{secret}}", "secret": "leak" });
        assert_eq!(bind(&t, &d).unwrap(), json!("{{secret}}!"));
    }

    #[test]
    fn no_expressions_or_calls() {
        for bad in [
            "{{a + b}}",
            "{{env(HOME)}}",
            "{{ }}",
            "{{a..b}}",
            "{{unterminated",
        ] {
            let errs = bind(&json!(bad), &json!({ "a": 1, "b": 2 })).unwrap_err();
            assert_eq!(errs[0].code, ErrorCode::InvalidTemplate, "{bad}");
        }
    }

    #[test]
    fn objects_cannot_be_spliced_into_text() {
        let errs = bind(&json!("x {{o}}"), &json!({ "o": { "k": 1 } })).unwrap_err();
        assert_eq!(errs[0].code, ErrorCode::MissingData);
    }

    #[test]
    fn lists_slots() {
        let t = json!({ "a": "{{x.y}} {{z}}", "b": ["{{z}}"], "c": 4 });
        assert_eq!(list_slots(&t), vec!["x.y".to_string(), "z".to_string()]);
    }
}

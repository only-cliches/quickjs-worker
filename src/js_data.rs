use quickjs_runtime::{jsutils::JsError, quickjsrealmadapter::QuickJsRealmAdapter, quickjsvalueadapter::QuickJsValueAdapter};
use neon::{prelude::*, result::Throw, types::{JsBuffer, JsDate, buffer::TypedArray}};

#[derive(Debug, Clone)]
/// Enum `JsDataTypes`.
///
/// **What it represents:**
/// - High-level role in this module (data container / FFI handle / configuration).
///
/// **Key invariants:**
/// - If this item is used across FFI boundaries, its layout/ownership rules must be preserved.
pub enum JsDataTypes {
    Unknown,
    Undefined,
    Null,
    String { msg: String },
    Json { msg: String },
    Array { msg: String },
    Number { msg: f64 },
    Boolean { msg: bool },
    Date { msg: f64 },
    Buffer { msg: Vec<u8> } 
}


/// Implementation block for `ToString for JsDataTypes`.
///
impl ToString for JsDataTypes {
    /// `to_string` — function entry point.
    ///
    /// **Purpose:**
    /// - Describe the behavior at a high level and how callers should use it.
    ///
    /// **Parameters / return:**
    /// - See the signature below; pay special attention to lifetimes and ownership.
    ///
    /// **Error handling:**
    /// - This function typically returns a `Result` when it can fail; propagate context upward.
    fn to_string(&self) -> String {
        match self {
            JsDataTypes::Date { msg } => {
                let msg_str = msg.to_string();
                std::format!("new Date({msg_str})")
            }
            JsDataTypes::String { msg } => std::format!("`{msg}`"),
            JsDataTypes::Json { msg } => msg.clone(),
            JsDataTypes::Array { msg } => msg.clone(),
            JsDataTypes::Number { msg } => msg.to_string(),
            JsDataTypes::Boolean { msg } => if *msg { String::from("true") } else { String::from("false") },
            JsDataTypes::Undefined => String::from("undefined"),
            JsDataTypes::Null => String::from("null"),
            JsDataTypes::Unknown => String::from("undefined"),
            JsDataTypes::Buffer { msg: _ } => String::from("[object Buffer]") 
        }
    }
}

/// Implementation block for `JsDataTypes`.
///
/// The methods inside often form the *public API* for this type and/or encapsulate
/// `unsafe` details so most callers can stay in safe Rust.
impl JsDataTypes {

    /// `to_quick_value` — function entry point.
    ///
    /// **Purpose:**
    /// - Describe the behavior at a high level and how callers should use it.
    ///
    /// **Parameters / return:**
    /// - See the signature below; pay special attention to lifetimes and ownership.
    ///
    /// **Error handling:**
    /// - This function typically returns a `Result` when it can fail; propagate context upward.
    pub fn to_quick_value(&self, realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
        match self {
            JsDataTypes::Date { msg } => {
                let date_fn = realm
                    .get_object_property(
                        &realm.get_global()?,
                        "Date",
                    )?;
                realm
                    .construct_object(
                        &date_fn,
                        &[&realm.create_f64(*msg).unwrap()],
                    )
            }
            JsDataTypes::String { msg } => realm.create_string(msg),
            JsDataTypes::Json { msg } => realm.json_parse(msg.as_str()),
            JsDataTypes::Array { msg } => realm.json_parse(msg.as_str()),
            JsDataTypes::Number { msg } => realm.create_f64(*msg),
            JsDataTypes::Boolean { msg } => realm.create_boolean(*msg),
            JsDataTypes::Undefined => realm.create_undefined(),
            JsDataTypes::Null => realm.create_null(),
            JsDataTypes::Unknown => realm.create_undefined(),
            JsDataTypes::Buffer { msg } => {
                realm.create_typed_array_uint8(msg.clone())
            }
        }
    }

    /// `from_quick_value` — function entry point.
    ///
    /// **Purpose:**
    /// - Describe the behavior at a high level and how callers should use it.
    ///
    /// **Parameters / return:**
    /// - See the signature below; pay special attention to lifetimes and ownership.
    ///
    /// **Error handling:**
    /// - This function typically returns a `Result` when it can fail; propagate context upward.
    pub fn from_quick_value(value: &QuickJsValueAdapter, realm: &QuickJsRealmAdapter) -> Result<Self, JsError> {
        match value.get_js_type() {
            quickjs_runtime::jsutils::JsValueType::I32 => Ok(JsDataTypes::Number { msg: value.to_i32() as f64 }),
            quickjs_runtime::jsutils::JsValueType::F64 => Ok(JsDataTypes::Number { msg: value.to_f64() }),
            quickjs_runtime::jsutils::JsValueType::String => Ok(JsDataTypes::String { msg: value.to_string()? }),
            quickjs_runtime::jsutils::JsValueType::Boolean => Ok(JsDataTypes::Boolean { msg: value.to_bool() }),
            quickjs_runtime::jsutils::JsValueType::Object => {
                // 1. Check for Date
                let get_time = realm.get_object_property(value, "getTime")?;
                if !get_time.is_undefined() { 
                    let time = realm.invoke_function(Some(value), &get_time, &[])?;
                    let msg = time.to_f64();
                    return Ok(JsDataTypes::Date { msg });
                }

                // 2. Check for Uint8Array (Buffer)
                let constructor = realm.get_object_property(value, "constructor")?;
                if !constructor.is_undefined() {
                    let name = realm.get_object_property(&constructor, "name")?;
                    if name.is_string() && name.to_string()? == "Uint8Array" {
                        let length_val = realm.get_object_property(value, "length")?;
                        let len = length_val.to_i32() as usize;
                        
                        let mut msg: Vec<u8> = Vec::with_capacity(len);
                        for i in 0..len {
                            let byte_val = realm.get_array_element(value, i as u32)?;
                            msg.push(byte_val.to_i32() as u8);
                        }
                        return Ok(JsDataTypes::Buffer { msg });
                    }
                }

                // 3. Fallback to generic object
                let msg = realm.json_stringify(value, None)?;
                Ok(JsDataTypes::Json { msg })
            }
            quickjs_runtime::jsutils::JsValueType::Array => Ok(JsDataTypes::Array { msg: realm.json_stringify(value, None)? }),
            quickjs_runtime::jsutils::JsValueType::Null => Ok(JsDataTypes::Null),
            quickjs_runtime::jsutils::JsValueType::Undefined => Ok(JsDataTypes::Undefined),
            _ => {
                Ok(JsDataTypes::Unknown)
            } 
        }
    }

    pub fn to_node_value<'a, C: Context<'a>>(&self, cxf: &mut C) -> Result<Handle<'a, JsValue>, Throw> {
        match self {
            JsDataTypes::Unknown => Ok(cxf.undefined().as_value(cxf)),
            JsDataTypes::Undefined => Ok(cxf.undefined().as_value(cxf)),
            JsDataTypes::Null => Ok(cxf.null().as_value(cxf)),
            JsDataTypes::String { msg } => Ok(cxf.string(msg).as_value(cxf)),
            JsDataTypes::Json { msg } =>  {
                let json_parse = cxf
                .global::<JsObject>("JSON")?
                .get_value(cxf, "parse")?
                .downcast::<JsFunction, _>(cxf)
                .unwrap();
                let out = json_parse
                    .call_with(cxf)
                    .arg(cxf.string(msg))
                    .apply::<JsObject, _>(cxf)?;

                Ok(out.as_value(cxf))
            },
            JsDataTypes::Array { msg } => {
                let json_parse = cxf
                .global::<JsObject>("JSON")?
                .get_value(cxf, "parse")?
                .downcast::<JsFunction, _>(cxf)
                .unwrap();
                let out = json_parse
                    .call_with(cxf)
                    .arg(cxf.string(msg))
                    .apply::<JsObject, _>(cxf)?;

                Ok(out.as_value(cxf))
            },
            JsDataTypes::Number { msg } => Ok(cxf.number(*msg).as_value(cxf)),
            JsDataTypes::Boolean { msg } => Ok(cxf.boolean(*msg).as_value(cxf)),
            JsDataTypes::Date { msg } => Ok(cxf.date(*msg).unwrap().as_value(cxf)),
            JsDataTypes::Buffer { msg } => {
                let mut buffer = JsBuffer::new(cxf, msg.len())?;
                buffer.as_mut_slice(cxf).copy_from_slice(msg);
                Ok(buffer.as_value(cxf))
            }
        }
    }

    pub fn from_node_value<'a, C: Context<'a>, V: Value>(value: Handle<'a, V>, cxf: &mut C) -> Result<Self, Throw>  {

        if value.is_a::<JsDate, _>(cxf) {
            let msg = value
                .downcast::<JsDate, _>(cxf)
                .unwrap()
                .value(cxf);
            return Ok(JsDataTypes::Date { msg })
        } else if value.is_a::<JsString, _>(cxf) {
            let msg = value
                .downcast::<JsString, _>(cxf)
                .unwrap()
                .value(cxf);
            return Ok(JsDataTypes::String { msg })
        
        } else if value.is_a::<JsBuffer, _>(cxf) {
            let buffer = value.downcast::<JsBuffer, _>(cxf).unwrap();
            let msg = buffer.as_slice(cxf).to_vec();
            return Ok(JsDataTypes::Buffer { msg });

        } else if value.is_a::<JsObject, _>(cxf) || value.is_a::<JsArray, _>(cxf) {
            let json_stringify = cxf
                .global::<JsObject>("JSON")?
                .get_value(cxf, "stringify")?
                .downcast::<JsFunction, _>(cxf)
                .unwrap();
            let msg = json_stringify
                .call_with(cxf)
                .arg(value)
                .apply::<JsString, _>(cxf)?
                .value(cxf);
            return Ok(JsDataTypes::Json { msg })
        } else if value.is_a::<JsNumber, _>(cxf) {
            let msg = value
                .downcast::<JsNumber, _>(cxf)
                .unwrap()
                .value(cxf);
            return Ok(JsDataTypes::Number { msg })
        } else if value.is_a::<JsBoolean, _>(cxf) {
            let msg = value
                .downcast::<JsBoolean, _>(cxf)
                .unwrap()
                .value(cxf);
            return Ok(JsDataTypes::Boolean { msg })
        } else if value.is_a::<JsUndefined, _>(cxf) {
            return Ok(JsDataTypes::Undefined)
        } else if value.is_a::<JsNull, _>(cxf) {
            return Ok(JsDataTypes::Null)
        } else {
            return Ok(JsDataTypes::Unknown);
        }

    }
}
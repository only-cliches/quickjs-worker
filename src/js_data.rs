use neon::{
    prelude::*,
    result::Throw,
    types::{buffer::TypedArray, JsBuffer, JsDate},
};
use quickjs_runtime::{
    jsutils::JsError, quickjsrealmadapter::QuickJsRealmAdapter,
    quickjsvalueadapter::QuickJsValueAdapter,
};

#[derive(Debug, Clone)]
pub enum JsDataTypes {
    Unknown,
    Undefined,
    Null,
    String {
        msg: String,
    },
    Json {
        msg: String,
    },
    Array {
        msg: String,
    },
    Number {
        msg: f64,
    },
    Boolean {
        msg: bool,
    },
    Date {
        msg: f64,
    },
    Buffer {
        msg: Vec<u8>,
    },
    Error {
        name: String,
        message: String,
        stack: Option<String>,
    },
}

impl ToString for JsDataTypes {
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
            JsDataTypes::Boolean { msg } => {
                if *msg {
                    String::from("true")
                } else {
                    String::from("false")
                }
            }
            JsDataTypes::Undefined => String::from("undefined"),
            JsDataTypes::Null => String::from("null"),
            JsDataTypes::Unknown => String::from("undefined"),
            JsDataTypes::Buffer { msg: _ } => String::from("[object Buffer]"),
            JsDataTypes::Error {
                name: _,
                message: _,
                stack: _,
            } => String::from("[object Error]"),
        }
    }
}

impl JsDataTypes {
    pub fn to_quick_value(
        &self,
        realm: &QuickJsRealmAdapter,
    ) -> Result<QuickJsValueAdapter, JsError> {
        match self {
            JsDataTypes::Date { msg } => {
                let date_fn = realm.get_object_property(&realm.get_global()?, "Date")?;
                realm.construct_object(&date_fn, &[&realm.create_f64(*msg).unwrap()])
            }
            JsDataTypes::String { msg } => realm.create_string(msg),
            JsDataTypes::Json { msg } => realm.json_parse(msg.as_str()),
            JsDataTypes::Array { msg } => realm.json_parse(msg.as_str()),
            JsDataTypes::Number { msg } => realm.create_f64(*msg),
            JsDataTypes::Boolean { msg } => realm.create_boolean(*msg),
            JsDataTypes::Undefined => realm.create_undefined(),
            JsDataTypes::Null => realm.create_null(),
            JsDataTypes::Unknown => realm.create_undefined(),
            JsDataTypes::Buffer { msg } => realm.create_typed_array_uint8(msg.clone()),
            JsDataTypes::Error {
                name,
                message,
                stack,
            } => {
                let err_ctor = realm.get_object_property(&realm.get_global()?, "Error")?;
                let msg_v = realm.create_string(message)?;
                let err_obj = realm.construct_object(&err_ctor, &[&msg_v])?;

                let _ = realm.set_object_property(&err_obj, "name", &realm.create_string(name)?);
                if let Some(stack) = stack {
                    let _ =
                        realm.set_object_property(&err_obj, "stack", &realm.create_string(stack)?);
                }
                Ok(err_obj)
            }
        }
    }

    pub fn from_quick_value(
        value: &QuickJsValueAdapter,
        realm: &QuickJsRealmAdapter,
    ) -> Result<Self, JsError> {
        match value.get_js_type() {
            quickjs_runtime::jsutils::JsValueType::I32 => Ok(JsDataTypes::Number {
                msg: value.to_i32() as f64,
            }),
            quickjs_runtime::jsutils::JsValueType::F64 => Ok(JsDataTypes::Number {
                msg: value.to_f64(),
            }),
            quickjs_runtime::jsutils::JsValueType::String => Ok(JsDataTypes::String {
                msg: value.to_string()?,
            }),
            quickjs_runtime::jsutils::JsValueType::Boolean => Ok(JsDataTypes::Boolean {
                msg: value.to_bool(),
            }),
            quickjs_runtime::jsutils::JsValueType::Error => {
                let name_val = realm.get_object_property(value, "name")?;
                let msg_val = realm.get_object_property(value, "message")?;

                let name = if name_val.is_string() {
                    name_val.to_string()?
                } else {
                    "Error".to_string()
                };
                let message = if msg_val.is_string() {
                    Some(msg_val.to_string()?)
                } else {
                    None
                };

                let stack_val = realm.get_object_property(value, "stack")?;
                let stack = if stack_val.is_string() {
                    Some(stack_val.to_string()?)
                } else {
                    None
                };


                Ok(JsDataTypes::Error {
                    name: name,
                    message: message.unwrap_or_default(),
                    stack,
                })
            }
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
            quickjs_runtime::jsutils::JsValueType::Array => Ok(JsDataTypes::Array {
                msg: realm.json_stringify(value, None)?,
            }),
            quickjs_runtime::jsutils::JsValueType::Null => Ok(JsDataTypes::Null),
            quickjs_runtime::jsutils::JsValueType::Undefined => Ok(JsDataTypes::Undefined),
            _ => Ok(JsDataTypes::Unknown),
        }
    }

    pub fn to_node_value<'a, C: Context<'a>>(
        &self,
        cxf: &mut C,
    ) -> Result<Handle<'a, JsValue>, Throw> {
        match self {
            JsDataTypes::Unknown => Ok(cxf.undefined().as_value(cxf)),
            JsDataTypes::Undefined => Ok(cxf.undefined().as_value(cxf)),
            JsDataTypes::Null => Ok(cxf.null().as_value(cxf)),
            JsDataTypes::String { msg } => Ok(cxf.string(msg).as_value(cxf)),
            JsDataTypes::Json { msg } => {
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
            }
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
            }
            JsDataTypes::Error {
                name,
                message,
                stack,
            } => {
                let js_err = cxf.error(message.as_str())?;
                let obj = js_err.upcast::<JsObject>();

                // Create handles first (each borrows &mut cx), then use them.
                let name_key = cxf.string("name");
                let name_val = cxf.string(name.as_str());
                obj.set(cxf, name_key, name_val)?;

                if let Some(stack) = stack {
                    let stack_key = cxf.string("stack");
                    let stack_val = cxf.string(stack.as_str());
                    obj.set(cxf, stack_key, stack_val)?;
                }

                Ok(obj.as_value(cxf))
            }
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

    pub fn from_node_value<'a, C: Context<'a>, V: Value>(
        value: Handle<'a, V>,
        cxf: &mut C,
    ) -> Result<Self, Throw> {
        if value.is_a::<JsDate, _>(cxf) {
            let msg = value.downcast::<JsDate, _>(cxf).unwrap().value(cxf);
            return Ok(JsDataTypes::Date { msg });
        }

        if value.is_a::<JsString, _>(cxf) {
            let msg = value.downcast::<JsString, _>(cxf).unwrap().value(cxf);
            return Ok(JsDataTypes::String { msg });
        }

        if value.is_a::<JsBuffer, _>(cxf) {
            let buffer = value.downcast::<JsBuffer, _>(cxf).unwrap();
            let msg = buffer.as_slice(cxf).to_vec();
            return Ok(JsDataTypes::Buffer { msg });
        }

        if value.is_a::<JsArray, _>(cxf) {
            let json_stringify = cxf
                .global::<JsObject>("JSON")?
                .get_value(cxf, "stringify")?
                .downcast::<JsFunction, _>(cxf)
                .unwrap();

            let out = json_stringify
                .call_with(cxf)
                .arg(value)
                .apply::<JsString, _>(cxf)?
                .value(cxf);

            return Ok(JsDataTypes::Array { msg: out });
        }

        if value.is_a::<neon::types::JsError, _>(cxf) {
            let error = value.downcast::<neon::types::JsError, _>(cxf).unwrap();

            let name = error
                .get_value(cxf, "name")
                .ok()
                .and_then(|n| n.downcast::<JsString, _>(cxf).ok())
                .map(|s| s.value(cxf));

            let message = error
                .get_value(cxf, "message")
                .ok()
                .and_then(|m| m.downcast::<JsString, _>(cxf).ok())
                .map(|s| s.value(cxf));

            let stack = error
                .get_value(cxf, "stack")
                .ok()
                .and_then(|st| st.downcast::<JsString, _>(cxf).ok())
                .map(|s| s.value(cxf));

            return Ok(JsDataTypes::Error {
                name: name.unwrap_or_else(|| "Error".to_string()),
                message: message.unwrap_or_default(),
                stack,
            });
        }

        // Handle objects (including Error detection) here
        if value.is_a::<JsObject, _>(cxf) {
            // Normal object -> JSON stringify
            let json_stringify = cxf
                .global::<JsObject>("JSON")?
                .get_value(cxf, "stringify")?
                .downcast::<JsFunction, _>(cxf)
                .unwrap();

            let out = json_stringify
                .call_with(cxf)
                .arg(value)
                .apply::<JsString, _>(cxf)?
                .value(cxf);

            return Ok(JsDataTypes::Json { msg: out });
        }

        if value.is_a::<JsNumber, _>(cxf) {
            let msg = value.downcast::<JsNumber, _>(cxf).unwrap().value(cxf);
            return Ok(JsDataTypes::Number { msg });
        }

        if value.is_a::<JsBoolean, _>(cxf) {
            let msg = value.downcast::<JsBoolean, _>(cxf).unwrap().value(cxf);
            return Ok(JsDataTypes::Boolean { msg });
        }

        if value.is_a::<JsNull, _>(cxf) {
            return Ok(JsDataTypes::Null);
        }

        if value.is_a::<JsUndefined, _>(cxf) {
            return Ok(JsDataTypes::Undefined);
        }

        Ok(JsDataTypes::Unknown)
    }
}

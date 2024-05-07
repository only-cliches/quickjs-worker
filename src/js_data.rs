use quickjs_runtime::{jsutils::JsError, quickjsrealmadapter::QuickJsRealmAdapter, quickjsvalueadapter::QuickJsValueAdapter};
use neon::{prelude::*, result::Throw, types::JsDate};

#[derive(Debug, Clone)]
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
}

impl JsDataTypes {

    pub fn to_quick_value(&self, realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
        match self {
            JsDataTypes::Date { msg } => {
                let DateFn = realm
                    .get_object_property(
                        &realm.get_global()?,
                        "Date",
                    )?;
                realm
                    .construct_object(
                        &DateFn,
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
            JsDataTypes::Unknown => todo!(),
        }
    }

    pub fn from_quick_value(value: &QuickJsValueAdapter, realm: &QuickJsRealmAdapter) -> Result<Self, JsError> {
        match value.get_js_type() {
            quickjs_runtime::jsutils::JsValueType::I32 => Ok(JsDataTypes::Number { msg: value.to_i32() as f64 }),
            quickjs_runtime::jsutils::JsValueType::F64 => Ok(JsDataTypes::Number { msg: value.to_f64() }),
            quickjs_runtime::jsutils::JsValueType::String => Ok(JsDataTypes::String { msg: value.to_string()? }),
            quickjs_runtime::jsutils::JsValueType::Boolean => Ok(JsDataTypes::Boolean { msg: value.to_bool() }),
            quickjs_runtime::jsutils::JsValueType::Object => {
                let getTime = realm.get_object_property(value, "getTime")?;
                if getTime.is_undefined() {
                    let msg = realm.json_stringify(value, None)?;
                    Ok(JsDataTypes::Json { msg })
                } else {
                    let time = realm.invoke_function(Some(value), &getTime, &[])?;
                    let msg = time.to_f64();
                    Ok(JsDataTypes::Date { msg })
                }
            }
            quickjs_runtime::jsutils::JsValueType::Array => Ok(JsDataTypes::Array { msg: realm.json_stringify(value, None)? }),
            // quickjs_runtime::jsutils::JsValueType::Date => {
            //     let getTime = realm.get_object_property(value, "getTime")?;
            //     let time = realm.invoke_function(None, &getTime, &[])?;
            //     let msg = time.to_f64();
            //     tx.blocking_send(ChannelMsg::SendMessageToNode {
            //         message: MessageTypes::Date { msg },
            //     })
            //     .unwrap();
            // }
            quickjs_runtime::jsutils::JsValueType::Null => Ok(JsDataTypes::Null),
            quickjs_runtime::jsutils::JsValueType::Undefined => Ok(JsDataTypes::Undefined),
            _ => {
                Ok(JsDataTypes::Unknown)
                //return Err(JsError::new(String::from("Uknown Type"), String::from("hello"), String::from("")))
            } 
            // quickjs_runtime::jsutils::JsValueType::Function => todo!(),
            // quickjs_runtime::jsutils::JsValueType::BigInt => todo!(),
            // quickjs_runtime::jsutils::JsValueType::Promise => todo!(),
            // quickjs_runtime::jsutils::JsValueType::Error => todo!(),
        }
    }

    pub fn to_node_value<'a, C: Context<'a>>(&self, cxf: &mut C) -> Result<Handle<'a, JsValue>, Throw> {
        match self {
            JsDataTypes::Unknown => cxf.throw_error("Uknown type!"),
            JsDataTypes::Undefined => Ok(cxf.undefined().as_value(cxf)),
            JsDataTypes::Null => Ok(cxf.null().as_value(cxf)),
            JsDataTypes::String { msg } => Ok(cxf.string(msg).as_value(cxf)),
            JsDataTypes::Json { msg } =>  {
                let jsonParse = cxf
                .global::<JsObject>("JSON")?
                .get_value(cxf, "parse")?
                .downcast::<JsFunction, _>(cxf)
                .unwrap();
                let out = jsonParse
                    .call_with(cxf)
                    .arg(cxf.string(msg))
                    .apply::<JsObject, _>(cxf)?;

                Ok(out.as_value(cxf))
            },
            JsDataTypes::Array { msg } => {
                let jsonParse = cxf
                .global::<JsObject>("JSON")?
                .get_value(cxf, "parse")?
                .downcast::<JsFunction, _>(cxf)
                .unwrap();
                let out = jsonParse
                    .call_with(cxf)
                    .arg(cxf.string(msg))
                    .apply::<JsObject, _>(cxf)?;

                Ok(out.as_value(cxf))
            },
            JsDataTypes::Number { msg } => Ok(cxf.number(*msg).as_value(cxf)),
            JsDataTypes::Boolean { msg } => Ok(cxf.boolean(*msg).as_value(cxf)),
            JsDataTypes::Date { msg } => Ok(cxf.date(*msg).unwrap().as_value(cxf)),
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
        } else if value.is_a::<JsObject, _>(cxf) || value.is_a::<JsArray, _>(cxf) {
            let jsonStringify = cxf
                .global::<JsObject>("JSON")?
                .get_value(cxf, "stringify")?
                .downcast::<JsFunction, _>(cxf)
                .unwrap();
            let msg = jsonStringify
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
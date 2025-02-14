use neon::{context::{Context, FunctionContext}, handle::Handle, result::Throw, types::{JsFunction, JsObject, JsString }};
use neon::object::Object;

use crate::QUICKJS_SENDERS;


pub enum HandleType {
    Eval,
    EvalSync,
    Generate
}

pub fn create_handle<'a>(cx: &'a mut FunctionContext, channel_id: usize, handle_type: HandleType) -> Result<Handle<'a, JsObject>, Throw> {

    let send_fn = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
    let script = cx.argument::<JsString>(0)?.value(cx);
    let (deferred, promise) = cx.promise();
    
    let return_obj = cx.empty_object();

    let handle_id = uuid::Uuid::new_v4().to_string();

    let type_of_fn = JsFunction::new(cx, move |mut cxf: FunctionContext| {
        

        Ok(cxf.undefined())
    })?;
    return_obj.set(cx, "typeOf", type_of_fn)?;

    let keys_fn = JsFunction::new(cx, move |mut cxf: FunctionContext| {
        

        Ok(cxf.undefined())
    })?;
    return_obj.set(cx, "keys", keys_fn)?;


    let get_property_fn = JsFunction::new(cx, move |mut cxf: FunctionContext| {

        
        Ok(cxf.undefined())
    })?;
    return_obj.set(cx, "getProperty", get_property_fn)?;


    let set_property_fn = JsFunction::new(cx, move |mut cxf: FunctionContext| {


        Ok(cxf.undefined())
    })?;
    return_obj.set(cx, "setProperty", set_property_fn)?;


    let get_value_fn = JsFunction::new(cx, move |mut cxf: FunctionContext| {


        Ok(cxf.undefined())
    })?;
    return_obj.set(cx, "getValue", get_value_fn)?;

    let set_value_fn = JsFunction::new(cx, move |mut cxf: FunctionContext| {


        Ok(cxf.undefined())
    })?;
    return_obj.set(cx, "setValue", set_value_fn)?;


    let call_fn = JsFunction::new(cx, move |mut cxf: FunctionContext| {

        Ok(cxf.undefined())
    })?;
    return_obj.set(cx, "call", call_fn)?;


    let close_fn = JsFunction::new(cx, move |mut cxf: FunctionContext| {

        Ok(cxf.undefined())
    })?;
    return_obj.set(cx, "close", close_fn)?;

    Ok(return_obj)
}
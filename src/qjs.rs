use neon::prelude::*;
use neon::types::{Deferred, JsDate};
use quickjs_runtime::builder::QuickJsRuntimeBuilder;
use quickjs_runtime::facades::QuickJsRuntimeFacade;
use quickjs_runtime::jsutils::Script;
use quickjs_runtime::quickjsrealmadapter::QuickJsRealmAdapter;
use quickjs_runtime::quickjsruntimeadapter::QuickJsRuntimeAdapter;
use quickjs_runtime::quickjsvalueadapter::QuickJsValueAdapter;
use quickjs_runtime::values::{JsValueConvertable, JsValueFacade};
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tokio::runtime::Runtime;

use crate::{
    ChannelMsg, MessageTypes, QuickJSOptions, INT_COUNTERS, NODE_CALLBACKS, NODE_CHANNELS,
    QUICKJS_CALLBACKS, QUICKJS_SENDERS,
};

struct CustomModuleLoader {}

impl quickjs_runtime::jsutils::modules::ScriptModuleLoader for CustomModuleLoader {
    fn normalize_path(
        &self,
        realm: &QuickJsRealmAdapter,
        ref_path: &str,
        path: &str,
    ) -> Option<String> {
        let channelID = realm
            .get_object_property(&realm.get_global().unwrap(), "__CHANNELID")
            .unwrap()
            .to_i32() as usize;
        let unlocked = NODE_CHANNELS.blocking_lock();
        let pathStr = String::from(path);
        let refStr = String::from(ref_path);

        if let Some(channel) = unlocked[channelID].clone() {
            return channel
                .send(move |mut cx| {
                    let callbacks = NODE_CALLBACKS.blocking_lock();
                    if let Some(require) = &callbacks[channelID].normalize {
                        let normalizeFn = require.to_inner(&mut cx);
                        let normalizeCall = normalizeFn
                            .call_with(&mut cx)
                            .arg(cx.string(refStr))
                            .arg(cx.string(pathStr))
                            .apply::<JsString, _>(&mut cx);

                        if let Ok(resolvedPath) = normalizeCall {
                            return Ok(resolvedPath.value(&mut cx));
                        }
                    }

                    return cx.throw_error("");
                })
                .join()
                .ok();
        }

        Some(String::from(path))
    }

    fn load_module(&self, realm: &QuickJsRealmAdapter, absolute_path: &str) -> String {
        let channelID = realm
            .get_object_property(&realm.get_global().unwrap(), "__CHANNELID")
            .unwrap()
            .to_i32() as usize;
        let unlocked = NODE_CHANNELS.blocking_lock();
        let pathStr = String::from(absolute_path);

        if let Some(channel) = unlocked[channelID].clone() {
            return channel
                .send(move |mut cx| {
                    let callbacks = NODE_CALLBACKS.blocking_lock();
                    if let Some(require) = &callbacks[channelID].require {
                        let requireFn = require.to_inner(&mut cx);
                        let pathString = cx.string(pathStr.as_str());
                        let moduleSrc = requireFn
                            .call_with(&mut cx)
                            .arg(pathString)
                            .apply::<JsString, _>(&mut cx)?;
                        return Ok(moduleSrc.value(&mut cx));
                    }
                    Ok(String::from("export default ({});"))
                })
                .join()
                .unwrap();
        }

        String::from("export default ({});")
    }
}

pub fn build_runtime(
    opts: &QuickJSOptions,
    channelId: usize,
) -> Arc<tokio::sync::Mutex<QuickJsRuntimeFacade>> {
    let mut rtbuilder = QuickJsRuntimeBuilder::new().script_module_loader(CustomModuleLoader {});

    if let Some(int) = opts.maxInt {
        let mut intCounters = INT_COUNTERS.blocking_lock();
        intCounters[channelId].0 = int as i64;
        if intCounters[channelId].0 > 0 {
            rtbuilder = rtbuilder.set_interrupt_handler(move |rt| {
                let mut intCounters = INT_COUNTERS.blocking_lock();
                intCounters[channelId].1 += 1;
                return intCounters[channelId].1 > intCounters[channelId].0;
            });
        }
    }

    if let Some(max) = opts.maxMemory {
        rtbuilder = rtbuilder.memory_limit(max);
    }
    if let Some(max) = opts.maxStackSize {
        rtbuilder = rtbuilder.max_stack_size(max);
    }
    if let Some(max) = opts.gcThreshold {
        rtbuilder = rtbuilder.gc_threshold(max);
    }
    if let Some(millis) = opts.gcInterval {
        rtbuilder = rtbuilder.gc_interval(std::time::Duration::from_millis(millis));
    }

    return Arc::new(tokio::sync::Mutex::new(rtbuilder.build()));
}

struct QuickJSWorker {}

impl QuickJSWorker {
    pub fn vec_u8_to_NodeArray<'a, C: Context<'a>>(
        vec: &Vec<u8>,
        cx: &mut C,
    ) -> JsResult<'a, JsUint8Array> {
        let a = JsUint8Array::new(cx, vec.len() as usize).unwrap();

        for (i, s) in vec.iter().enumerate() {
            let v = cx.number(*s);
            a.set(cx, i as u32, v)?;
        }

        Ok(a)
    }

    pub async fn process_script_eval(
        cpuTime: cpu_time::ProcessTime,
        startTime: Instant,
        maxEvalTime: Option<u64>,
        channelId: usize,
        script_result: std::result::Result<JsValueFacade, quickjs_runtime::jsutils::JsError>,
        def: Deferred,
        rtClone: Arc<tokio::sync::Mutex<QuickJsRuntimeFacade>>,
    ) {
        let mut maxTimeMs = maxEvalTime.unwrap_or(0);

        if let Err(err) = script_result {
            let unlocked = NODE_CHANNELS.lock().await;
            let channel = unlocked[channelId].clone();
            if let Some(channel) = channel {
                channel
                    .send(move |mut cx| {
                        let msg = cx.string(err.to_string());
                        def.reject(&mut cx, msg);
                        Ok(())
                    })
                    .join()
                    .unwrap();
            }
        } else if let Ok(mut script_result) = script_result {
            let mut promiseErr = (None, None, false);

            if script_result.is_js_promise() {
                while script_result.is_js_promise() {
                    match script_result {
                        JsValueFacade::JsPromise { ref cached_promise } => {
                            let promiseFuture = cached_promise.get_promise_result();

                            if let Some(_) = maxEvalTime {
                                maxTimeMs = maxTimeMs
                                    .saturating_sub(startTime.elapsed().as_millis() as u64);

                                match tokio::time::timeout(
                                    std::time::Duration::from_millis(maxTimeMs),
                                    promiseFuture,
                                )
                                .await
                                {
                                    Ok(done) => match done {
                                        Ok(result) => match result {
                                            Ok(jsValue) => {
                                                script_result = jsValue;
                                            }
                                            Err(e) => {
                                                promiseErr.1 = Some(e);
                                                promiseErr.2 = true;
                                                break;
                                            }
                                        },
                                        Err(e) => {
                                            promiseErr.0 = Some(e);
                                            promiseErr.2 = true;
                                            break;
                                        }
                                    },
                                    Err(timeoutReached) => {
                                        let unlocked = NODE_CHANNELS.lock().await;
                                        let channel = unlocked[channelId].clone();
                                        if let Some(channel) = channel {
                                            channel
                                                .send(move |mut cx| {
                                                    let msg = cx.string("Timeout");
                                                    def.reject(&mut cx, msg);
                                                    Ok(())
                                                })
                                                .join()
                                                .unwrap();
                                        }
                                        return;
                                    }
                                }
                            } else {
                                match promiseFuture.await {
                                    Ok(result) => match result {
                                        Ok(jsValue) => {
                                            script_result = jsValue;
                                        }
                                        Err(e) => {
                                            promiseErr.1 = Some(e);
                                            promiseErr.2 = true;
                                            break;
                                        }
                                    },
                                    Err(e) => {
                                        promiseErr.0 = Some(e);
                                        promiseErr.2 = true;
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            if promiseErr.2 == true {
                let unlocked = NODE_CHANNELS.lock().await;
                let channel = unlocked[channelId].clone();

                if let Some(channel) = channel {
                    channel
                        .send(move |mut cx| {
                            if let Some(e) = promiseErr.1 {
                                // TODO: resolve type and pass, do not stringify
                                let msg = cx.string(e.stringify());
                                def.reject(&mut cx, msg);
                            } else if let Some(e) = promiseErr.0 {
                                let msg = cx.string(e.to_string());
                                def.reject(&mut cx, msg);
                            }
                            Ok(())
                        })
                        .join()
                        .unwrap();
                }
                return;
            }

            // stringify JSON and Arrays
            let (script_result, maybeByteCode, stringified) =
                if script_result.is_js_array() || script_result.is_js_object() {
                    rtClone
                        .lock()
                        .await
                        .loop_async(move |runtime| {
                            let realm = runtime.get_main_realm();
                            let adapter = realm.from_js_value_facade(script_result).unwrap();
                            let string = realm.json_stringify(&adapter, None).unwrap();
                            return (
                                realm.to_js_value_facade(&adapter).unwrap(),
                                Some(realm.to_js_value_facade(&adapter).unwrap()),
                                Some(string),
                            );
                        })
                        .await
                } else {
                    rtClone
                        .lock()
                        .await
                        .loop_async(move |runtime| {
                            let realm = runtime.get_main_realm();
                            let adapter = realm.from_js_value_facade(script_result).unwrap();
                            return (
                                realm.to_js_value_facade(&adapter).unwrap(),
                                Some(realm.to_js_value_facade(&adapter).unwrap()),
                                None,
                            );
                        })
                        .await
                };

            // extract timestamp from Date Object
            // doesn't work right now....
            // let (script_result, dateTime) = if script_result.get_value_type() == quickjs_runtime::jsutils::JsValueType::Date {

            //     rt.loop_realm(None, move |runtime, realm| {
            //         let adapter = realm.from_js_value_facade(script_result).unwrap();
            //         let getTime = realm.get_object_property(&adapter, "getTime").unwrap();
            //         let time = realm.invoke_function(None, &getTime, &[]).unwrap();
            //         return (realm.to_js_value_facade(&adapter).unwrap(), Some(time.to_f64()));
            //     }).await
            // } else { (script_result, None) };

            let unlocked = NODE_CHANNELS.lock().await;
            let channel = unlocked[channelId].clone();

            let totalCPU = cpuTime.elapsed().as_nanos();

            if let Some(channel) = channel {
                channel.send(move |mut cx| {
                    let returnValue = cx.empty_array();
                    let stats = cx.empty_object();
                    let intCounters = INT_COUNTERS.blocking_lock();

                    let interruptCount = if intCounters[channelId].0 > 0 {
                        cx.number(intCounters[channelId].1 as f64)
                    } else {
                        cx.number(-1)
                    };
                    let evalTime =
                        cx.number(startTime.elapsed().as_nanos() as f64 / (1000f64 * 1000f64));
                    let totalCPU = cx.number(totalCPU as f64 / (1000f64 * 1000f64));
                    stats.set(&mut cx, "interrupts", interruptCount)?;
                    stats.set(&mut cx, "evalTimeMs", evalTime)?;
                    stats.set(&mut cx, "cpuTimeMs", totalCPU)?;
                    returnValue.set(&mut cx, 1, stats)?;

                    match script_result.get_value_type() {
                        quickjs_runtime::jsutils::JsValueType::I32 => {
                            let msg = cx.number(script_result.get_i32() as f64);
                            returnValue.set(&mut cx, 0, msg)?;
                            def.resolve(&mut cx, returnValue);
                        }
                        quickjs_runtime::jsutils::JsValueType::F64 => {
                            let msg = cx.number(script_result.get_f64());
                            returnValue.set(&mut cx, 0, msg)?;
                            def.resolve(&mut cx, returnValue);
                        }
                        quickjs_runtime::jsutils::JsValueType::String => {
                            let msg = cx.string(script_result.get_str());
                            returnValue.set(&mut cx, 0, msg)?;
                            def.resolve(&mut cx, returnValue);
                        }
                        quickjs_runtime::jsutils::JsValueType::Boolean => {
                            let msg = cx.boolean(script_result.get_bool());
                            returnValue.set(&mut cx, 0, msg)?;
                            def.resolve(&mut cx, returnValue);
                        }
                        quickjs_runtime::jsutils::JsValueType::Array => {
                            if let Some(json) = stringified {
                                let jsonParse = cx
                                    .global::<JsObject>("JSON")?
                                    .get_value(&mut cx, "parse")?
                                    .downcast::<JsFunction, _>(&mut cx)
                                    .unwrap();
                                let msg = jsonParse
                                    .call_with(&mut cx)
                                    .arg(cx.string(json))
                                    .apply::<JsValue, _>(&mut cx)?;
                                returnValue.set(&mut cx, 0, msg)?;
                                def.resolve(&mut cx, returnValue);
                            }
                        }
                        quickjs_runtime::jsutils::JsValueType::Object => {
                            if let Some(json) = stringified {
                                let jsonParse = cx
                                    .global::<JsObject>("JSON")?
                                    .get_value(&mut cx, "parse")?
                                    .downcast::<JsFunction, _>(&mut cx)
                                    .unwrap();
                                let msg = jsonParse
                                    .call_with(&mut cx)
                                    .arg(cx.string(json))
                                    .apply::<JsValue, _>(&mut cx)?;
                                returnValue.set(&mut cx, 0, msg)?;
                                def.resolve(&mut cx, returnValue);
                            }
                        }
                        // this doesn't work, calls the "object" path every time for some reason
                        // quickjs_runtime::jsutils::JsValueType::Date => {

                        //     if let Some(timestamp) = dateTime {
                        //         let msg = cx.date(timestamp).unwrap();
                        //         def.resolve(&mut cx, msg);
                        //     }
                        // },
                        quickjs_runtime::jsutils::JsValueType::Null => {
                            let msg = cx.null();
                            returnValue.set(&mut cx, 0, msg)?;
                            def.resolve(&mut cx, returnValue);
                        }
                        quickjs_runtime::jsutils::JsValueType::Undefined => {
                            let msg = cx.undefined();
                            returnValue.set(&mut cx, 0, msg)?;
                            def.resolve(&mut cx, returnValue);
                        }
                        _ => {
                            let msg = cx.string(script_result.stringify());
                            returnValue.set(&mut cx, 0, msg)?;
                            def.resolve(&mut cx, returnValue);
                        } // quickjs_runtime::jsutils::JsValueType::Function => todo!(),
                          // quickjs_runtime::jsutils::JsValueType::BigInt => todo!(),
                          // quickjs_runtime::jsutils::JsValueType::Promise => todo!(),
                          // quickjs_runtime::jsutils::JsValueType::Error => todo!(),
                    }

                    Ok(())
                });
            }
        }
    }

    pub fn event_listener(
        rt: &QuickJsRuntimeAdapter,
        realm: &QuickJsRealmAdapter,
        _: &QuickJsValueAdapter,
        args: &[QuickJsValueAdapter],
    ) -> Result<QuickJsValueAdapter, quickjs_runtime::jsutils::JsError> {
        let channelID = realm
            .get_object_property(&realm.get_global()?, "__CHANNELID")?
            .to_i32() as usize;

        {
            // check inturrupt limit
            let intCounters = &mut INT_COUNTERS.blocking_lock();
            let thisCounter = &mut intCounters[channelID];
            thisCounter.1 += 1;
            if thisCounter.0 > 0 && thisCounter.1 > thisCounter.0 {
                return realm.create_error("InternalError", "interrupted", "");
            }
        }

        let callbackType = args[0].to_string()?;
        match callbackType.as_str() {
            "message" => {
                let callbacks = &mut QUICKJS_CALLBACKS.blocking_lock()[channelID];
                let root = realm.to_js_value_facade(&args[1])?;
                callbacks.push(root);
            }
            _ => {}
        }

        Ok(realm.create_undefined().unwrap())
    }

    pub fn post_message(
        rt: &QuickJsRuntimeAdapter,
        realm: &QuickJsRealmAdapter,
        _: &QuickJsValueAdapter,
        args: &[QuickJsValueAdapter],
    ) -> Result<QuickJsValueAdapter, quickjs_runtime::jsutils::JsError> {
        let realm = rt.get_main_realm();

        let channelID = realm
            .get_object_property(&realm.get_global().unwrap(), "__CHANNELID")?
            .to_i32() as usize;

        {
            // check inturrupt limit
            let intCounters = &mut INT_COUNTERS.blocking_lock();
            let thisCounter = &mut intCounters[channelID];
            thisCounter.1 += 1;
            if thisCounter.0 > 0 && thisCounter.1 > thisCounter.0 {
                return realm.create_error("InternalError", "interrupted", "");
            }
        }

        let unlocked = QUICKJS_SENDERS.blocking_lock();
        let tx = unlocked[channelID].clone();

        match args[0].get_js_type() {
            quickjs_runtime::jsutils::JsValueType::I32 => {
                let msg = args[0].to_i32() as f64;
                tx.blocking_send(ChannelMsg::SendMessageToNode {
                    message: MessageTypes::Number { msg },
                })
                .unwrap();
            }
            quickjs_runtime::jsutils::JsValueType::F64 => {
                let msg = args[0].to_f64();
                tx.blocking_send(ChannelMsg::SendMessageToNode {
                    message: MessageTypes::Number { msg },
                })
                .unwrap();
            }
            quickjs_runtime::jsutils::JsValueType::String => {
                let msg = args[0].to_string().unwrap();
                tx.blocking_send(ChannelMsg::SendMessageToNode {
                    message: MessageTypes::String { msg },
                })
                .unwrap();
            }
            quickjs_runtime::jsutils::JsValueType::Boolean => {
                let msg = args[0].to_bool();
                tx.blocking_send(ChannelMsg::SendMessageToNode {
                    message: MessageTypes::Boolean { msg },
                })
                .unwrap();
            }
            quickjs_runtime::jsutils::JsValueType::Object => {
                let msg = realm.json_stringify(&args[0], None)?;
                tx.blocking_send(ChannelMsg::SendMessageToNode {
                    message: MessageTypes::Json { msg },
                })
                .unwrap();
            }
            quickjs_runtime::jsutils::JsValueType::Array => {
                let msg = realm.json_stringify(&args[0], None)?;
                tx.blocking_send(ChannelMsg::SendMessageToNode {
                    message: MessageTypes::Array { msg },
                })
                .unwrap();
            }
            quickjs_runtime::jsutils::JsValueType::Date => {
                let getTime = realm.get_object_property(&args[0], "getTime")?;
                let time = realm.invoke_function(None, &getTime, &[])?;
                let msg = time.to_f64();
                tx.blocking_send(ChannelMsg::SendMessageToNode {
                    message: MessageTypes::Date { msg },
                })
                .unwrap();
            }
            _ => {
                return realm.create_error("Unsupported Message Type", "", "");
            } // quickjs_runtime::jsutils::JsValueType::Function => todo!(),
              // quickjs_runtime::jsutils::JsValueType::BigInt => todo!(),
              // quickjs_runtime::jsutils::JsValueType::Promise => todo!(),
              //
              // quickjs_runtime::jsutils::JsValueType::Null => todo!(),
              // quickjs_runtime::jsutils::JsValueType::Undefined => todo!(),
              // quickjs_runtime::jsutils::JsValueType::Error => todo!(),
        }

        Ok(realm.create_undefined().unwrap())
    }

    pub fn console_log(
        rt: &QuickJsRuntimeAdapter,
        realm: &QuickJsRealmAdapter,
        _: &QuickJsValueAdapter,
        args: &[QuickJsValueAdapter],
    ) -> Result<QuickJsValueAdapter, quickjs_runtime::jsutils::JsError> {
        let realm = rt.get_main_realm();

        let unlockedChannels = NODE_CHANNELS.blocking_lock();
        let channelID = realm
            .get_object_property(&realm.get_global().unwrap(), "__CHANNELID")?
            .to_i32() as usize;

        {
            let intCounters = &mut INT_COUNTERS.blocking_lock();
            let thisCounter = &mut intCounters[channelID];
            thisCounter.1 += 1;
            if thisCounter.0 > 0 && thisCounter.1 > thisCounter.0 {
                return realm.create_error("InternalError", "interrupted", "");
            }
        }

        let channel = unlockedChannels[channelID].clone();

        if let Some(channel) = channel {
            match args[0].get_js_type() {
                quickjs_runtime::jsutils::JsValueType::I32 => {
                    let data = args[0].to_i32() as f64;
                    channel.send(move |mut cx| {
                        let unlockedCallbacks = NODE_CALLBACKS.blocking_lock();
                        let callbacks = &unlockedCallbacks[channelID];
                        if let Some(callback) = &callbacks.console.log {
                            let callbackInner = callback.to_inner(&mut cx);
                            callbackInner
                                .call_with(&mut cx)
                                .arg(cx.number(data))
                                .apply::<JsValue, _>(&mut cx)?;
                        }

                        Ok(())
                    });
                }
                quickjs_runtime::jsutils::JsValueType::F64 => {
                    let data = args[0].to_f64();
                    channel.send(move |mut cx| {
                        let unlockedCallbacks = NODE_CALLBACKS.blocking_lock();
                        let callbacks = &unlockedCallbacks[channelID];
                        if let Some(callback) = &callbacks.console.log {
                            let callbackInner = callback.to_inner(&mut cx);
                            callbackInner
                                .call_with(&mut cx)
                                .arg(cx.number(data))
                                .apply::<JsValue, _>(&mut cx)?;
                        }

                        Ok(())
                    });
                }
                quickjs_runtime::jsutils::JsValueType::String => {
                    let data = args[0].to_string().unwrap();
                    channel.send(move |mut cx| {
                        let unlockedCallbacks = NODE_CALLBACKS.blocking_lock();
                        let callbacks = &unlockedCallbacks[channelID];
                        if let Some(callback) = &callbacks.console.log {
                            let callbackInner = callback.to_inner(&mut cx);
                            callbackInner
                                .call_with(&mut cx)
                                .arg(cx.string(data))
                                .apply::<JsValue, _>(&mut cx)?;
                        }

                        Ok(())
                    });
                }
                quickjs_runtime::jsutils::JsValueType::Boolean => {
                    let data = args[0].to_bool();
                    channel.send(move |mut cx| {
                        let unlockedCallbacks = NODE_CALLBACKS.blocking_lock();
                        let callbacks = &unlockedCallbacks[channelID];
                        if let Some(callback) = &callbacks.console.log {
                            let callbackInner = callback.to_inner(&mut cx);
                            callbackInner
                                .call_with(&mut cx)
                                .arg(cx.boolean(data))
                                .apply::<JsValue, _>(&mut cx)?;
                        }

                        Ok(())
                    });
                }
                quickjs_runtime::jsutils::JsValueType::Object => {
                    let data = realm.json_stringify(&args[0], None).unwrap();
                    channel.send(move |mut cx| {
                        let jsonParse = cx
                            .global::<JsObject>("JSON")?
                            .get_value(&mut cx, "parse")?
                            .downcast::<JsFunction, _>(&mut cx)
                            .unwrap();
                        let out = jsonParse
                            .call_with(&mut cx)
                            .arg(cx.string(data))
                            .apply::<JsObject, _>(&mut cx)?;

                        let unlockedCallbacks = NODE_CALLBACKS.blocking_lock();
                        let callbacks = &unlockedCallbacks[channelID];
                        if let Some(callback) = &callbacks.console.log {
                            let callbackInner = callback.to_inner(&mut cx);
                            callbackInner
                                .call_with(&mut cx)
                                .arg(out)
                                .apply::<JsValue, _>(&mut cx)?;
                        }

                        Ok(())
                    });
                }
                quickjs_runtime::jsutils::JsValueType::Array => {
                    let data = realm.json_stringify(&args[0], None).unwrap();
                    channel.send(move |mut cx| {
                        let jsonParse = cx
                            .global::<JsObject>("JSON")?
                            .get_value(&mut cx, "parse")?
                            .downcast::<JsFunction, _>(&mut cx)
                            .unwrap();
                        let out = jsonParse
                            .call_with(&mut cx)
                            .arg(cx.string(data))
                            .apply::<JsArray, _>(&mut cx)?;

                        let unlockedCallbacks = NODE_CALLBACKS.blocking_lock();
                        let callbacks = &unlockedCallbacks[channelID];
                        if let Some(callback) = &callbacks.console.log {
                            let callbackInner = callback.to_inner(&mut cx);
                            callbackInner
                                .call_with(&mut cx)
                                .arg(out)
                                .apply::<JsValue, _>(&mut cx)?;
                        }

                        Ok(())
                    });
                }
                quickjs_runtime::jsutils::JsValueType::Null => {
                    channel.send(move |mut cx| {
                        let unlockedCallbacks = NODE_CALLBACKS.blocking_lock();
                        let callbacks = &unlockedCallbacks[channelID];
                        if let Some(callback) = &callbacks.console.log {
                            let callbackInner = callback.to_inner(&mut cx);
                            callbackInner
                                .call_with(&mut cx)
                                .arg(cx.null())
                                .apply::<JsValue, _>(&mut cx)?;
                        }

                        Ok(())
                    });
                }
                quickjs_runtime::jsutils::JsValueType::Undefined => {
                    channel.send(move |mut cx| {
                        let unlockedCallbacks = NODE_CALLBACKS.blocking_lock();
                        let callbacks = &unlockedCallbacks[channelID];
                        if let Some(callback) = &callbacks.console.log {
                            let callbackInner = callback.to_inner(&mut cx);
                            callbackInner
                                .call_with(&mut cx)
                                .arg(cx.undefined())
                                .apply::<JsValue, _>(&mut cx)?;
                        }

                        Ok(())
                    });
                }
                quickjs_runtime::jsutils::JsValueType::Date => {
                    let getTime = realm.get_object_property(&args[0], "getTime")?;
                    let time = realm.invoke_function(None, &getTime, &[])?;
                    let msg = time.to_f64();
                    channel.send(move |mut cx| {
                        let unlockedCallbacks = NODE_CALLBACKS.blocking_lock();
                        let callbacks = &unlockedCallbacks[channelID];
                        if let Some(callback) = &callbacks.console.log {
                            let callbackInner = callback.to_inner(&mut cx);
                            callbackInner
                                .call_with(&mut cx)
                                .arg(cx.date(msg).unwrap())
                                .apply::<JsValue, _>(&mut cx)?;
                        }

                        Ok(())
                    });
                }
                _ => {
                    let data = if let Ok(value) = args[0].to_string() {
                        value
                    } else {
                        String::from("Unsupported type")
                    };

                    channel.send(move |mut cx| {
                        let unlockedCallbacks = NODE_CALLBACKS.blocking_lock();
                        let callbacks = &unlockedCallbacks[channelID];
                        if let Some(callback) = &callbacks.console.log {
                            let callbackInner = callback.to_inner(&mut cx);
                            callbackInner
                                .call_with(&mut cx)
                                .arg(cx.string(data))
                                .apply::<JsValue, _>(&mut cx)?;
                        }

                        Ok(())
                    });
                } // quickjs_runtime::jsutils::JsValueType::Function => todo!(),
                  // quickjs_runtime::jsutils::JsValueType::BigInt => todo!(),
                  // quickjs_runtime::jsutils::JsValueType::Promise => todo!(),
                  // quickjs_runtime::jsutils::JsValueType::Error => todo!(),
            }
        }

        Ok(realm.create_undefined().unwrap())
    }
}

pub fn quickjs_thread(
    opts: QuickJSOptions,
    channelId: usize,
    mut urx: tokio::sync::mpsc::Receiver<ChannelMsg>,
) {
    let rt = build_runtime(&opts, channelId);

    let nodeMsgCallbacks: Arc<tokio::sync::Mutex<Vec<Root<JsFunction>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    // QuickJS Runtime Thread
    thread::spawn(move || {
        let tokioRT = Runtime::new().unwrap();

        let msgCBClone = nodeMsgCallbacks.clone();

        tokioRT.block_on(async move {
            let msgCBClone = msgCBClone.clone();

            // Init realm
            rt.lock()
                .await
                .loop_realm(None, move |runtime, realm| {
                    // if enableSetImmediate {
                    //     // quickjs_runtime::features::setimmediate::init(runtime).unwrap();
                    // } else {
                    //     realm.eval(Script::new(".", "setImmediate = () => {};")).unwrap();
                    // }

                    // if enableSetTimeout {
                    //     quickjs_runtime::features::set_timeout::init(runtime).unwrap();
                    // } else {
                    //     realm.delete_object_property(&realm.get_global().unwrap(), "setTimeout").unwrap();
                    //     // realm.install_function(&[], "setTimeout",  |_, realm, _, _| { Ok(realm.create_undefined().unwrap()) }, 2).unwrap();
                    //     // realm.set_object_property(&realm.get_global().unwrap(), "setTimeout", &realm.create_function("setTimeout", |realm, _, _| { Ok(realm.create_undefined().unwrap()) }, 2).unwrap()).unwrap();
                    // }

                    realm
                        .set_object_property(
                            &realm.get_global().unwrap(),
                            "__CHANNELID",
                            &realm.create_i32(channelId as i32).unwrap(),
                        )
                        .unwrap();

                    realm
                        .install_function(&["console"], "log", QuickJSWorker::console_log, 1)
                        .unwrap();

                    realm
                        .install_function(&[], "postMessage", QuickJSWorker::post_message, 1)
                        .unwrap();

                    realm
                        .install_function(&[], "on", QuickJSWorker::event_listener, 2)
                        .unwrap();
                })
                .await;

            while let Some(message) = urx.recv().await {
                // handle quit message
                if let ChannelMsg::Quit { def } = message {
                    let mut unlocked = NODE_CHANNELS.lock().await;

                    if let Some(channel) = &unlocked[channelId] {
                        channel
                            .send(move |mut cx| {
                                let handle = cx.undefined();
                                def.resolve(&mut cx, handle);
                                Ok(())
                            })
                            .join()
                            .unwrap();
                    }

                    unlocked[channelId] = None;
                    break;
                }

                let rtClone = rt.clone();
                let msgCBClone = msgCBClone.clone();

                tokio::spawn(async move {
                    let msgCBClone = msgCBClone.clone();

                    match message {
                        ChannelMsg::GetByteCode { def, source } => {
                            let bytes = rtClone
                                .lock()
                                .await
                                .loop_async(move |runtime| unsafe {
                                    let ctx = runtime.get_main_realm().context;
                                    let compiled =
                                        quickjs_runtime::quickjs_utils::compile::compile(
                                            ctx,
                                            Script::new(".", source.as_str()),
                                        )
                                        .unwrap();
                                    let bytes =
                                        quickjs_runtime::quickjs_utils::compile::to_bytecode(
                                            ctx, &compiled,
                                        );

                                    return bytes;
                                })
                                .await;

                            let unlocked = NODE_CHANNELS.lock().await;
                            let channel = unlocked[channelId].clone();

                            if let Some(channel) = channel {
                                channel
                                    .send(move |mut cx| {
                                        let jsonValue =
                                            QuickJSWorker::vec_u8_to_NodeArray(&bytes, &mut cx)
                                                .unwrap();

                                        def.resolve(&mut cx, jsonValue);

                                        Ok(())
                                    })
                                    .join()
                                    .unwrap();
                            }
                        }
                        ChannelMsg::LoadByteCode { bytes, def } => {
                            rtClone
                                .lock()
                                .await
                                .loop_async(move |runtime| unsafe {
                                    let ctx = runtime.get_main_realm().context;
                                    let out =
                                        quickjs_runtime::quickjs_utils::compile::from_bytecode(
                                            ctx,
                                            bytes.as_slice(),
                                        )
                                        .unwrap();
                                    quickjs_runtime::quickjs_utils::compile::run_compiled_function(
                                        ctx, &out,
                                    )
                                    .unwrap();
                                })
                                .await;

                            let unlocked = NODE_CHANNELS.lock().await;
                            let channel = unlocked[channelId].clone();

                            if let Some(channel) = channel {
                                channel
                                    .send(move |mut cx| {
                                        let val = cx.undefined();
                                        def.resolve(&mut cx, val);

                                        Ok(())
                                    })
                                    .join()
                                    .unwrap();
                            }
                        }
                        ChannelMsg::GarbageCollect { def, start } => {
                            rtClone.lock().await.gc().await;
                            let nodeChannel = &NODE_CHANNELS.lock().await;
                            if let Some(channel) = &nodeChannel[channelId] {
                                channel
                                    .send(move |mut cx| {
                                        let handle = cx.number(start.elapsed().as_millis() as f64);
                                        def.resolve(&mut cx, handle);
                                        Ok(())
                                    })
                                    .join()
                                    .unwrap();
                            }
                        }
                        ChannelMsg::Memory { def } => {
                            rtClone
                                .lock()
                                .await
                                .loop_async(move |runtime| {
                                    let realm = runtime.get_main_realm();
                                    let nodeChannel = &NODE_CHANNELS.blocking_lock()[channelId];

                                    let memory =
                                        serde_json::to_string(&runtime.memory_usage()).unwrap();

                                    if let Some(channel) = nodeChannel {
                                        channel.send(move |mut cx| {
                                            let obj = cx.empty_object();

                                            let jsonParse = cx
                                                .global::<JsObject>("JSON")?
                                                .get_value(&mut cx, "parse")?
                                                .downcast::<JsFunction, _>(&mut cx)
                                                .unwrap();
                                            let out = jsonParse
                                                .call_with(&mut cx)
                                                .arg(cx.string(memory))
                                                .apply::<JsObject, _>(&mut cx)?;

                                            def.resolve(&mut cx, out);
                                            Ok(())
                                        });
                                    }
                                })
                                .await;
                        }
                        ChannelMsg::Quit { def } => {}
                        ChannelMsg::Test {} => {}
                        ChannelMsg::SendMessageToNode { message } => {
                            let cbs = msgCBClone.clone();

                            let unlocked = NODE_CHANNELS.lock().await;
                            let channel = unlocked[channelId].clone();

                            if let Some(channel) = channel {
                                channel.send(move |mut cx| {
                                    let unlocked = cbs.blocking_lock();

                                    match message.clone() {
                                        MessageTypes::Date { msg } => {
                                            let out = cx.date(msg).unwrap();
                                            for cbb in unlocked.iter() {
                                                cbb.to_inner(&mut cx).call_with(&cx)
                                                .arg(out)
                                                .apply::<JsUndefined, TaskContext<'_>>(&mut cx)?;
                                            }
                                        }
                                        MessageTypes::String { msg } => {
                                            let out = cx.string(msg);
                                            for cbb in unlocked.iter() {
                                                cbb.to_inner(&mut cx).call_with(&cx)
                                                .arg(out)
                                                .apply::<JsUndefined, TaskContext<'_>>(&mut cx)?;
                                            }
                                        }
                                        MessageTypes::Json { msg } => {
                                            let jsonParse = cx
                                                .global::<JsObject>("JSON")?
                                                .get_value(&mut cx, "parse")?
                                                .downcast::<JsFunction, _>(&mut cx)
                                                .unwrap();
                                            let out = jsonParse
                                                .call_with(&mut cx)
                                                .arg(cx.string(msg))
                                                .apply::<JsObject, _>(&mut cx)?;

                                            for cbb in unlocked.iter() {
                                                cbb.to_inner(&mut cx).call_with(&cx)
                                                .arg(out)
                                                .apply::<JsUndefined, TaskContext<'_>>(&mut cx)?;
                                            }
                                        }
                                        MessageTypes::Array { msg } => {
                                            let jsonParse = cx
                                                .global::<JsObject>("JSON")?
                                                .get_value(&mut cx, "parse")?
                                                .downcast::<JsFunction, _>(&mut cx)
                                                .unwrap();
                                            let out = jsonParse
                                                .call_with(&mut cx)
                                                .arg(cx.string(msg))
                                                .apply::<JsArray, _>(&mut cx)?;

                                            for cbb in unlocked.iter() {
                                                cbb.to_inner(&mut cx).call_with(&cx)
                                                .arg(out)
                                                .apply::<JsUndefined, TaskContext<'_>>(&mut cx)?;
                                            }
                                        }
                                        MessageTypes::Number { msg } => {
                                            for cbb in unlocked.iter() {
                                                cbb.to_inner(&mut cx)
                                                    .call_with(&cx)
                                                    .arg(cx.number(msg))
                                                    .apply::<JsUndefined, TaskContext<'_>>(
                                                        &mut cx,
                                                    )?;
                                            }
                                        }
                                        MessageTypes::Boolean { msg } => {
                                            for cbb in unlocked.iter() {
                                                cbb.to_inner(&mut cx)
                                                    .call_with(&cx)
                                                    .arg(cx.boolean(msg))
                                                    .apply::<JsUndefined, TaskContext<'_>>(
                                                        &mut cx,
                                                    )?;
                                            }
                                        }
                                    }

                                    Ok(())
                                });
                            }
                        }
                        ChannelMsg::SendMessageToQuick { message } => {
                            rtClone
                                .lock()
                                .await
                                .loop_async(move |runtime| {
                                    let realm = runtime.get_main_realm();

                                    let channelID = realm
                                        .get_object_property(
                                            &realm.get_global().unwrap(),
                                            "__CHANNELID",
                                        )
                                        .unwrap()
                                        .to_i32()
                                        as usize;

                                    let callbacks =
                                        &mut QUICKJS_CALLBACKS.blocking_lock()[channelID];

                                    let mut parsedCallbacks: Vec<QuickJsValueAdapter> = callbacks
                                        .drain(..)
                                        .map(|cb| realm.from_js_value_facade(cb).unwrap())
                                        .collect();

                                    let callbackMsg = match message {
                                        MessageTypes::Date { msg } => {
                                            let DateFn = realm
                                                .get_object_property(
                                                    &realm.get_global().unwrap(),
                                                    "Date",
                                                )
                                                .unwrap();
                                            let dateObj = realm
                                                .construct_object(
                                                    &DateFn,
                                                    &[&realm.create_f64(msg).unwrap()],
                                                )
                                                .unwrap();
                                            realm.to_js_value_facade(&dateObj).unwrap()
                                        }
                                        MessageTypes::String { msg } => msg.to_js_value_facade(),
                                        MessageTypes::Json { msg } => realm
                                            .to_js_value_facade(
                                                &realm.json_parse(msg.as_str()).unwrap(),
                                            )
                                            .unwrap(),
                                        MessageTypes::Array { msg } => realm
                                            .to_js_value_facade(
                                                &realm.json_parse(msg.as_str()).unwrap(),
                                            )
                                            .unwrap(),
                                        MessageTypes::Number { msg } => realm
                                            .to_js_value_facade(&realm.create_f64(msg).unwrap())
                                            .unwrap(),
                                        MessageTypes::Boolean { msg } => realm
                                            .to_js_value_facade(&realm.create_boolean(msg).unwrap())
                                            .unwrap(),
                                    };

                                    let callbackValue =
                                        realm.from_js_value_facade(callbackMsg).unwrap();

                                    for i in 0..parsedCallbacks.len() {
                                        realm
                                            .invoke_function(
                                                None,
                                                &parsedCallbacks[i],
                                                &[&callbackValue],
                                            )
                                            .unwrap();
                                    }

                                    let _ = parsedCallbacks.drain(..).for_each(|cb| {
                                        callbacks.push(realm.to_js_value_facade(&cb).unwrap());
                                    });
                                })
                                .await;
                        }
                        ChannelMsg::NewCallback { root } => {
                            let cbs = msgCBClone.clone();
                            let mut unlocked = cbs.lock().await;
                            unlocked.push(root);
                        }
                        ChannelMsg::EvalScript { source, def } => {
                            {
                                // reset inturupt counter
                                let mut intCounters = INT_COUNTERS.lock().await;
                                intCounters[channelId].1 = 0;
                            }

                            let script_future = rtClone.lock().await.loop_async(move |runtime| {
                                let realm = runtime.get_main_realm();
                                match realm.eval(Script::new(".", &source)) {
                                    Ok(eval_result) => {
                                        return realm.to_js_value_facade(&eval_result);
                                    }
                                    Err(e) => return Err(e),
                                }
                            });

                            let startTime = std::time::Instant::now();

                            let startCPU = cpu_time::ProcessTime::now();

                            if let Some(maxTimeMs) = opts.maxEvalTime {
                                match tokio::time::timeout(
                                    std::time::Duration::from_millis(maxTimeMs),
                                    script_future,
                                )
                                .await
                                {
                                    Err(e) => {
                                        let unlocked = NODE_CHANNELS.lock().await;
                                        let channel = unlocked[channelId].clone();
                                        if let Some(channel) = channel {
                                            channel
                                                .send(move |mut cx| {
                                                    let msg = cx.string(e.to_string());
                                                    def.reject(&mut cx, msg);
                                                    Ok(())
                                                })
                                                .join()
                                                .unwrap();
                                        }
                                        return;
                                    }
                                    Ok(script_result) => {
                                        QuickJSWorker::process_script_eval(
                                            startCPU,
                                            startTime,
                                            opts.maxEvalTime,
                                            channelId,
                                            script_result,
                                            def,
                                            rtClone,
                                        )
                                        .await;
                                    }
                                };
                            } else {
                                let script_result = script_future.await;
                                QuickJSWorker::process_script_eval(
                                    startCPU,
                                    startTime,
                                    opts.maxEvalTime,
                                    channelId,
                                    script_result,
                                    def,
                                    rtClone,
                                )
                                .await;
                            }
                        }
                    }
                });
            }
        });
    });
}

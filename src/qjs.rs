use neon::{prelude::*};
use neon::types::Deferred;
use quickjs_runtime::builder::QuickJsRuntimeBuilder;
use quickjs_runtime::facades::QuickJsRuntimeFacade;
use quickjs_runtime::jsutils::Script;
use quickjs_runtime::quickjsrealmadapter::QuickJsRealmAdapter;
use quickjs_runtime::quickjsruntimeadapter::QuickJsRuntimeAdapter;
use quickjs_runtime::quickjsvalueadapter::QuickJsValueAdapter;
use quickjs_runtime::values::{JsValueConvertable, JsValueFacade};

use std::sync::Arc;
use std::thread;
use std::time::{Instant};
use tokio::runtime::Runtime;

use crate::{
    JsDataTypes, NodeCallbackTypes, QuickChannelMsg, QuickJSOptions, SyncChannelMsg, INT_COUNTERS, NODE_CALLBACKS, NODE_CHANNELS, QUICKJS_CALLBACKS, SYNC_SENDERS
};

macro_rules! quick_js_console {
    ($key:tt, $func_name:ident) => {
        fn $func_name(
            _rt: &QuickJsRuntimeAdapter,
            realm: &QuickJsRealmAdapter,
            _: &QuickJsValueAdapter,
            args: &[QuickJsValueAdapter],
        ) -> Result<QuickJsValueAdapter, quickjs_runtime::jsutils::JsError> {
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

            let jsType = JsDataTypes::from_quick_value(&args[0], realm).unwrap();

            if let Some(channel) = channel {
                channel.send(move |mut cx| {
                    let unlockedCallbacks = NODE_CALLBACKS.blocking_lock();
                    let callbacks = &unlockedCallbacks[channelID];
                    if let Some(callback) =
                        &QuickJSWorker::get_console_by_key($key, callbacks)
                    {
                        let nodeType = jsType.to_node_value(&mut cx)?;
                        let callbackInner = callback.to_inner(&mut cx);
                        callbackInner
                            .call_with(&mut cx)
                            .arg(nodeType)
                            .apply::<JsValue, _>(&mut cx)?;
                    }
    
                    Ok(())
                });
            }

            Ok(realm.create_undefined().unwrap())
        }
    };
}

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

    let mut rtbuilder = QuickJsRuntimeBuilder::new();

    //rtbuilder = rtbuilder.script_module_loader(CustomModuleLoader {});

    if let Some(int) = opts.maxInt {
        let mut intCounters = INT_COUNTERS.blocking_lock();
        intCounters[channelId].0 = int as i64;
        if intCounters[channelId].0 > 0 {
            rtbuilder = rtbuilder.set_interrupt_handler(move |_rt| {
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

pub struct QuickJSWorker {}

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
        cpu_time: cpu_time::ProcessTime,
        start_time: Instant,
        max_eval_time: Option<u64>,
        channel_id: usize,
        script_result: std::result::Result<JsValueFacade, quickjs_runtime::jsutils::JsError>,
        def: Deferred,
        rt_clone: Arc<tokio::sync::Mutex<QuickJsRuntimeFacade>>,
    ) {
        let mut maxTimeMs = max_eval_time.unwrap_or(0);

        if let Err(err) = script_result {

            let syncChannel = SYNC_SENDERS.lock().await;
            syncChannel[channel_id].send(SyncChannelMsg::SendError { e: err.to_string(), def }).await.unwrap();
            return;
        } else if let Ok(mut script_result) = script_result {
            let mut promiseErr = (None, None, false);

            if script_result.is_js_promise() {
                while script_result.is_js_promise() {
                    match script_result {
                        JsValueFacade::JsPromise { ref cached_promise } => {
                            let promiseFuture = cached_promise.get_promise_result();

                            if let Some(_) = max_eval_time {
                                maxTimeMs = maxTimeMs
                                    .saturating_sub(start_time.elapsed().as_millis() as u64);

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
                                        let syncChannel = SYNC_SENDERS.lock().await;
                                        syncChannel[channel_id].send(SyncChannelMsg::SendError { e: timeoutReached.to_string(), def }).await.unwrap();
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

            let syncChannel = SYNC_SENDERS.lock().await;

            if promiseErr.2 == true {
                
                if let Some(e) = promiseErr.1 {
                    // TODO: resolve type and pass, do not stringify
                    syncChannel[channel_id].send(SyncChannelMsg::SendError { e: e.stringify(), def }).await.unwrap();
                } else if let Some(e) = promiseErr.0 {
                    syncChannel[channel_id].send(SyncChannelMsg::SendError { e: e.to_string(), def }).await.unwrap();
                }

                return;
            }

            let jsDataType = rt_clone.lock().await.loop_async(|rt| {
                let realm = rt.get_main_realm();
                return JsDataTypes::from_quick_value(&realm.from_js_value_facade(script_result).unwrap(), realm).unwrap();
            }).await;

            syncChannel[channel_id].send(SyncChannelMsg::RespondEval { result: jsDataType, def, cpu_time, start_time }).await.unwrap();

        }
    }

    pub fn event_listener(
        _rt: &QuickJsRuntimeAdapter,
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
        _rt: &QuickJsRuntimeAdapter,
        realm: &QuickJsRealmAdapter,
        _: &QuickJsValueAdapter,
        args: &[QuickJsValueAdapter],
    ) -> Result<QuickJsValueAdapter, quickjs_runtime::jsutils::JsError> {

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

        let unlocked = SYNC_SENDERS.blocking_lock();
        let tx = &unlocked[channelID];

        let msg = JsDataTypes::from_quick_value(&args[0], realm)?;

        tx.blocking_send(SyncChannelMsg::SendMessageToNode {
            message: msg,
        })
        .unwrap();

        Ok(realm.create_undefined().unwrap())
    }

    pub fn get_console_by_key<'a>(
        key: &str,
        callbacks: &'a crate::NodeCallbacks,
    ) -> &'a Option<Root<JsFunction>> {
        match key {
            "log" => &callbacks.console.log,
            "warn" => &callbacks.console.warn,
            "info" => &callbacks.console.info,
            "error" => &callbacks.console.error,
            _ => &None,
        }
    }
}

pub fn quickjs_thread(
    opts: QuickJSOptions,
    channelId: usize,
    mut urx: tokio::sync::mpsc::Receiver<QuickChannelMsg>,
    mut urx2: tokio::sync::mpsc::Receiver<crate::SyncChannelMsg>
) {
    let rt = build_runtime(&opts, channelId);

    let nodeMsgCallbacks: Arc<tokio::sync::Mutex<Vec<(Root<JsFunction>, NodeCallbackTypes)>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let nodeClone = nodeMsgCallbacks.clone();

    // blocking actions thread
    thread::spawn(move || {

        let msgCBClone = nodeClone.clone();

        loop {

            if let Some(msg) = urx2.blocking_recv() {
                match msg {
                    SyncChannelMsg::RespondEval { def, result, start_time, cpu_time } => {
                        let unlocked = NODE_CHANNELS.blocking_lock();
                        let channel = unlocked[channelId].clone();
            
                        if let Some(channel) = channel {
                            channel.send(move |mut cx| {
            
                                let returnValue = cx.empty_array();
                                let stats = cx.empty_object();
                                let intCounters = INT_COUNTERS.blocking_lock();
    
                                let out = result.to_node_value(&mut cx)?;
                                
                                let interruptCount = if intCounters[channelId].0 > 0 {
                                    cx.number(intCounters[channelId].1 as f64)
                                } else {
                                    cx.number(-1)
                                };
                                let totalCPU = cpu_time.elapsed().as_nanos();
                                let evalTime =
                                    cx.number(start_time.elapsed().as_nanos() as f64 / (1000f64 * 1000f64));
                                let totalCPU = cx.number(totalCPU as f64 / (1000f64 * 1000f64));
                                stats.set(&mut cx, "interrupts", interruptCount)?;
                                stats.set(&mut cx, "evalTimeMs", evalTime)?;
                                stats.set(&mut cx, "cpuTimeMs", totalCPU)?;
                                returnValue.set(&mut cx, 1, stats)?;
                                returnValue.set(&mut cx, 0, out)?;
                                def.resolve(&mut cx, returnValue);
            
                                Ok(())
                            });
                        }
                    }
                    SyncChannelMsg::SendError { e, def } => {
                        let unlocked = NODE_CHANNELS.blocking_lock();
                        let channel = &unlocked[channelId];
                        if let Some(channel) = channel {
                            channel
                                .send(move |mut cx| {
                                    let msg = cx.string(e);
                                    def.reject(&mut cx, msg);
                                    Ok(())
                                })
                                .join()
                                .unwrap();
                        }
                    }
                    SyncChannelMsg::SendMessageToNode { message } => {
                        let cbs = msgCBClone.clone();

                        let unlocked = NODE_CHANNELS.blocking_lock();

                        if let Some(channel) = &unlocked[channelId] {
                            channel.send(move |mut cx| {
                                let unlocked = cbs.blocking_lock();

                                let out = message.to_node_value(&mut cx)?;

                                for cbb in unlocked.iter() {
                                    if cbb.1 == NodeCallbackTypes::Message {
                                        cbb.0
                                            .to_inner(&mut cx)
                                            .call_with(&cx)
                                            .arg(out)
                                            .apply::<JsUndefined, TaskContext<'_>>(
                                                &mut cx,
                                            )?;
                                    }
                                }

                                Ok(())
                            });
                        }
                    }
                    SyncChannelMsg::Memory { json, def } => {

                        let nodeChannel = &NODE_CHANNELS.blocking_lock()[channelId];
                        if let Some(channel) = nodeChannel {
                            channel.send(move |mut cx| {
                                let _obj = cx.empty_object();

                                let jsonParse = cx
                                    .global::<JsObject>("JSON")?
                                    .get_value(&mut cx, "parse")?
                                    .downcast::<JsFunction, _>(&mut cx)
                                    .unwrap();
                                let out = jsonParse
                                    .call_with(&mut cx)
                                    .arg(cx.string(json))
                                    .apply::<JsObject, _>(&mut cx)?;

                                def.resolve(&mut cx, out);
                                Ok(())
                            });
                        }
                    }
                    SyncChannelMsg::LoadBytes { def } => {
                        let unlocked = NODE_CHANNELS.blocking_lock();
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
                    },
                    SyncChannelMsg::GarbageCollect { start, def } => {
                        let nodeChannel = NODE_CHANNELS.blocking_lock();
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
                    },
                    SyncChannelMsg::SendBytes { bytes, def } => {

                        let unlocked = NODE_CHANNELS.blocking_lock();

                        if let Some(channel) = &unlocked[channelId] {
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
                    },
                    SyncChannelMsg::Quit { def } => {
                        let cbs = msgCBClone.clone();

                        let mut unlocked = NODE_CHANNELS.blocking_lock();
    
                        if let Some(channel) = &unlocked[channelId] {
    
                            // complete Promise on Node side
                            channel
                                .send(move |mut cx| {
    
                                    let unlocked_cbs = cbs.try_lock().unwrap();
                                    
                                    for cbb in unlocked_cbs.iter() {
                                        if cbb.1 == NodeCallbackTypes::Close {
                                            cbb.0
                                                .to_inner(&mut cx)
                                                .call_with(&cx)
                                                .apply::<JsUndefined, TaskContext<'_>>(&mut cx)?;
                                        }
                                    }
    
                                    let handle = cx.undefined();
                                    def.resolve(&mut cx, handle);
                                    Ok(())
                                })
                                .join()
                                .unwrap();
                        }
    
                        // clean up memory
                        unlocked[channelId] = None;

                        // stop the thread
                        break;
                    }
                }
            }
        }
    });

    // QuickJS Runtime Thread (async Tokio)
    thread::spawn(move || {
        let tokioRT = Runtime::new().unwrap();

        let msgCBClone = nodeMsgCallbacks.clone();

        tokioRT.block_on(async move {
            let msgCBClone = msgCBClone.clone();

            // Init realm
            rt.lock()
                .await
                .loop_realm(None, move |_runtime, realm| {
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

                    quick_js_console!("log", console_log);
                    realm
                        .install_function(&["console"], "log", console_log, 1)
                        .unwrap();

                    quick_js_console!("info", console_info);
                    realm
                        .install_function(&["console"], "info", console_info, 1)
                        .unwrap();

                    quick_js_console!("warn", console_warn);
                    realm
                        .install_function(&["console"], "warn", console_warn, 1)
                        .unwrap();

                    quick_js_console!("error", console_error);
                    realm
                        .install_function(&["console"], "error", console_error, 1)
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
                if let QuickChannelMsg::Quit { def } = message {

                    let syncChannel = SYNC_SENDERS.lock().await;
                    syncChannel[channelId].send(SyncChannelMsg::Quit { def }).await.unwrap();

                    // stop everything
                    break;
                }

                let rtClone = rt.clone();
                let msgCBClone = msgCBClone.clone();

                tokio::spawn(async move {
                    let msgCBClone = msgCBClone.clone();

                    match message {
                        // ChannelMsg::ProcessAsync { future } => {

                        // },
                        QuickChannelMsg::InitGlobals => {

                            // rtClone
                            //     .lock()
                            //     .await
                            //     .loop_async(move |runtime| {
                            //         let realm = runtime.get_main_realm();

                            //         let callback = move |
                            //             rt: &QuickJsRuntimeAdapter,
                            //             realm: &QuickJsRealmAdapter,
                            //             _: &QuickJsValueAdapter,
                            //             args: &[QuickJsValueAdapter]| -> Result<QuickJsValueAdapter, quickjs_runtime::jsutils::JsError> {
                                            
                            //                 let channelID = realm
                            //                     .get_object_property(&realm.get_global().unwrap(), "__CHANNELID")?
                            //                     .to_i32() as usize;
                            //                 let unlocked = NODE_CHANNELS.blocking_lock();
                            //                 let channel = unlocked[channelID].clone();
                                            
                            //                 let jsTypeArg = JsDataTypes::from_quick_value(&args[0], realm)?;

                            //                 if let Some(channel) = channel {
                            //                     let resultType = channel
                            //                         .send(move |mut cx| {

                            //                             let mut globals = crate::GLOBAL_PTRS.blocking_lock();
                            //                             let globalFns = &globals[channelID];
                            //                             let (key, value) = &globalFns[globalFns.len() - 1];
                            //                             let undefined = cx.undefined();
                            //                             let callArg = jsTypeArg.to_node_value(&mut cx)?;

                            //                             if let GlobalTypes::function { value } = value {
                            //                                 let result = value.to_inner(&mut cx).call_with(&mut cx).arg(callArg).apply::<JsValue, _>(&mut cx)?;
                            //                                 let dataType = JsDataTypes::from_node_value(result, &mut cx)?;
                            //                                 return Ok(dataType);
                            //                             }

                            //                             Ok(JsDataTypes::Unknown)
                                                        
                            //                             // TODO: Handle functions that return a promise
                            //                             // if result.is_a::<JsPromise, _>(&mut cx) {
                            //                             //     let promise = result.downcast::<JsPromise, _>(&mut cx).unwrap();
                            //                             //     let future = promise.to_future(&mut cx, |mut cx, result| {
                            //                             //         Ok(2)
                            //                             //     }).unwrap();
                            //                             //     let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
                            //                             //     let (tx, rx) = oneshot::channel::<()>();
                            //                             //     send_fn.blocking_send(ChannelMsg::ProcessAsync { future, tx }).unwrap();
                            //                             // }

                            //                         })
                            //                         .join()
                            //                         .unwrap();

                            //                     return Ok(resultType.to_quick_value(realm).unwrap())
                            //                 }

                            //             Ok(realm.create_undefined().unwrap())
                            //         };

                            //         let someFn = realm.create_function("something", |
                            //             realm,
                            //             this: &QuickJsValueAdapter,
                            //             args: &[QuickJsValueAdapter]| {

                            //                 Ok(realm.create_undefined().unwrap())
                            //         }, 2).unwrap();

                            //         realm.set_object_property(&realm.get_global().unwrap(), "fnName", &someFn).unwrap();

                            //         // realm.install_closure(&[], name.as_str(), callback, 1).unwrap();

                            //     })
                            //     .await;
                        }
                        QuickChannelMsg::GetByteCode { def, source } => {
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

                            let syncChannel = SYNC_SENDERS.lock().await;
                            syncChannel[channelId].send(SyncChannelMsg::SendBytes { bytes, def }).await.unwrap();

                        }
                        QuickChannelMsg::LoadByteCode { bytes, def } => {
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

                            let syncChannel = SYNC_SENDERS.lock().await;
                            syncChannel[channelId].send(SyncChannelMsg::LoadBytes { def }).await.unwrap();

                        }
                        QuickChannelMsg::GarbageCollect { def, start } => {
                            rtClone.lock().await.gc().await;

                            let syncChannel = SYNC_SENDERS.lock().await;
                            syncChannel[channelId].send(SyncChannelMsg::GarbageCollect { start, def }).await.unwrap();
                        }
                        QuickChannelMsg::Memory { def } => {
                            let json = rtClone
                                .lock()
                                .await
                                .loop_async(move |runtime| {

                                    return serde_json::to_string(&runtime.memory_usage()).unwrap();

                                })
                                .await;

                            let syncChannel = SYNC_SENDERS.lock().await;
                            syncChannel[channelId].send(SyncChannelMsg::Memory { json, def }).await.unwrap();
                        }
                        QuickChannelMsg::Quit { def: _ } => {
                            // handled in upper loop
                        }
                        QuickChannelMsg::SendMessageToQuick { message } => {
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


                                    let callbackValue = message.to_quick_value(realm).unwrap();

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
                        QuickChannelMsg::NewCallback { root, on } => {
                            let cbs = msgCBClone.clone();
                            let mut unlocked = cbs.lock().await;
                            unlocked.push((root, on));
                        }
                        QuickChannelMsg::EvalScript { source, def } => {
                            {
                                // reset inturupt counter
                                let mut intCounters = INT_COUNTERS.lock().await;
                                intCounters[channelId].1 = 0;
                            }

                            let script_future = rtClone.lock().await.loop_async(move |runtime| {
                                let realm = runtime.get_main_realm();
                                match realm.eval(Script::new("./", &source)) {
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
                                        let syncChannel = SYNC_SENDERS.lock().await;
                                        syncChannel[channelId].send(SyncChannelMsg::SendError { e: e.to_string(), def }).await.unwrap();
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

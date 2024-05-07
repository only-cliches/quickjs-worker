use js_data::JsDataTypes;
use lazy_static::lazy_static;
use neon::prelude::*;
use neon::types::{Deferred};





use quickjs_runtime::values::JsValueFacade;


use std::sync::Arc;

use std::time::Instant;

use tokio::sync::mpsc;

pub mod qjs;
pub mod js_data;

#[derive(Default, Debug)]
struct NodeConsole {
    log: Option<Root<JsFunction>>,
    info: Option<Root<JsFunction>>,
    warn: Option<Root<JsFunction>>,
    error: Option<Root<JsFunction>>,
}

#[derive(Default, Debug)]
pub struct NodeCallbacks {
    require: Option<Root<JsFunction>>,
    normalize: Option<Root<JsFunction>>,
    console: NodeConsole,
}

pub enum GlobalTypes {
    function { value: Root<JsFunction> },
    data { value: JsDataTypes }
}

lazy_static! {
    static ref QUICKJS_SENDERS: Arc<tokio::sync::Mutex<Vec<mpsc::Sender<QuickChannelMsg>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    static ref SYNC_SENDERS: Arc<tokio::sync::Mutex<Vec<mpsc::Sender<SyncChannelMsg>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    static ref NODE_CHANNELS: Arc<tokio::sync::Mutex<Vec<Option<Channel>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    static ref NODE_CALLBACKS: Arc<tokio::sync::Mutex<Vec<NodeCallbacks>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    static ref QUICKJS_CALLBACKS: Arc<tokio::sync::Mutex<Vec<Vec<JsValueFacade>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    static ref INT_COUNTERS: Arc<tokio::sync::Mutex<Vec<(i64, i64)>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    static ref GLOBAL_PTRS: Arc<tokio::sync::Mutex<Vec<Vec<(String, GlobalTypes)>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
}

pub enum SyncChannelMsg {
    Quit { def: Deferred },
    SendBytes { bytes: Vec<u8>, def: Deferred },
    LoadBytes { def: Deferred },
    GarbageCollect {
        start: Instant,
        def: Deferred,
    },
    Memory { json: String, def: Deferred },
    SendMessageToNode {
        message: JsDataTypes,
    },
    SendError { e: String, def: Deferred },
    RespondEval { def: Deferred, result: JsDataTypes,         cpu_time: cpu_time::ProcessTime,
        start_time: Instant }
}

#[derive(PartialEq)]
pub enum NodeCallbackTypes {
    Message,
    Close,
}
// #[derive(Debug)]
pub enum QuickChannelMsg {
    Quit {
        def: Deferred,
    },
    // SendMessageToNode {
    //     message: JsDataTypes,
    // },
    SendMessageToQuick {
        message: JsDataTypes,
    },
    // ProcessAsync {
    //     future: JsFuture<i32>
    // },
    InitGlobals,
    NewCallback {
        on: NodeCallbackTypes,
        root: Root<JsFunction>,
    },
    GarbageCollect {
        start: Instant,
        def: Deferred,
    },
    Memory {
        def: Deferred,
    },
    GetByteCode {
        def: Deferred,
        source: String,
    },
    LoadByteCode {
        bytes: Vec<u8>,
        def: Deferred,
    },
    EvalScript {
        def: Deferred,
        source: String,
    },
}



#[derive(Debug, Default, Clone)]
pub struct QuickJSOptions {
    enableSetTimeout: bool,
    enableSetImmediate: bool,
    maxMemory: Option<u64>,
    maxStackSize: Option<u64>,
    gcThreshold: Option<u64>,
    gcInterval: Option<u64>,
    maxInt: Option<u64>,
    maxEvalTime: Option<u64>,
}

fn create_worker(mut cx: FunctionContext) -> JsResult<JsObject> {
    let (channelId, urx, urx2) = {
        let (utx, urx) = mpsc::channel::<QuickChannelMsg>(10);
        let (utx2, urx2) = mpsc::channel::<SyncChannelMsg>(10);
        let mut channelVec = NODE_CHANNELS.blocking_lock();
        let mut senderVec = QUICKJS_SENDERS.blocking_lock();
        let mut callbackVec = NODE_CALLBACKS.blocking_lock();
        let mut callbackVec2 = QUICKJS_CALLBACKS.blocking_lock();
        let mut intCounters = INT_COUNTERS.blocking_lock();
        let mut globals = GLOBAL_PTRS.blocking_lock();
        let mut syncVec = SYNC_SENDERS.blocking_lock();
        let id = channelVec.len();

        channelVec.push(Some(cx.channel()));
        senderVec.push(utx);
        callbackVec.push(NodeCallbacks::default());
        callbackVec2.push(Vec::new());
        intCounters.push((-1, 0));
        globals.push(Vec::new());
        syncVec.push(utx2);

        (id, urx, urx2)
    };

    let mut NodeCallbacks = NodeCallbacks::default();
    let mut consoleStruct = NodeConsole::default();

    let mut Options = QuickJSOptions::default();

    if let Some(args_obj) = cx.argument_opt(0) {
        if let Ok(args_obj) = args_obj.downcast::<JsObject, _>(&mut cx) {
            // handle optional "require" callback
            // if let Ok(value) = args_obj.get_value(&mut cx, "imports") {
            //     if let Ok(callback) = value.downcast::<JsFunction, _>(&mut cx) {
            //         NodeCallbacks.require = Some(callback.root(&mut cx));
            //     }
            // }

            // handle optional "normalize" callback
            // if let Ok(value) = args_obj.get_value(&mut cx, "normalize") {
            //     if let Ok(callback) = value.downcast::<JsFunction, _>(&mut cx) {
            //         NodeCallbacks.normalize = Some(callback.root(&mut cx));
            //     }
            // }

            // if let Ok(value) = args_obj.get_value(&mut cx, "setTimeout") {
            //     if let Ok(boolean) = value.downcast::<JsBoolean, _>(&mut cx) {
            //         if boolean.value(&mut cx) {
            //             Options.enableSetTimeout = true;
            //         }
            //     }
            // }

            // if let Ok(value) = args_obj.get_value(&mut cx, "setImmediate") {
            //     if let Ok(boolean) = value.downcast::<JsBoolean, _>(&mut cx) {
            //         if boolean.value(&mut cx) {
            //             Options.enableSetImmediate = true;
            //         }
            //     }
            // }

            if let Ok(value) = args_obj.get_value(&mut cx, "maxInterrupt") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    Options.maxInt = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "maxMemoryBytes") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    Options.maxMemory = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "maxStackSizeBytes") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    Options.maxStackSize = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "maxEvalMs") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    Options.maxEvalTime = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "gcThresholdAlloc") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    Options.gcThreshold = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "gcIntervalMs") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    Options.gcInterval = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "console") {
                if let Ok(consoleObj) = value.downcast::<JsObject, _>(&mut cx) {
                    if let Ok(log) = consoleObj.get_value(&mut cx, "log") {
                        if let Ok(callback) = log.downcast::<JsFunction, _>(&mut cx) {
                            consoleStruct.log = Some(callback.root(&mut cx));
                        }
                    }
                    if let Ok(log) = consoleObj.get_value(&mut cx, "info") {
                        if let Ok(callback) = log.downcast::<JsFunction, _>(&mut cx) {
                            consoleStruct.info = Some(callback.root(&mut cx));
                        }
                    }
                    if let Ok(log) = consoleObj.get_value(&mut cx, "error") {
                        if let Ok(callback) = log.downcast::<JsFunction, _>(&mut cx) {
                            consoleStruct.error = Some(callback.root(&mut cx));
                        }
                    }
                    if let Ok(log) = consoleObj.get_value(&mut cx, "warn") {
                        if let Ok(callback) = log.downcast::<JsFunction, _>(&mut cx) {
                            consoleStruct.warn = Some(callback.root(&mut cx));
                        }
                    }
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "globals") {
                if let Ok(globals) = value.downcast::<JsObject, _>(&mut cx) {

                    let ObjectKeys = cx
                    .global::<JsFunction>("Object")?
                    .get_value(&mut cx, "keys")?
                    .downcast::<JsFunction, _>(&mut cx)
                    .unwrap();

                    let keys = ObjectKeys
                        .call_with(&mut cx)
                        .arg(globals)
                        .apply::<JsArray, _>(&mut cx)?;

                    let mut globalsArray = GLOBAL_PTRS.blocking_lock();

                    for i in 0..keys.len(&mut cx) {
                        let key = keys.get_value(&mut cx, i)?.downcast::<JsString, _>(&mut cx).unwrap().value(&mut cx);
                        let value = globals.get_value(&mut cx, key.as_str()).unwrap();

                        if let Ok(fnCallback) = value.downcast::<JsFunction, _>(&mut cx) {
                            globalsArray[channelId].push((key.clone(), GlobalTypes::function { value: fnCallback.root(&mut cx) }));
                        } else {
                            globalsArray[channelId].push((key.clone(), GlobalTypes::data { value: JsDataTypes::from_node_value(value, &mut cx)? }));
                        }
                    }

                    let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
                    send_fn
                    .blocking_send(QuickChannelMsg::InitGlobals)
                    .unwrap();

                }
            }
        }
    }

    NodeCallbacks.console = consoleStruct;

    {
        let mut callbackVec = NODE_CALLBACKS.blocking_lock();
        callbackVec[channelId] = NodeCallbacks;
    }

    qjs::quickjs_thread(Options, channelId, urx, urx2);

    // set return object for module
    let return_obj = cx.empty_object();

    let is_closed = Arc::new(std::sync::Mutex::new(false));

    let sendFn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();

        let value = cxf.argument::<JsValue>(0).unwrap();

        let msg = JsDataTypes::from_node_value(value, &mut cxf).unwrap();

        send_fn
        .blocking_send(QuickChannelMsg::SendMessageToQuick {
            message: msg,
        })
        .unwrap();

        return Ok(cxf.undefined());
    })
    .unwrap();

    return_obj.set(&mut cx, "postMessage", sendFn).unwrap();

    let closedClone = is_closed.clone();
    let killFn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        let closed = &mut closedClone.lock().unwrap();
        if **closed == true {
            return cxf.throw_error("Runtime is closed");
        }
        **closed = true;

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
        let (deferred, promise) = cxf.promise();
        send_fn
            .blocking_send(QuickChannelMsg::Quit { def: deferred })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();
    return_obj.set(&mut cx, "close", killFn).unwrap();

    let closedClone = is_closed.clone();
    let is_closed_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        let closed = &mut closedClone.lock().unwrap();
        if **closed == true {
            return Ok(cxf.boolean(true));
        } else {
            return Ok(cxf.boolean(false));
        }
    })
    .unwrap();
    return_obj.set(&mut cx, "isClosed", is_closed_fn).unwrap();

    let closedClone = is_closed.clone();
    let onFn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
        let onType = cxf.argument::<JsString>(0)?.value(&mut cxf);
        match onType.as_str() {
            "message" => {
                if *closedClone.lock().unwrap() == true {
                    return cxf.throw_error("Runtime is closed");
                }
                let cb_fn = cxf.argument::<JsFunction>(1)?.root(&mut cxf);
                send_fn
                    .blocking_send(QuickChannelMsg::NewCallback {
                        on: NodeCallbackTypes::Message,
                        root: cb_fn,
                    })
                    .unwrap();
            }
            "close" => {
                let cb_fn = cxf.argument::<JsFunction>(1)?.root(&mut cxf);
                send_fn
                    .blocking_send(QuickChannelMsg::NewCallback {
                        on: NodeCallbackTypes::Close,
                        root: cb_fn,
                    })
                    .unwrap();
            }
            _ => {}
        }
        return Ok(cxf.undefined());
    })
    .unwrap();
    return_obj.set(&mut cx, "on", onFn).unwrap();

    let closedClone = is_closed.clone();
    let memory_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closedClone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
        let (deferred, promise) = cxf.promise();
        send_fn
            .blocking_send(QuickChannelMsg::Memory { def: deferred })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();
    return_obj.set(&mut cx, "memory", memory_fn).unwrap();

    let closedClone = is_closed.clone();
    let gc_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closedClone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
        let (deferred, promise) = cxf.promise();
        send_fn
            .blocking_send(QuickChannelMsg::GarbageCollect {
                def: deferred,
                start: Instant::now(),
            })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();
    return_obj.set(&mut cx, "gc", gc_fn).unwrap();

    let closedClone = is_closed.clone();
    let eval_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closedClone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
        let script = cxf.argument::<JsString>(0)?.value(&mut cxf);
        let (deferred, promise) = cxf.promise();

        send_fn
            .blocking_send(QuickChannelMsg::EvalScript {
                source: String::from(script),
                def: deferred,
            })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();

    return_obj.set(&mut cx, "eval", eval_fn).unwrap();

    let closedClone = is_closed.clone();
    let to_byte_code = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closedClone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
        let script = cxf.argument::<JsString>(0)?.value(&mut cxf);
        let (deferred, promise) = cxf.promise();

        send_fn
            .blocking_send(QuickChannelMsg::GetByteCode {
                source: String::from(script),
                def: deferred,
            })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();

    return_obj
        .set(&mut cx, "getByteCode", to_byte_code)
        .unwrap();

    let closedClone = is_closed.clone();
    let to_byte_code = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closedClone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
        let bytes = cxf.argument::<JsUint8Array>(0)?;
        let vec_size = bytes.len(&mut cxf);
        let mut buffer: Vec<u8> = Vec::with_capacity(vec_size);

        for i in 0..vec_size {
            let value = bytes
                .get_value(&mut cxf, i as u32)
                .unwrap()
                .downcast::<JsNumber, _>(&mut cxf)
                .unwrap();
            buffer.push(value.value(&mut cxf) as u8);
        }

        let (deferred, promise) = cxf.promise();

        send_fn
            .blocking_send(QuickChannelMsg::LoadByteCode {
                bytes: buffer,
                def: deferred,
            })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();

    return_obj
        .set(&mut cx, "loadByteCode", to_byte_code)
        .unwrap();

    Ok(return_obj)
}

#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    cx.export_function("QuickJSWorker", create_worker)?;

    Ok(())
}

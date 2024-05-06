use lazy_static::lazy_static;
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
use tokio::sync::mpsc;

pub mod qjs;

#[derive(Default, Debug)]
struct NodeConsole {
    log: Option<Root<JsFunction>>,
    info: Option<Root<JsFunction>>,
    warn: Option<Root<JsFunction>>,
    error: Option<Root<JsFunction>>,
}

#[derive(Default, Debug)]
struct NodeCallbacks {
    require: Option<Root<JsFunction>>,
    normalize: Option<Root<JsFunction>>,
    console: NodeConsole,
}

lazy_static! {
    static ref QUICKJS_SENDERS: Arc<tokio::sync::Mutex<Vec<mpsc::Sender<ChannelMsg>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    static ref NODE_CHANNELS: Arc<tokio::sync::Mutex<Vec<Option<Channel>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    static ref NODE_CALLBACKS: Arc<tokio::sync::Mutex<Vec<NodeCallbacks>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    static ref QUICKJS_CALLBACKS: Arc<tokio::sync::Mutex<Vec<Vec<JsValueFacade>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    static ref INT_COUNTERS: Arc<tokio::sync::Mutex<Vec<(i64, i64)>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
}

// #[derive(Debug)]
enum ChannelMsg {
    Quit { def: Deferred },
    Test {},
    SendMessageToNode { message: MessageTypes },
    SendMessageToQuick { message: MessageTypes },
    NewCallback { root: Root<JsFunction> },
    GarbageCollect { start: Instant, def: Deferred },
    Memory { def: Deferred },
    GetByteCode { def: Deferred, source: String },
    LoadByteCode { bytes: Vec<u8>, def: Deferred },
    EvalScript { def: Deferred, source: String },
}

#[derive(Debug, Clone)]
enum MessageTypes {
    String { msg: String },
    Json { msg: String },
    Array { msg: String },
    Number { msg: f64 },
    Boolean { msg: bool },
    Date { msg: f64 },
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
    let (channelId, mut urx) = {
        let (utx, mut urx) = mpsc::channel::<ChannelMsg>(10);
        let mut channelVec = NODE_CHANNELS.blocking_lock();
        let mut senderVec = QUICKJS_SENDERS.blocking_lock();
        let mut callbackVec = NODE_CALLBACKS.blocking_lock();
        let mut callbackVec2 = QUICKJS_CALLBACKS.blocking_lock();
        let mut intCounters = INT_COUNTERS.blocking_lock();
        let id = channelVec.len();

        channelVec.push(Some(cx.channel()));
        senderVec.push(utx);
        callbackVec.push(NodeCallbacks::default());
        callbackVec2.push(Vec::new());
        intCounters.push((-1, 0));

        (id, urx)
    };

    let mut NodeCallbacks = NodeCallbacks::default();
    let mut consoleStruct = NodeConsole::default();

    let mut Options = QuickJSOptions::default();

    if let Some(args_obj) = cx.argument_opt(0) {
        if let Ok(args_obj) = args_obj.downcast::<JsObject, _>(&mut cx) {
            // handle optional "require" callback
            if let Ok(value) = args_obj.get_value(&mut cx, "imports") {
                if let Ok(callback) = value.downcast::<JsFunction, _>(&mut cx) {
                    NodeCallbacks.require = Some(callback.root(&mut cx));
                }
            }

            // handle optional "normalize" callback
            if let Ok(value) = args_obj.get_value(&mut cx, "normalize") {
                if let Ok(callback) = value.downcast::<JsFunction, _>(&mut cx) {
                    NodeCallbacks.normalize = Some(callback.root(&mut cx));
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "setTimeout") {
                if let Ok(boolean) = value.downcast::<JsBoolean, _>(&mut cx) {
                    if boolean.value(&mut cx) {
                        Options.enableSetTimeout = true;
                    }
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "setImmediate") {
                if let Ok(boolean) = value.downcast::<JsBoolean, _>(&mut cx) {
                    if boolean.value(&mut cx) {
                        Options.enableSetImmediate = true;
                    }
                }
            }

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
        }
    }

    NodeCallbacks.console = consoleStruct;

    {
        let mut callbackVec = NODE_CALLBACKS.blocking_lock();
        callbackVec[channelId] = NodeCallbacks;
    }

    qjs::quickjs_thread(Options, channelId, urx);

    // set return object for module
    let obj = cx.empty_object();

    let mut is_closed = Arc::new(std::sync::Mutex::new(false));

    let sendFn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();

        let value = cxf.argument::<JsValue>(0).unwrap();

        if value.is_a::<JsDate, _>(&mut cxf) {
            let msg = value
                .downcast::<JsDate, _>(&mut cxf)
                .unwrap()
                .value(&mut cxf);
            send_fn
                .blocking_send(ChannelMsg::SendMessageToQuick {
                    message: MessageTypes::Date { msg },
                })
                .unwrap();
        } else if value.is_a::<JsString, _>(&mut cxf) {
            let msg = value
                .downcast::<JsString, _>(&mut cxf)
                .unwrap()
                .value(&mut cxf);
            send_fn
                .blocking_send(ChannelMsg::SendMessageToQuick {
                    message: MessageTypes::String { msg },
                })
                .unwrap();
        } else if value.is_a::<JsObject, _>(&mut cxf) || value.is_a::<JsArray, _>(&mut cxf) {
            let jsonStringify = cxf
                .global::<JsObject>("JSON")?
                .get_value(&mut cxf, "stringify")?
                .downcast::<JsFunction, _>(&mut cxf)
                .unwrap();
            let msg = jsonStringify
                .call_with(&mut cxf)
                .arg(value)
                .apply::<JsString, _>(&mut cxf)?
                .value(&mut cxf);
            send_fn
                .blocking_send(ChannelMsg::SendMessageToQuick {
                    message: MessageTypes::Json { msg },
                })
                .unwrap();
        } else if value.is_a::<JsNumber, _>(&mut cxf) {
            let msg = value
                .downcast::<JsNumber, _>(&mut cxf)
                .unwrap()
                .value(&mut cxf);
            send_fn
                .blocking_send(ChannelMsg::SendMessageToQuick {
                    message: MessageTypes::Number { msg },
                })
                .unwrap();
        } else if value.is_a::<JsBoolean, _>(&mut cxf) {
            let msg = value
                .downcast::<JsBoolean, _>(&mut cxf)
                .unwrap()
                .value(&mut cxf);
            send_fn
                .blocking_send(ChannelMsg::SendMessageToQuick {
                    message: MessageTypes::Boolean { msg },
                })
                .unwrap();
        } else {
            return cxf.throw_error("Unsupported data type passed in");
        }

        return Ok(cxf.undefined());
    })
    .unwrap();

    obj.set(&mut cx, "postMessage", sendFn).unwrap();

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
            .blocking_send(ChannelMsg::Quit { def: deferred })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();
    obj.set(&mut cx, "close", killFn).unwrap();

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
    obj.set(&mut cx, "isClosed", is_closed_fn).unwrap();

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
                    .blocking_send(ChannelMsg::NewCallback { root: cb_fn })
                    .unwrap();
            }
            _ => {}
        }
        return Ok(cxf.undefined());
    })
    .unwrap();
    obj.set(&mut cx, "on", onFn).unwrap();

    let closedClone = is_closed.clone();
    let memory_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closedClone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
        let (deferred, promise) = cxf.promise();
        send_fn
            .blocking_send(ChannelMsg::Memory { def: deferred })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();
    obj.set(&mut cx, "memory", memory_fn).unwrap();

    let closedClone = is_closed.clone();
    let gc_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closedClone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
        let (deferred, promise) = cxf.promise();
        send_fn
            .blocking_send(ChannelMsg::GarbageCollect {
                def: deferred,
                start: Instant::now(),
            })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();
    obj.set(&mut cx, "gc", gc_fn).unwrap();

    let closedClone = is_closed.clone();
    let eval_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closedClone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
        let script = cxf.argument::<JsString>(0)?.value(&mut cxf);
        let (deferred, promise) = cxf.promise();

        send_fn
            .blocking_send(ChannelMsg::EvalScript {
                source: String::from(script),
                def: deferred,
            })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();

    obj.set(&mut cx, "eval", eval_fn).unwrap();

    let closedClone = is_closed.clone();
    let to_byte_code = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closedClone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channelId].clone();
        let script = cxf.argument::<JsString>(0)?.value(&mut cxf);
        let (deferred, promise) = cxf.promise();

        send_fn
            .blocking_send(ChannelMsg::GetByteCode {
                source: String::from(script),
                def: deferred,
            })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();

    obj.set(&mut cx, "getByteCode", to_byte_code).unwrap();

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
            .blocking_send(ChannelMsg::LoadByteCode {
                bytes: buffer,
                def: deferred,
            })
            .unwrap();

        return Ok(promise);
    })
    .unwrap();

    obj.set(&mut cx, "loadByteCode", to_byte_code).unwrap();

    Ok(obj)
}

#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    cx.export_function("QuickJSWorker", create_worker)?;

    Ok(())
}

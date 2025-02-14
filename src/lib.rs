use js_data::JsDataTypes;
use lazy_static::lazy_static;
use neon::prelude::*;
use neon::types::Deferred;
use quickjs_runtime::values::JsValueFacade;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

pub mod qjs;
pub mod js_data;
// pub mod handle;

#[derive(Default, Debug)]
struct NodeConsole {
    log: Option<Root<JsFunction>>,
    info: Option<Root<JsFunction>>,
    warn: Option<Root<JsFunction>>,
    error: Option<Root<JsFunction>>,
}

#[derive(Default, Debug)]
pub struct NodeCallbacks {
    // require: Option<Root<JsFunction>>,
    // normalize: Option<Root<JsFunction>>,
    console: NodeConsole,
}

pub enum GlobalTypes {
    Function { value: Root<JsFunction> },
    Data { value: JsDataTypes }
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
    static ref QJS_HANDLES: Arc<tokio::sync::Mutex<Vec<HashMap<String, JsValueFacade>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
}

pub enum SyncChannelMsg {
    ProcessAsync {
        future: neon::types::JsFuture<JsDataTypes>,
        tx: tokio::sync::oneshot::Sender<JsDataTypes>
    },
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
    RespondEval { 
        def: Deferred, 
        result: JsDataTypes,         
        cpu_time: cpu_time::ProcessTime,
        start_time: Instant 
    }
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
    SendMessageToQuick {
        message: JsDataTypes,
    },
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
        target: ScriptEvalType
    },
}

pub enum ScriptEvalType {
    Sync { 
        script_name: String,
        source: String,
        sender: tokio::sync::oneshot::Sender<Result<(JsDataTypes, cpu_time::ProcessTime, Instant), String>>,
        args: Option<Vec<JsDataTypes>>
    },
    Async { 
        script_name: String,
        source: String,
        promise: Deferred,
        args: Option<Vec<JsDataTypes>>
    }
}


#[derive(Debug, Default, Clone)]
pub struct QuickJSOptions {
    max_memory: Option<u64>,
    max_stack_size: Option<u64>,
    gc_threshold: Option<u64>,
    gc_interval: Option<u64>,
    max_int: Option<u64>,
    max_eval_time: Option<u64>,
}

fn create_worker(mut cx: FunctionContext) -> JsResult<JsObject> {

    let mut channel_size = 10usize;
    if let Some(args_obj) = cx.argument_opt(0) {
        if let Ok(args_obj) = args_obj.downcast::<JsObject, _>(&mut cx) {
            if let Ok(value) = args_obj.get_value(&mut cx, "channelSize") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    channel_size = value.value(&mut cx) as usize;
                }
            }
        }
    }

    let (channel_id, urx, urx2) = {
        let (utx, urx) = mpsc::channel::<QuickChannelMsg>(channel_size);
        let (utx2, urx2) = mpsc::channel::<SyncChannelMsg>(channel_size);
        let mut channel_vec = NODE_CHANNELS.blocking_lock();
        let mut sender_vec = QUICKJS_SENDERS.blocking_lock();
        let mut callback_vec = NODE_CALLBACKS.blocking_lock();
        let mut quick_callback_vec = QUICKJS_CALLBACKS.blocking_lock();
        let mut int_counters = INT_COUNTERS.blocking_lock();
        let mut globals = GLOBAL_PTRS.blocking_lock();
        let mut sync_vec = SYNC_SENDERS.blocking_lock();
        let id = channel_vec.len();

        channel_vec.push(Some(cx.channel()));
        sender_vec.push(utx);
        callback_vec.push(NodeCallbacks::default());
        quick_callback_vec.push(Vec::new());
        int_counters.push((-1, 0));
        globals.push(Vec::new());
        sync_vec.push(utx2);

        (id, urx, urx2)
    };

    let mut node_callbacks = NodeCallbacks::default();
    let mut console_struct = NodeConsole::default();

    let mut options = QuickJSOptions::default();

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
                    options.max_int = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "maxMemoryBytes") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    options.max_memory = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "maxStackSizeBytes") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    options.max_stack_size = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "maxEvalMs") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    options.max_eval_time = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "gcThresholdAlloc") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    options.gc_threshold = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "gcIntervalMs") {
                if let Ok(value) = value.downcast::<JsNumber, _>(&mut cx) {
                    options.gc_interval = Some(value.value(&mut cx) as u64);
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "console") {
                if let Ok(console_obj) = value.downcast::<JsObject, _>(&mut cx) {
                    if let Ok(log) = console_obj.get_value(&mut cx, "log") {
                        if let Ok(callback) = log.downcast::<JsFunction, _>(&mut cx) {
                            console_struct.log = Some(callback.root(&mut cx));
                        }
                    }
                    if let Ok(log) = console_obj.get_value(&mut cx, "info") {
                        if let Ok(callback) = log.downcast::<JsFunction, _>(&mut cx) {
                            console_struct.info = Some(callback.root(&mut cx));
                        }
                    }
                    if let Ok(log) = console_obj.get_value(&mut cx, "error") {
                        if let Ok(callback) = log.downcast::<JsFunction, _>(&mut cx) {
                            console_struct.error = Some(callback.root(&mut cx));
                        }
                    }
                    if let Ok(log) = console_obj.get_value(&mut cx, "warn") {
                        if let Ok(callback) = log.downcast::<JsFunction, _>(&mut cx) {
                            console_struct.warn = Some(callback.root(&mut cx));
                        }
                    }
                }
            }

            if let Ok(value) = args_obj.get_value(&mut cx, "staticGlobals") {
                if let Ok(globals) = value.downcast::<JsObject, _>(&mut cx) {

                    let object_keys = cx
                    .global::<JsFunction>("Object")?
                    .get_value(&mut cx, "keys")?
                    .downcast::<JsFunction, _>(&mut cx).unwrap();

                    let keys = object_keys
                        .call_with(&mut cx)
                        .arg(globals)
                        .apply::<JsArray, _>(&mut cx)?;

                    let mut globals_array = GLOBAL_PTRS.blocking_lock();

                    for i in 0..keys.len(&mut cx) {
                        let key = keys.get_value(&mut cx, i)?.downcast::<JsString, _>(&mut cx).unwrap().value(&mut cx);
                        let value = globals.get_value(&mut cx, key.as_str()).unwrap();

                        if let Ok(fn_callback) = value.downcast::<JsFunction, _>(&mut cx) {
                            globals_array[channel_id].push((key.clone(), GlobalTypes::Function { value: fn_callback.root(&mut cx) }));
                        } else {
                            globals_array[channel_id].push((key.clone(), GlobalTypes::Data { value: JsDataTypes::from_node_value(value, &mut cx)? }));
                        }
                    }

                    let send_fn = &QUICKJS_SENDERS.blocking_lock()[channel_id];
                    send_fn
                    .blocking_send(QuickChannelMsg::InitGlobals)
                    .unwrap();
                }
            }
        }
    }

    node_callbacks.console = console_struct;

    {
        let mut callback_vec = NODE_CALLBACKS.blocking_lock();
        callback_vec[channel_id] = node_callbacks;
    }

    qjs::quickjs_thread(options, channel_id, urx, urx2);

    // create return object for js module
    let return_obj = cx.empty_object();

    let is_closed = Arc::new(std::sync::Mutex::new(false));

    let post_message_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        let quick_chnl = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();

        let value = cxf.argument::<JsValue>(0).unwrap();

        let msg = JsDataTypes::from_node_value(value, &mut cxf).unwrap();
        quick_chnl
        .blocking_send(QuickChannelMsg::SendMessageToQuick {
            message: msg,
        }).unwrap_or(());

        return Ok(cxf.undefined());
    })
    .unwrap();

    return_obj.set(&mut cx, "postMessage", post_message_fn).unwrap();

    let close_clone = is_closed.clone();
    let kill_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        let closed = &mut close_clone.lock().unwrap();
        if **closed == true {
            return cxf.throw_error("Runtime is closed");
        }
        **closed = true;

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
        let (deferred, promise) = cxf.promise();
        send_fn
            .blocking_send(QuickChannelMsg::Quit { def: deferred })
            .unwrap_or(());

        return Ok(promise);
    })
    .unwrap();
    return_obj.set(&mut cx, "close", kill_fn).unwrap();

    let closed_clone = is_closed.clone();
    let is_closed_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        let closed = &mut closed_clone.lock().unwrap();
        if **closed == true {
            return Ok(cxf.boolean(true));
        } else {
            return Ok(cxf.boolean(false));
        }
    })
    .unwrap();
    return_obj.set(&mut cx, "isClosed", is_closed_fn).unwrap();

    let closed_clone = is_closed.clone();
    let on_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closed_clone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }
        let send_fn = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
        let on_type = cxf.argument::<JsString>(0)?.value(&mut cxf);
        match on_type.as_str() {
            "message" => {

                let cb_fn = cxf.argument::<JsFunction>(1)?.root(&mut cxf);
                send_fn
                    .blocking_send(QuickChannelMsg::NewCallback {
                        on: NodeCallbackTypes::Message,
                        root: cb_fn,
                    })
                    .unwrap_or(());
            }
            "close" => {
                let cb_fn = cxf.argument::<JsFunction>(1)?.root(&mut cxf);
                send_fn
                    .blocking_send(QuickChannelMsg::NewCallback {
                        on: NodeCallbackTypes::Close,
                        root: cb_fn,
                    })
                    .unwrap_or(());
            }
            _ => {}
        }
        return Ok(cxf.undefined());
    })
    .unwrap();
    return_obj.set(&mut cx, "on", on_fn).unwrap();

    let closed_clone = is_closed.clone();
    let memory_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closed_clone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
        let (deferred, promise) = cxf.promise();
        send_fn
            .blocking_send(QuickChannelMsg::Memory { def: deferred })
            .unwrap_or(());

        return Ok(promise);
    })
    .unwrap();
    return_obj.set(&mut cx, "memory", memory_fn).unwrap();

    let closed_clone = is_closed.clone();
    let gc_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closed_clone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
        let (deferred, promise) = cxf.promise();
        send_fn
            .blocking_send(QuickChannelMsg::GarbageCollect {
                def: deferred,
                start: Instant::now(),
            })
            .unwrap_or(());

        return Ok(promise);
    })
    .unwrap();
    return_obj.set(&mut cx, "gc", gc_fn).unwrap();

    let closed_clone = is_closed.clone();
    let eval_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closed_clone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
        let script = cxf.argument::<JsString>(0)?.value(&mut cxf);
        let (deferred, promise) = cxf.promise();

        let script_name = if let Some(args_obj) = cxf.argument_opt(1) { 
            args_obj.downcast_or_throw::<JsString, _>(&mut cxf)?.value(&mut cxf)
        } else { String::from("Unnamed Script") };

        let mut args: Vec<JsDataTypes> = Vec::new();
        for i in 0..30 {
            if let Some(arg_value) = cxf.argument_opt(i + 2) {
                args.push(JsDataTypes::from_node_value(arg_value, &mut cxf)?);
            }
        }

        send_fn
            .blocking_send(QuickChannelMsg::EvalScript {
                target: ScriptEvalType::Async { promise: deferred, source: String::from(script), script_name, args: if args.len() > 0 { Some(args) } else { None } }
            })
            .unwrap_or(());

        return Ok(promise);
    })
    .unwrap();

    return_obj.set(&mut cx, "eval", eval_fn).unwrap();

    let closed_clone = is_closed.clone();
    let eval_sync_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closed_clone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
        let script = cxf.argument::<JsString>(0)?.value(&mut cxf);

        let script_name = if let Some(args_obj) = cxf.argument_opt(1) { 
            args_obj.downcast_or_throw::<JsString, _>(&mut cxf)?.value(&mut cxf)
        } else { String::from("./") };

        let mut args: Vec<JsDataTypes> = Vec::new();
        for i in 0..30 {
            if let Some(arg_value) = cxf.argument_opt(i + 2) {
                args.push(JsDataTypes::from_node_value(arg_value, &mut cxf)?);
            }
        }

        let (tx, rx) = tokio::sync::oneshot::channel();
        send_fn
            .blocking_send(QuickChannelMsg::EvalScript {
                target: ScriptEvalType::Sync { sender: tx, script_name, source: String::from(script), args: if args.len() > 0 { Some(args) } else { None } }
            })
            .unwrap();

        let result = rx.blocking_recv().unwrap();

        match result {
            Ok((data, cpu_time, start_time)) => {

                let return_value = cxf.empty_array();
                let stats = cxf.empty_object();
                let int_counters = INT_COUNTERS.blocking_lock();

                let out = data.to_node_value(&mut cxf)?;
                
                let interrupt_count = if int_counters[channel_id].0 > 0 {
                    cxf.number(int_counters[channel_id].1 as f64)
                } else {
                    cxf.number(-1)
                };
                let total_cpu = cpu_time.elapsed().as_nanos();
                let eval_time =
                    cxf.number(start_time.elapsed().as_nanos() as f64 / (1000f64 * 1000f64));
                let total_cpu = cxf.number(total_cpu as f64 / (1000f64 * 1000f64));
                stats.set(&mut cxf, "interrupts", interrupt_count)?;
                stats.set(&mut cxf, "evalTimeMs", eval_time)?;
                stats.set(&mut cxf, "cpuTimeMs", total_cpu)?;
                return_value.set(&mut cxf, 1, stats)?;
                return_value.set(&mut cxf, 0, out)?;
                Ok(return_value)
            },
            Err(e) => cxf.throw_error(e)
        }
    })
    .unwrap();

    return_obj.set(&mut cx, "evalSync", eval_sync_fn).unwrap();

    let closed_clone = is_closed.clone();
    let to_byte_code = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closed_clone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
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

    let closed_clone = is_closed.clone();
    let to_byte_code = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closed_clone.clone().lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
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
    }).unwrap();

    return_obj
        .set(&mut cx, "loadByteCode", to_byte_code)?;

    // Handle Object
    // let handle_obj = JsObject::new(&mut cx);

    // let eval_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {

    // })?;

    // handle::create_handle(&mut cxf, channel_id, handle::HandleType::Eval);

    // handle_obj.set(&mut cx, "eval", eval_fn)?;

    // let eval_fn_sync = JsFunction::new(&mut cx, |mut cxf: FunctionContext| {

    //     handle::create_handle(&mut cxf, channel_id, handle::HandleType::EvalSync)
    // })?;

    // handle_obj.set(&mut cx, "evalSync", eval_fn_sync)?;

    // let closed_clone = is_closed.clone();
    // let create_fn = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
    //     if *closed_clone.lock().unwrap() == true {
    //         return cxf.throw_error("Runtime is closed");
    //     }

    //     let send_fn = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
    //     let script = cxf.argument::<JsString>(0)?.value(&mut cxf);

    //     let script_name = String::from("qjs-worker: create_handle.js");

    //     let (tx, rx) = tokio::sync::oneshot::channel();
    //     send_fn
    //         .blocking_send(QuickChannelMsg::EvalScript {
    //             target: ScriptEvalType::Sync { sender: tx, script_name, source: String::from(script), args: None }
    //         })
    //         .unwrap();

    //     let result = rx.blocking_recv().unwrap();

    //     match result {
    //         Ok((data, cpu_time, start_time)) => {

    //             let return_value = cxf.empty_array();
    //             let stats = cxf.empty_object();
    //             let int_counters = INT_COUNTERS.blocking_lock();

    //             let out = data.to_node_value(&mut cxf)?;
                
    //             let interrupt_count = if int_counters[channel_id].0 > 0 {
    //                 cxf.number(int_counters[channel_id].1 as f64)
    //             } else {
    //                 cxf.number(-1)
    //             };
    //             let total_cpu = cpu_time.elapsed().as_nanos();
    //             let eval_time =
    //                 cxf.number(start_time.elapsed().as_nanos() as f64 / (1000f64 * 1000f64));
    //             let total_cpu = cxf.number(total_cpu as f64 / (1000f64 * 1000f64));
    //             stats.set(&mut cxf, "interrupts", interrupt_count)?;
    //             stats.set(&mut cxf, "evalTimeMs", eval_time)?;
    //             stats.set(&mut cxf, "cpuTimeMs", total_cpu)?;
    //             return_value.set(&mut cxf, 1, stats)?;
    //             return_value.set(&mut cxf, 0, out)?;
    //             Ok(return_value)
    //         },
    //         Err(e) => cxf.throw_error(e)
    //     }
        
    // }).unwrap();
    // handle_obj.set(&mut cx, "create", create_fn)?;

    // return_obj
    // .set(&mut cx, "handle", handle_obj)
    // .unwrap();


    Ok(return_obj)
}

#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    cx.export_function("QJSWorker", create_worker)?;

    Ok(())
}

use js_data::JsDataTypes;
use lazy_static::lazy_static;
use neon::prelude::*;
use neon::types::{Deferred, JsDate, JsBuffer, JsNull};
use quickjs_runtime::values::JsValueFacade;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use serde_json::json;

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
    imports: Option<Root<JsFunction>>,
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

pub enum SyncEvalRequest {
    Execute {
        callback_id: usize,
        args: Vec<JsDataTypes>,
        resp: oneshot::Sender<Result<JsDataTypes, String>>
    },
    Console {
        level: String,
        args: Vec<JsDataTypes>,
        resp: oneshot::Sender<()>
    },
    Import {
        module_name: String,
        resp: oneshot::Sender<Result<String, String>>
    },
    Done {
        result: Result<(JsDataTypes, cpu_time::ProcessTime, Instant), String>
    }
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

pub enum QuickChannelMsg {
    Quit {
        def: Deferred,
    },
    SendMessageToQuick {
        message: JsDataTypes,
    },
    InitGlobals {
        structure: String
    },
    SetGlobal {
        key: String,
        structure: String,
        def: Deferred
    },
    ResolveGlobalPromise {
        id: u32,
        result: JsDataTypes,
        error: Option<String>,
    },
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
        is_module: bool,
        sender: tokio::sync::mpsc::Sender<SyncEvalRequest>,
        args: Option<Vec<JsDataTypes>>
    },
    Async { 
        script_name: String,
        source: String,
        is_module: bool,
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

fn process_globals<'a, C: Context<'a>>(
    cx: &mut C, 
    obj: Handle<'a, JsObject>, 
    global_ptrs: &mut Vec<(String, GlobalTypes)>,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let keys = obj.get_own_property_names(cx).unwrap_or(JsArray::new(cx, 0));
    let len = keys.len(cx);
    
    for i in 0..len {
        let key_val: Handle<JsValue> = keys.get(cx, i).unwrap();
        if let Ok(key_str_obj) = key_val.downcast::<JsString, _>(cx) {
            let key_str = key_str_obj.value(cx);
            let val: Handle<JsValue> = obj.get(cx, key_val).unwrap();

            if val.is_a::<JsFunction, _>(cx) {
                 let func = val.downcast::<JsFunction, _>(cx).unwrap();
                 let id = global_ptrs.len();
                 global_ptrs.push((key_str.clone(), GlobalTypes::Function { value: func.root(cx) }));
                 map.insert(key_str, json!({ "__t": "f", "__i": id }));

            } else if val.is_a::<JsObject, _>(cx) 
                && !val.is_a::<JsArray, _>(cx) 
                && !val.is_a::<JsDate, _>(cx) 
                && !val.is_a::<JsNull, _>(cx) 
                && !val.is_a::<JsBuffer, _>(cx)
            {
                 let sub_obj = val.downcast::<JsObject, _>(cx).unwrap();
                 let sub_json = process_globals(cx, sub_obj, global_ptrs);
                 map.insert(key_str, sub_json);

            } else {
                 let data = JsDataTypes::from_node_value(val, cx).unwrap_or(JsDataTypes::Unknown);
                 let id = global_ptrs.len();
                 global_ptrs.push((key_str.clone(), GlobalTypes::Data { value: data }));
                 map.insert(key_str, json!({ "__t": "d", "__i": id }));
            }
        }
    }
    serde_json::Value::Object(map)
}


fn parse_eval_options<'a>(
    cx: &mut FunctionContext<'a>, 
    idx: i32
) -> (String, bool, Vec<JsDataTypes>) {
    let mut script_name = String::from("Unnamed Script");
    let mut is_module = false;
    let mut args: Vec<JsDataTypes> = Vec::new();

    if let Some(arg) = cx.argument_opt(idx as usize) {
        if let Ok(obj_arg) = arg.downcast::<JsObject, _>(cx) {
            // Check for filename/scriptPath
            if let Ok(val) = obj_arg.get_value(cx, "filename") {
                 if let Ok(s) = val.downcast::<JsString, _>(cx) {
                    script_name = s.value(cx);
                 }
            }
            // Check for type: 'module'
            if let Ok(val) = obj_arg.get_value(cx, "type") {
                if let Ok(s) = val.downcast::<JsString, _>(cx) {
                    if s.value(cx) == "module" {
                        is_module = true;
                    }
                }
            }
            // Check for args
            if let Ok(val) = obj_arg.get_value(cx, "args") {
                if let Ok(arr) = val.downcast::<JsArray, _>(cx) {
                    let len = arr.len(cx);
                    for i in 0..len {
                        let item: Handle<JsValue> = arr.get(cx, i).unwrap();
                         if let Ok(dt) = JsDataTypes::from_node_value(item, cx) {
                             args.push(dt);
                         }
                    }
                }
            }
        }
    }
    (script_name, is_module, args)
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

            if let Ok(value) = args_obj.get_value(&mut cx, "imports") {
                if let Ok(callback) = value.downcast::<JsFunction, _>(&mut cx) {
                    node_callbacks.imports = Some(callback.root(&mut cx));
                }
            }

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

            if let Ok(value) = args_obj.get_value(&mut cx, "globals") {
                if let Ok(globals) = value.downcast::<JsObject, _>(&mut cx) {

                    let mut globals_vec = GLOBAL_PTRS.blocking_lock();
                    let target_vec = &mut globals_vec[channel_id];
                    
                    let structure_json = process_globals(&mut cx, globals, target_vec);
                    let structure_str = structure_json.to_string();

                    let send_fn = &QUICKJS_SENDERS.blocking_lock()[channel_id];
                    send_fn
                        .blocking_send(QuickChannelMsg::InitGlobals { structure: structure_str })
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

        let (script_name, is_module, args) = parse_eval_options(&mut cxf, 1);

        send_fn
            .blocking_send(QuickChannelMsg::EvalScript {
                target: ScriptEvalType::Async { 
                    promise: deferred, 
                    source: String::from(script), 
                    script_name, 
                    is_module,
                    args: if args.len() > 0 { Some(args) } else { None } 
                }
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

        let (script_name, is_module, args) = parse_eval_options(&mut cxf, 1);

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        send_fn
            .blocking_send(QuickChannelMsg::EvalScript {
                target: ScriptEvalType::Sync { 
                    sender: tx, 
                    script_name, 
                    source: String::from(script), 
                    is_module,
                    args: if args.len() > 0 { Some(args) } else { None } 
                }
            })
            .unwrap();

        let result = loop {
            if let Some(msg) = rx.blocking_recv() {
                match msg {
                    SyncEvalRequest::Done { result } => break result,
                    
                    SyncEvalRequest::Console { level, args, resp } => {
                         let unlocked_callbacks = NODE_CALLBACKS.blocking_lock();
                         let callbacks = &unlocked_callbacks[channel_id];
                         let callback_opt = match level.as_str() {
                             "log" => &callbacks.console.log,
                             "warn" => &callbacks.console.warn,
                             "info" => &callbacks.console.info,
                             "error" => &callbacks.console.error,
                             _ => &None,
                         };

                         if let Some(root_fn) = callback_opt {
                             let cb_fn = root_fn.to_inner(&mut cxf);
                             let mut fn_handle = cb_fn.call_with(&mut cxf);
                             
                             for arg in args.iter() {
                                 if let Ok(node_val) = arg.to_node_value(&mut cxf) {
                                     fn_handle.arg(node_val);
                                 } else {
                                     fn_handle.arg(cxf.undefined());
                                 }
                             }
                             let _ = fn_handle.apply::<JsValue, _>(&mut cxf);
                         }
                         let _ = resp.send(());
                    },
                    
                    SyncEvalRequest::Import { module_name, resp } => {
                         let unlocked_callbacks = NODE_CALLBACKS.blocking_lock();
                         let callbacks = &unlocked_callbacks[channel_id];
                         
                         if let Some(import_fn) = &callbacks.imports {
                             let cb_fn = import_fn.to_inner(&mut cxf);
                             let mut fn_handle = cb_fn.call_with(&mut cxf);
                             fn_handle.arg(cxf.string(module_name));
                             
                             match fn_handle.apply::<JsValue, _>(&mut cxf) {
                                 Ok(val) => {
                                     if let Ok(str_val) = val.downcast::<JsString, _>(&mut cxf) {
                                         let _ = resp.send(Ok(str_val.value(&mut cxf)));
                                     } else {
                                         let _ = resp.send(Err("Import callback must return a string".to_string()));
                                     }
                                 },
                                 Err(_) => {
                                     let _ = resp.send(Err("Import callback threw an error".to_string()));
                                 }
                             }
                         } else {
                             let _ = resp.send(Err("No import callback defined".to_string()));
                         }
                    },

                    SyncEvalRequest::Execute { callback_id, args, resp } => {
                        let globals = GLOBAL_PTRS.blocking_lock();
                        let (_, value) = &globals[channel_id][callback_id];
                        
                        let execution_result = match value {
                            GlobalTypes::Function { value } => {
                                let mut fn_handle = value.to_inner(&mut cxf).call_with(&mut cxf);
                                for arg in args.iter() {
                                    if let Ok(node_val) = arg.to_node_value(&mut cxf) {
                                        fn_handle.arg(node_val);
                                    } else {
                                        fn_handle.arg(cxf.undefined());
                                    }
                                }
                                match fn_handle.apply::<JsValue, _>(&mut cxf) {
                                    Ok(res) => JsDataTypes::from_node_value(res, &mut cxf).map_err(|_e| "Conversion error".to_string()),
                                    Err(_) => Err("Execution Error".to_string())
                                }
                            },
                            GlobalTypes::Data { value } => Ok(value.clone())
                        };
                        let _ = resp.send(execution_result);
                    }
                }
            } else {
                break Err("Channel closed unexpectedly".to_string());
            }
        };

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
    let set_global = JsFunction::new(&mut cx, move |mut cxf: FunctionContext| {
        if *closed_clone.lock().unwrap() == true {
            return cxf.throw_error("Runtime is closed");
        }

        let send_fn = QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
        let key = cxf.argument::<JsString>(0)?.value(&mut cxf);
        let val_obj = cxf.argument::<JsObject>(1)?;
        let (deferred, promise) = cxf.promise();

        // Use process_globals recursively on this new object
        let mut globals_vec = GLOBAL_PTRS.blocking_lock();
        let target_vec = &mut globals_vec[channel_id];
        let structure_json = process_globals(&mut cxf, val_obj, target_vec);
        let structure_str = structure_json.to_string();

        send_fn
            .blocking_send(QuickChannelMsg::SetGlobal {
                key,
                structure: structure_str,
                def: deferred
            })
            .unwrap();

        return Ok(promise);
    }).unwrap();

    return_obj.set(&mut cx, "setGlobal", set_global).unwrap();

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

    Ok(return_obj)
}

#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    cx.export_function("QJSWorker", create_worker)?;
    Ok(())
}
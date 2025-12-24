use neon::prelude::*;
use quickjs_runtime::builder::QuickJsRuntimeBuilder;
use quickjs_runtime::facades::QuickJsRuntimeFacade;
use quickjs_runtime::jsutils::Script;
use quickjs_runtime::quickjsrealmadapter::QuickJsRealmAdapter;
use quickjs_runtime::quickjsruntimeadapter::QuickJsRuntimeAdapter;
use quickjs_runtime::quickjsvalueadapter::QuickJsValueAdapter;
use quickjs_runtime::values::JsValueFacade;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

use crate::{
    GlobalTypes, JsDataTypes, NodeCallbackTypes, QuickChannelMsg, QuickJSOptions, ScriptEvalType,
    SyncChannelMsg, INT_COUNTERS, NODE_CALLBACKS, NODE_CHANNELS, QUICKJS_CALLBACKS, SYNC_SENDERS,
};

lazy_static::lazy_static! {
    static ref PENDING_GLOBAL_PROMISES: Arc<tokio::sync::Mutex<HashMap<u32, (JsValueFacade, JsValueFacade)>>> = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    static ref GLOBAL_PROMISE_ID: AtomicU32 = AtomicU32::new(0);
    static ref ACTIVE_SYNC_REQUESTS: Arc<tokio::sync::Mutex<HashMap<usize, tokio::sync::mpsc::Sender<crate::SyncEvalRequest>>>> = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
}

macro_rules! quick_js_console {
    ($key:tt, $func_name:ident) => {
        fn $func_name(
            _rt: &QuickJsRuntimeAdapter,
            realm: &QuickJsRealmAdapter,
            _: &QuickJsValueAdapter,
            args: &[QuickJsValueAdapter],
        ) -> Result<QuickJsValueAdapter, quickjs_runtime::jsutils::JsError> {
            let unlocked_channels = NODE_CHANNELS.blocking_lock();
            let channel_id = realm.id.parse::<usize>().unwrap();
            {
                let int_counters = &mut INT_COUNTERS.blocking_lock();
                let this_counter = &mut int_counters[channel_id];
                this_counter.1 += 1;
                if this_counter.0 > 0 && this_counter.1 > this_counter.0 {
                    return realm.create_error("InternalError", "interrupted", "");
                }
            }
            let mut js_args: Vec<JsDataTypes> = Vec::new();
            for i in 0..args.len() {
                js_args.push(JsDataTypes::from_quick_value(&args[i], realm).unwrap());
            }
            let sync_sender_opt = {
                let map = ACTIVE_SYNC_REQUESTS.blocking_lock();
                map.get(&channel_id).cloned()
            };
            if let Some(sync_sender) = sync_sender_opt {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let req = crate::SyncEvalRequest::Console {
                    level: String::from($key),
                    args: js_args,
                    resp: tx,
                };
                if let Ok(_) = sync_sender.blocking_send(req) {
                    let _ = rx.blocking_recv();
                }
            } else {
                let channel = unlocked_channels[channel_id].clone();
                if let Some(channel) = channel {
                    channel.send(move |mut cx| {
                        let unlocked_callbacks = NODE_CALLBACKS.blocking_lock();
                        let callbacks = &unlocked_callbacks[channel_id];
                        if let Some(callback) = &QuickJSWorker::get_console_by_key($key, callbacks)
                        {
                            let callback_inner = callback.to_inner(&mut cx);
                            let mut fn_handle = callback_inner.call_with(&mut cx);
                            for arg in js_args.iter() {
                                fn_handle.arg(arg.to_node_value(&mut cx)?);
                            }
                            fn_handle.apply::<JsValue, _>(&mut cx)?;
                        }
                        Ok(())
                    });
                }
            }
            Ok(realm.create_undefined().unwrap())
        }
    };
}

struct CustomModuleLoader {
    channel_id: usize,
}

impl quickjs_runtime::jsutils::modules::ScriptModuleLoader for CustomModuleLoader {
    fn normalize_path(
        &self,
        _realm: &QuickJsRealmAdapter,
        _ref_path: &str,
        path: &str,
    ) -> Option<String> {
        Some(String::from(path))
    }
    fn load_module(&self, _realm: &QuickJsRealmAdapter, absolute_path: &str) -> String {
        let channel_id = self.channel_id;
        let path_str = String::from(absolute_path);
        let sync_sender_opt = {
            let map = ACTIVE_SYNC_REQUESTS.blocking_lock();
            map.get(&channel_id).cloned()
        };
        if let Some(sync_sender) = sync_sender_opt {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let req = crate::SyncEvalRequest::Import {
                module_name: path_str,
                resp: tx,
            };
            if let Ok(_) = sync_sender.blocking_send(req) {
                match rx.blocking_recv() {
                    Ok(Ok(source)) => return source,
                    Ok(Err(e)) => return format!("throw new Error('Import Error: {}');", e),
                    Err(_) => return String::from("throw new Error('Internal Import Error');"),
                }
            }
        } else {
            let unlocked = NODE_CHANNELS.blocking_lock();
            if let Some(channel) = unlocked[channel_id].clone() {
                let (tx, rx) = std::sync::mpsc::channel();
                channel.send(move |mut cx| {
                    let callbacks = NODE_CALLBACKS.blocking_lock();
                    let result = if let Some(import_cb) = &callbacks[channel_id].imports {
                        let import_fn = import_cb.to_inner(&mut cx);
                        let call = import_fn
                            .call_with(&mut cx)
                            .arg(cx.string(path_str))
                            .apply::<JsValue, _>(&mut cx);
                        match call {
                            Ok(val) => {
                                if let Ok(s) = val.downcast::<JsString, _>(&mut cx) {
                                    Ok(s.value(&mut cx))
                                } else {
                                    Err(String::from("Callback did not return string"))
                                }
                            }
                            Err(_) => Err(String::from("Callback threw error")),
                        }
                    } else {
                        Err(String::from("Imports not supported"))
                    };
                    let _ = tx.send(result);
                    Ok(())
                });
                match rx.recv() {
                    Ok(Ok(source)) => return source,
                    Ok(Err(e)) => return format!("throw new Error('Import Error: {}');", e),
                    _ => return String::from("throw new Error('Import Communication Error');"),
                }
            }
        }
        String::from("throw new Error('Module loader failed');")
    }
}

fn generate_init_script(structure: &serde_json::Value, path: String) -> String {
    let mut script = String::new();
    if let Some(obj) = structure.as_object() {
        for (key, val) in obj {
            let current_access = if path.is_empty() {
                format!("const {}", key) // This should generally not be reached if called recursively correctly
            } else {
                format!("{}.{}", path, key)
            };
            if let Some(inner_obj) = val.as_object() {
                if inner_obj.contains_key("__t") {
                    let id = inner_obj["__i"].as_u64().unwrap();
                    let type_str = inner_obj["__t"].as_str().unwrap();
                    if type_str == "f" {
                        script.push_str(&format!("{} = function(...args) {{ return __INTERNAL_CALL_GLOBAL({}, args); }};\n", current_access, id));
                    } else {
                        script.push_str(&format!(
                            "{} = __INTERNAL_CALL_GLOBAL({}, []);\n",
                            current_access, id
                        ));
                    }
                } else {
                    if path.is_empty() {
                        // Top level object creation (handled in SetGlobal mostly, but kept for InitGlobals)
                        script.push_str(&format!(
                            "if (typeof {} === 'undefined') var {} = {{}};\n",
                            key, key
                        ));
                        script.push_str(&generate_init_script(val, key.clone()));
                    } else {
                        script.push_str(&format!("{} = {{}};\n", current_access));
                        script.push_str(&generate_init_script(val, format!("{}.{}", path, key)));
                    }
                }
            }
        }
    }
    script
}

pub fn build_runtime(
    opts: &QuickJSOptions,
    channel_id: usize,
) -> Arc<tokio::sync::Mutex<QuickJsRuntimeFacade>> {
    let mut rtbuilder = QuickJsRuntimeBuilder::new();
    rtbuilder = rtbuilder.script_module_loader(CustomModuleLoader { channel_id });

    if opts.max_int.is_some() || opts.max_eval_time.is_some() {
        // Ensure default instruction count is set if only time is used, to ensure callback triggers
        let max_int_val = opts.max_int.unwrap_or(200000) as i64;

        let mut int_counters = INT_COUNTERS.blocking_lock();
        int_counters[channel_id].0 = max_int_val;

        rtbuilder = rtbuilder.set_interrupt_handler(move |_rt| {
            let mut int_counters = INT_COUNTERS.blocking_lock();
            let counters = &mut int_counters[channel_id];

            // Instruction Count
            counters.1 += 1;
            if counters.0 > 0 && counters.1 > counters.0 {
                return true;
            }

            // Time Deadline Check (counters.2 is deadline timestamp in millis)
            if counters.2 > 0 {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                if now > counters.2 {
                    return true;
                }
            }
            return false;
        });
    }

    if let Some(max) = opts.max_memory {
        rtbuilder = rtbuilder.memory_limit(max);
    }
    if let Some(max) = opts.max_stack_size {
        rtbuilder = rtbuilder.max_stack_size(max);
    }
    if let Some(max) = opts.gc_threshold {
        rtbuilder = rtbuilder.gc_threshold(max);
    }
    if let Some(millis) = opts.gc_interval {
        rtbuilder = rtbuilder.gc_interval(std::time::Duration::from_millis(millis));
    }

    return Arc::new(tokio::sync::Mutex::new(rtbuilder.build()));
}

pub struct QuickJSWorker {}

impl QuickJSWorker {
    pub fn vec_u8_to_node_array<'a, C: Context<'a>>(
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
        realm_id: String,
        start_time: Instant,
        max_eval_time: Option<u64>,
        script_result: std::result::Result<JsValueFacade, quickjs_runtime::jsutils::JsError>,
        rt_clone: Arc<tokio::sync::Mutex<QuickJsRuntimeFacade>>,
        is_module: bool,
    ) -> Result<JsDataTypes, String> {
        let mut max_time_ms = max_eval_time.unwrap_or(0);
        match script_result {
            Err(err) => return Err(err.to_string()),
            Ok(mut script_result) => {
                let mut promise_err = (None, None, false);
                if script_result.is_js_promise() {
                    while script_result.is_js_promise() {
                        match script_result {
                            JsValueFacade::JsPromise { ref cached_promise } => {
                                let promise_future = cached_promise.get_promise_result();
                                if let Some(_) = max_eval_time {
                                    max_time_ms = max_time_ms
                                        .saturating_sub(start_time.elapsed().as_millis() as u64);
                                    match tokio::time::timeout(
                                        std::time::Duration::from_millis(max_time_ms),
                                        promise_future,
                                    )
                                    .await
                                    {
                                        Ok(done) => match done {
                                            Ok(result) => match result {
                                                Ok(js_value) => {
                                                    script_result = js_value;
                                                }
                                                Err(e) => {
                                                    promise_err.1 = Some(e);
                                                    promise_err.2 = true;
                                                    break;
                                                }
                                            },
                                            Err(e) => {
                                                promise_err.0 = Some(e);
                                                promise_err.2 = true;
                                                break;
                                            }
                                        },
                                        Err(timeout_reached) => {
                                            return Err(timeout_reached.to_string());
                                        }
                                    }
                                } else {
                                    match promise_future.await {
                                        Ok(result) => match result {
                                            Ok(js_value) => {
                                                script_result = js_value;
                                            }
                                            Err(e) => {
                                                promise_err.1 = Some(e);
                                                promise_err.2 = true;
                                                break;
                                            }
                                        },
                                        Err(e) => {
                                            promise_err.0 = Some(e);
                                            promise_err.2 = true;
                                            break;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // CRITICAL FIX: Explicitly return error if promise rejected
                if promise_err.2 {
                    if let Some(e) = promise_err.1 {
                        return Err(e.stringify());
                    } else if let Some(e) = promise_err.0 {
                        return Err(e.to_string());
                    }
                    return Err(String::from("Unknown promise error"));
                }

                let rt_id = realm_id.clone();
                let js_data_type = rt_clone
                    .lock()
                    .await
                    .loop_async(move |rt| {
                        let realm = rt.get_realm(&rt_id).unwrap();
                        if is_module {
                            if let Ok(ret_val) = realm.get_global().and_then(|g| {
                                realm.get_object_property(&g, "__qjs_module_return_value")
                            }) {
                                if !ret_val.is_undefined() {
                                    let _ = realm.get_global().and_then(|g| {
                                        realm.set_object_property(
                                            &g,
                                            "__qjs_module_return_value",
                                            &realm.create_undefined().unwrap(),
                                        )
                                    });
                                    return JsDataTypes::from_quick_value(&ret_val, realm).unwrap();
                                }
                            }
                        }
                        return JsDataTypes::from_quick_value(
                            &realm.from_js_value_facade(script_result).unwrap(),
                            realm,
                        )
                        .unwrap();
                    })
                    .await;
                return Ok(js_data_type);
            }
        }
    }

    pub fn event_listener(
        _rt: &QuickJsRuntimeAdapter,
        realm: &QuickJsRealmAdapter,
        _: &QuickJsValueAdapter,
        args: &[QuickJsValueAdapter],
    ) -> Result<QuickJsValueAdapter, quickjs_runtime::jsutils::JsError> {
        let channel_id = realm.id.parse::<usize>().unwrap();
        {
            let int_counters = &mut INT_COUNTERS.blocking_lock();
            let this_counter = &mut int_counters[channel_id];
            this_counter.1 += 1;
            if this_counter.0 > 0 && this_counter.1 > this_counter.0 {
                return realm.create_error("InternalError", "interrupted", "");
            }
        }
        let callback_type = args[0].to_string()?;
        match callback_type.as_str() {
            "message" => {
                let callbacks = &mut QUICKJS_CALLBACKS.blocking_lock()[channel_id];
                if callbacks.len() > 99 {
                    return realm.create_error("Reached callback limit of 100", "", "");
                }
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
        let channel_id = realm.id.parse::<usize>().unwrap();
        {
            let int_counters = &mut INT_COUNTERS.blocking_lock();
            let this_counter = &mut int_counters[channel_id];
            this_counter.1 += 1;
            if this_counter.0 > 0 && this_counter.1 > this_counter.0 {
                return realm.create_error("InternalError", "interrupted", "");
            }
        }
        let msg = JsDataTypes::from_quick_value(&args[0], realm)?;
        let unlocked = SYNC_SENDERS.blocking_lock();
        let tx = &unlocked[channel_id];
        match tx.blocking_send(SyncChannelMsg::SendMessageToNode { message: msg }) {
            Ok(_) => Ok(realm.create_undefined().unwrap()),
            Err(_) => realm.create_error("Message Queue Full", "", ""),
        }
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
    channel_id: usize,
    mut urx: tokio::sync::mpsc::Receiver<QuickChannelMsg>,
    mut urx2: tokio::sync::mpsc::Receiver<crate::SyncChannelMsg>,
) {
    let rt = build_runtime(&opts, channel_id);
    let node_msg_callbacks: Arc<tokio::sync::Mutex<Vec<(Root<JsFunction>, NodeCallbackTypes)>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let node_clone = node_msg_callbacks.clone();

    thread::spawn(move || {
        let msg_cb_clone = node_clone.clone();
        let tokio_rt = tokio::runtime::Runtime::new().unwrap();
        loop {
            if let Some(msg) = urx2.blocking_recv() {
                match msg {
                    SyncChannelMsg::ProcessAsync { future, tx } => {
                        match tokio_rt.block_on(future) {
                            Ok(result) => {
                                tx.send(result).unwrap();
                            }
                            Err(_e) => {
                                tx.send(JsDataTypes::Undefined).unwrap();
                            }
                        }
                    }
                    SyncChannelMsg::RespondEval {
                        def,
                        result,
                        start_time,
                        cpu_time,
                    } => {
                        let unlocked = NODE_CHANNELS.blocking_lock();
                        let channel = unlocked[channel_id].clone();
                        if let Some(channel) = channel {
                            channel.send(move |mut cx| {
                                let return_value = cx.empty_array();
                                let stats = cx.empty_object();
                                let int_counters = INT_COUNTERS.blocking_lock();
                                let out = result.to_node_value(&mut cx)?;
                                let interrupt_count = if int_counters[channel_id].0 > 0 {
                                    cx.number(int_counters[channel_id].1 as f64)
                                } else {
                                    cx.number(-1)
                                };
                                let total_cpu = cpu_time.elapsed().as_nanos();
                                let eval_time = cx.number(
                                    start_time.elapsed().as_nanos() as f64 / (1000f64 * 1000f64),
                                );
                                let total_cpu = cx.number(total_cpu as f64 / (1000f64 * 1000f64));
                                stats.set(&mut cx, "interrupts", interrupt_count)?;
                                stats.set(&mut cx, "evalTimeMs", eval_time)?;
                                stats.set(&mut cx, "cpuTimeMs", total_cpu)?;
                                return_value.set(&mut cx, 1, stats)?;
                                return_value.set(&mut cx, 0, out)?;
                                def.resolve(&mut cx, return_value);
                                Ok(())
                            });
                        }
                    }
                    SyncChannelMsg::SendError { e, def } => {
                        let unlocked = NODE_CHANNELS.blocking_lock();
                        let channel = &unlocked[channel_id];
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
                        let cbs = msg_cb_clone.clone();
                        let unlocked = NODE_CHANNELS.blocking_lock();
                        if let Some(channel) = &unlocked[channel_id] {
                            channel.send(move |mut cx| {
                                let unlocked = cbs.blocking_lock();
                                let out = message.to_node_value(&mut cx)?;
                                for cbb in unlocked.iter() {
                                    if cbb.1 == NodeCallbackTypes::Message {
                                        let _ = cbb
                                            .0
                                            .to_inner(&mut cx)
                                            .call_with(&mut cx)
                                            .arg(out)
                                            .apply::<JsValue, TaskContext<'_>>(&mut cx)?;
                                    }
                                }
                                Ok(())
                            });
                        }
                    }
                    SyncChannelMsg::Memory { json, def } => {
                        let node_channel = &NODE_CHANNELS.blocking_lock()[channel_id];
                        if let Some(channel) = node_channel {
                            channel.send(move |mut cx| {
                                let json_parse = cx
                                    .global::<JsObject>("JSON")?
                                    .get_value(&mut cx, "parse")?
                                    .downcast::<JsFunction, _>(&mut cx)
                                    .unwrap();
                                let out = json_parse
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
                        let channel = unlocked[channel_id].clone();
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
                    SyncChannelMsg::GarbageCollect { start, def } => {
                        let node_channels = NODE_CHANNELS.blocking_lock();
                        if let Some(channel) = &node_channels[channel_id] {
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
                    SyncChannelMsg::SendBytes { bytes, def } => {
                        let unlocked = NODE_CHANNELS.blocking_lock();
                        if let Some(channel) = &unlocked[channel_id] {
                            channel
                                .send(move |mut cx| {
                                    let json_value =
                                        QuickJSWorker::vec_u8_to_node_array(&bytes, &mut cx)
                                            .unwrap();
                                    def.resolve(&mut cx, json_value);
                                    Ok(())
                                })
                                .join()
                                .unwrap();
                        }
                    }
                    SyncChannelMsg::Quit { def } => {
                        let cbs = msg_cb_clone.clone();
                        // 1. Acquire lock to get the channel
                        let unlocked = NODE_CHANNELS.blocking_lock();

                        if let Some(channel) = &unlocked[channel_id] {
                            // 2. Send the cleanup task to the Node Main Thread
                            channel.send(move |mut cx| {
                                // A. Run user-defined 'close' listeners (e.g. qjs.on('close'))
                                // Use a scope to ensure lock is dropped before we clean up
                                {
                                    let unlocked_cbs = cbs.try_lock().unwrap();
                                    for cbb in unlocked_cbs.iter() {
                                        if cbb.1 == NodeCallbackTypes::Close {
                                            let _ = cbb
                                                .0
                                                .to_inner(&mut cx)
                                                .call_with(&mut cx)
                                                .apply::<JsValue, TaskContext<'_>>(&mut cx);
                                        }
                                    }
                                }

                                // B. CLEANUP: Drop all strong references (Roots) held by this worker
                                // This is crucial to allow the GC to collect the associated JS objects
                                {
                                    let mut cb_store = crate::NODE_CALLBACKS.blocking_lock();
                                    cb_store[channel_id] = crate::NodeCallbacks::default(); // Replaces with None/Empty

                                    let mut global_store = crate::GLOBAL_PTRS.blocking_lock();
                                    global_store[channel_id].clear();
                                }

                                // C. CRITICAL: Drop the Channel handle on the Main Thread.
                                // We do this explicitly to ensure the N-API ThreadSafeFunction is released now.
                                {
                                    let mut channels = crate::NODE_CHANNELS.blocking_lock();
                                    channels[channel_id] = None;
                                }

                                // D. Resolve the Promise to let JS know we are done
                                let handle = cx.undefined();
                                def.resolve(&mut cx, handle);
                                Ok(())
                            });
                        }

                        // 3. Drop the lock in the background thread immediately
                        drop(unlocked);

                        // Break the loop to stop the background thread
                        break;
                    }
                }
            }
        }
    });

    thread::spawn(move || {
        let tokio_rt = Runtime::new().unwrap();
        let msg_cb_clone = node_msg_callbacks.clone();

        tokio_rt.block_on(async move {
            let msg_cb_clone = msg_cb_clone.clone();
            let realm_id = channel_id.to_string();

            {
                let rtt = rt.lock().await;
                rtt.create_realm(channel_id.to_string().as_str()).unwrap();
                rtt.loop_realm(Some(&realm_id), move |_, realm| {
                        quick_js_console!("log", console_log);
                        realm.install_function(&["console"], "log", console_log, 20).unwrap();
                        quick_js_console!("info", console_info);
                        realm.install_function(&["console"], "info", console_info, 20).unwrap();
                        quick_js_console!("warn", console_warn);
                        realm.install_function(&["console"], "warn", console_warn, 20).unwrap();
                        quick_js_console!("error", console_error);
                        realm.install_function(&["console"], "error", console_error, 20).unwrap();
                        realm.install_function(&[], "postMessage", QuickJSWorker::post_message, 1).unwrap();
                        realm.install_function(&[], "on", QuickJSWorker::event_listener, 2).unwrap();
                        
                        realm.install_function(&[], "__INTERNAL_CALL_GLOBAL", |_runtime, realm, _this, args| {
                            let channel_id = realm.id.parse::<usize>().unwrap();
                            let callback_id = args[0].to_i32() as usize;
                            let callbacks_args_length = realm.get_array_length(&args[1])?;
                            let mut js_args: Vec<JsDataTypes> = Vec::new();
                            for i in 0..callbacks_args_length {
                                js_args.push(JsDataTypes::from_quick_value(&realm.get_array_element(&args[1], i)?, realm)?);
                            }
                            let sync_sender_opt = {
                                let map = ACTIVE_SYNC_REQUESTS.blocking_lock();
                                map.get(&channel_id).cloned()
                            };
                            if let Some(sync_sender) = sync_sender_opt {
                                let (resp_tx, resp_rx) = oneshot::channel();
                                let req = crate::SyncEvalRequest::Execute { callback_id, args: js_args, resp: resp_tx };
                                if let Err(_) = sync_sender.blocking_send(req) {
                                    return realm.create_error("Internal", "Node runtime closed", "");
                                }
                                match resp_rx.blocking_recv() {
                                    Ok(Ok(result_data)) => { return result_data.to_quick_value(realm); },
                                    Ok(Err(e_str)) => { return realm.create_error("ExecutionError", e_str.as_str(), ""); },
                                    Err(_) => { return realm.create_error("Internal", "Response channel closed", ""); }
                                }
                            } else {
                                let deferred_script = Script::new("create_deferred", "(() => { let m; const p = new Promise((res, rej) => { m = { res, rej }; }); return { p, res: m.res, rej: m.rej }; })()");
                                let deferred_obj = realm.eval(deferred_script)?;
                                let promise = realm.get_object_property(&deferred_obj, "p")?;
                                let resolve_func = realm.get_object_property(&deferred_obj, "res")?;
                                let reject_func = realm.get_object_property(&deferred_obj, "rej")?;
                                let req_id = GLOBAL_PROMISE_ID.fetch_add(1, Ordering::Relaxed);
                                {
                                    let mut promises = PENDING_GLOBAL_PROMISES.blocking_lock();
                                    let res_facade = realm.to_js_value_facade(&resolve_func)?;
                                    let rej_facade = realm.to_js_value_facade(&reject_func)?;
                                    promises.insert(req_id, (res_facade, rej_facade));
                                }
                                let unlocked = NODE_CHANNELS.blocking_lock();
                                if let Some(channel) = &unlocked[channel_id] {
                                    let quick_sender = crate::QUICKJS_SENDERS.blocking_lock()[channel_id].clone();
                                    channel.send(move |mut cx| {
                                        let globals = crate::GLOBAL_PTRS.blocking_lock();
                                        let (_, value) = &globals[channel_id][callback_id];
                                        match value {
                                            GlobalTypes::Function { value } => {
                                                let mut fn_handle = value.to_inner(&mut cx).call_with(&mut cx);
                                                for arg in js_args.iter() { fn_handle.arg(arg.to_node_value(&mut cx)?); }
                                                let result = fn_handle.apply::<JsValue, _>(&mut cx)?;
                                                if result.is_a::<JsPromise, _>(&mut cx) {
                                                    let promise = result.downcast::<JsPromise, _>(&mut cx).unwrap();
                                                    let then = promise.get::<JsValue, _, _>(&mut cx, "then")?.downcast::<JsFunction, _>(&mut cx).or_else(|_| cx.throw_error("Missing then"))?;
                                                    let qs_success = quick_sender.clone();
                                                    let success_func = JsFunction::new(&mut cx, move |mut cx| {
                                                        let val = cx.argument::<JsValue>(0)?;
                                                        let data = JsDataTypes::from_node_value(val, &mut cx).unwrap_or(JsDataTypes::Unknown);
                                                        qs_success.blocking_send(QuickChannelMsg::ResolveGlobalPromise { id: req_id, result: data, error: None }).ok();
                                                        Ok(cx.undefined())
                                                    })?;
                                                    let qs_error = quick_sender.clone();
                                                    let error_func = JsFunction::new(&mut cx, move |mut cx| {
                                                        let val = cx.argument::<JsValue>(0)?;
                                                        let err_msg = val.to_string(&mut cx)?.value(&mut cx);
                                                        qs_error.blocking_send(QuickChannelMsg::ResolveGlobalPromise { id: req_id, result: JsDataTypes::Undefined, error: Some(err_msg) }).ok();
                                                        Ok(cx.undefined())
                                                    })?;
                                                    let args: Vec<Handle<JsValue>> = vec![success_func.upcast(), error_func.upcast()];
                                                    then.call(&mut cx, promise, args)?;
                                                } else {
                                                    let data = JsDataTypes::from_node_value(result, &mut cx)?;
                                                    quick_sender.blocking_send(QuickChannelMsg::ResolveGlobalPromise { id: req_id, result: data, error: None }).ok();
                                                }
                                            },
                                            GlobalTypes::Data { value } => {
                                                quick_sender.blocking_send(QuickChannelMsg::ResolveGlobalPromise { id: req_id, result: value.clone(), error: None }).ok();
                                            }
                                        }
                                        Ok(())
                                    });
                                }
                                Ok(promise)
                            }
                        }, 2).unwrap();

                        realm.install_function(&[], "moduleReturn", |_runtime, realm, _this, args| {
                            let global = realm.get_global()?;
                            realm.set_object_property(&global, "__qjs_module_return_value", &args[0])?;
                            Ok(realm.create_undefined()?)
                        }, 1).unwrap();
                }).await;
            }

            while let Some(message) = urx.recv().await {
                if let QuickChannelMsg::Quit { def } = message {
                    let sync_channels = SYNC_SENDERS.lock().await;
                    sync_channels[channel_id].send(SyncChannelMsg::Quit { def }).await.unwrap();
                    break;
                }
                let rt_clone = rt.clone();
                let msg_cb_clone = msg_cb_clone.clone();
                let realm_id_clone = realm_id.clone();

                tokio::spawn(async move {
                    let msg_cb_clone = msg_cb_clone.clone();
                    match message {
                        QuickChannelMsg::SetGlobal { key, structure, def } => {
                            let runtime = rt_clone.lock().await;
                            
                            let init_script = runtime.loop_async(move |_runtime| {
                                let structure_json: serde_json::Value = serde_json::from_str(&structure).unwrap();
                                let mut generated_code = String::new();
                                // FIX: Initialize the root object variable first
                                generated_code.push_str(&format!("if (typeof {0} === 'undefined') var {0} = {{}};\n", key));
                                
                                if let Some(obj) = structure_json.as_object() {
                                    if obj.contains_key("__t") {
                                        let id = obj["__i"].as_u64().unwrap();
                                        let type_str = obj["__t"].as_str().unwrap();
                                        if type_str == "f" {
                                            return format!("globalThis.{} = function(...args) {{ return __INTERNAL_CALL_GLOBAL({}, args); }};", key, id);
                                        } else {
                                            return format!("globalThis.{} = __INTERNAL_CALL_GLOBAL({}, []);", key, id);
                                        }
                                    }
                                }
                                generated_code.push_str(&generate_init_script(&structure_json, key));
                                return generated_code;
                            }).await;
                            
                            let rt_id = realm_id_clone.clone();
                            runtime.loop_async(move |runtime| {
                                let realm = runtime.get_realm(&rt_id).unwrap();
                                realm.eval(Script::new(std::format!("init_globals.js").as_str(), init_script.as_str())).unwrap();
                            }).await;
                            let sync_channel = SYNC_SENDERS.lock().await;
                            sync_channel[channel_id].send(SyncChannelMsg::LoadBytes { def }).await.unwrap(); 
                        }

                        QuickChannelMsg::InitGlobals { structure } => {
                            let runtime = rt_clone.lock().await;
                            let init_script = runtime.loop_async(move |_runtime| {
                                let structure_json: serde_json::Value = serde_json::from_str(&structure).unwrap();
                                return generate_init_script(&structure_json, String::new());
                            }).await;
                            let rt_id = realm_id_clone.clone();
                            runtime.loop_async(move |runtime| {
                                let realm = runtime.get_realm(&rt_id).unwrap();
                                realm.eval(Script::new(std::format!("init_globals.js").as_str(), init_script.as_str())).unwrap();
                            }).await;
                        }

                        // ... REMAINING CASES SAME AS PROVIDED PREVIOUSLY ...
                        // Ensure EvalScript block resets INT_COUNTERS.2 correctly as provided in previous answer
                        QuickChannelMsg::EvalScript { target } => {
                            {
                                let mut int_counters = INT_COUNTERS.lock().await;
                                int_counters[channel_id].1 = 0;
                                if let Some(ms) = opts.max_eval_time {
                                    int_counters[channel_id].2 = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() + ms as u128;
                                } else {
                                    int_counters[channel_id].2 = 0;
                                }
                            }
                            // ... rest of EvalScript ...
                            if let ScriptEvalType::Sync { sender, .. } = &target {
                                let mut map: tokio::sync::MutexGuard<HashMap<usize, tokio::sync::mpsc::Sender<crate::SyncEvalRequest>>> = ACTIVE_SYNC_REQUESTS.lock().await;
                                map.insert(channel_id, sender.clone());
                            }
                            let rt_id = realm_id_clone.clone();
                            let (script_data, is_module_eval) = match &target {
                                ScriptEvalType::Sync { script_name, source, is_module, sender: _ , args } => 
                                    (Some((Script::new(&script_name, source), args.clone())), *is_module),
                                ScriptEvalType::Async { script_name, source, is_module, promise: _, args } => 
                                    (Some((Script::new(&script_name, source), args.clone())), *is_module)
                            };
                            let script_future = rt_clone.lock().await.loop_async( move |runtime| {
                                let realm = runtime.get_realm(&rt_id).unwrap();
                                if let Some((use_script, args)) = script_data {
                                    let eval_result = if is_module_eval {
                                        realm.eval_module(use_script)
                                    } else {
                                        realm.eval(use_script)
                                    };
                                    match eval_result {
                                        Ok(eval_result) => {
                                            if eval_result.is_function() {
                                                let use_args = if let Some(args) = args { args.iter().map(|x| x.to_quick_value(realm).unwrap()).collect() } else { Vec::new() }; 
                                                let result = realm.invoke_function(None, &eval_result, &use_args.iter().collect::<Vec<&QuickJsValueAdapter>>().as_slice())?;
                                                return realm.to_js_value_facade(&result);
                                            } else { return realm.to_js_value_facade(&eval_result); }
                                        }
                                        Err(e) => return Err(e),
                                    }
                                } else {
                                    match realm.eval(Script::new(".", "undefined")) {
                                        Ok(eval_result) => { return realm.to_js_value_facade(&eval_result); }
                                        Err(e) => return Err(e),
                                    }
                                }
                            });
                            let rt_id = realm_id_clone.clone();
                            let start_time = std::time::Instant::now();
                            let start_cpu = cpu_time::ProcessTime::now();
                            let out;
                            if let Some(max_time) = opts.max_eval_time {
                                match tokio::time::timeout(std::time::Duration::from_millis(max_time), script_future).await {
                                    Err(e) => {
                                        if let ScriptEvalType::Sync { .. } = &target {
                                            let mut map: tokio::sync::MutexGuard<HashMap<usize, tokio::sync::mpsc::Sender<crate::SyncEvalRequest>>> = ACTIVE_SYNC_REQUESTS.lock().await;
                                            map.remove(&channel_id);
                                        }
                                        match target {
                                            ScriptEvalType::Sync { sender, .. } => { sender.send(crate::SyncEvalRequest::Done { result: Err(e.to_string()) }).await.unwrap_or(()); },
                                            ScriptEvalType::Async { promise, .. }  => { let sync_channel = SYNC_SENDERS.lock().await; sync_channel[channel_id].send(SyncChannelMsg::SendError { e: e.to_string(), def: promise }).await.unwrap(); }
                                        }
                                        return;
                                    }
                                    Ok(script_result) => { out = QuickJSWorker::process_script_eval(rt_id.clone(), start_time, opts.max_eval_time, script_result, rt_clone.clone(), is_module_eval).await; }
                                };
                            } else {
                                let script_result = script_future.await;
                                out = QuickJSWorker::process_script_eval(rt_id.clone(), start_time, opts.max_eval_time, script_result, rt_clone.clone(), is_module_eval).await;
                            }
                            if let ScriptEvalType::Sync { .. } = &target {
                                let mut map: tokio::sync::MutexGuard<HashMap<usize, tokio::sync::mpsc::Sender<crate::SyncEvalRequest>>> = ACTIVE_SYNC_REQUESTS.lock().await;
                                map.remove(&channel_id);
                            }
                            match target {
                                ScriptEvalType::Sync { sender, .. } => {
                                    match out {
                                        Ok(data) => { sender.send(crate::SyncEvalRequest::Done { result: Ok((data, start_cpu, start_time)) }).await.unwrap(); },
                                        Err(e) => { sender.send(crate::SyncEvalRequest::Done { result: Err(e) }).await.unwrap(); }
                                    }
                                },
                                ScriptEvalType::Async { promise, .. }  => {
                                    let sync_channel = SYNC_SENDERS.lock().await;
                                    match out {
                                        Ok(data) => { sync_channel[channel_id].send(SyncChannelMsg::RespondEval { def: promise, result: data, cpu_time: start_cpu, start_time }).await.unwrap(); },
                                        Err(e) => { sync_channel[channel_id].send(SyncChannelMsg::SendError { e: e, def: promise }).await.unwrap(); }
                                    }
                                }
                            }
                        }
                        
                        QuickChannelMsg::GetByteCode { def, source } => {
                            let rt_id = realm_id_clone.clone();
                            let bytes = rt_clone.lock().await.loop_async(move |runtime| unsafe {
                                    let ctx = runtime.get_realm(&rt_id).unwrap().context;
                                    let compiled = quickjs_runtime::quickjs_utils::compile::compile(ctx, Script::new(".", source.as_str())).unwrap();
                                    return quickjs_runtime::quickjs_utils::compile::to_bytecode(ctx, &compiled);
                            }).await;
                            let sync_channel = SYNC_SENDERS.lock().await;
                            sync_channel[channel_id].send(SyncChannelMsg::SendBytes { bytes, def }).await.unwrap();
                        }
                        QuickChannelMsg::LoadByteCode { bytes, def } => {
                            let rt_id = realm_id_clone.clone();
                            rt_clone.lock().await.loop_async(move |runtime| unsafe {
                                    let ctx = runtime.get_realm(&rt_id).unwrap().context;
                                    let out = quickjs_runtime::quickjs_utils::compile::from_bytecode(ctx, bytes.as_slice()).unwrap();
                                    quickjs_runtime::quickjs_utils::compile::run_compiled_function(ctx, &out).unwrap();
                            }).await;
                            let sync_channel = SYNC_SENDERS.lock().await;
                            sync_channel[channel_id].send(SyncChannelMsg::LoadBytes { def }).await.unwrap();
                        }
                        QuickChannelMsg::GarbageCollect { def, start } => {
                            rt_clone.lock().await.gc().await;
                            let sync_channel = SYNC_SENDERS.lock().await;
                            sync_channel[channel_id].send(SyncChannelMsg::GarbageCollect { start, def }).await.unwrap();
                        }
                        QuickChannelMsg::Memory { def } => {
                            let json = rt_clone.lock().await.loop_async(move |runtime| { return serde_json::to_string(&runtime.memory_usage()).unwrap(); }).await;
                            let sync_channel = SYNC_SENDERS.lock().await;
                            sync_channel[channel_id].send(SyncChannelMsg::Memory { json, def }).await.unwrap();
                        }
                        QuickChannelMsg::Quit { def: _ } => { }
                        QuickChannelMsg::SendMessageToQuick { message } => {
                            let rt_id = realm_id_clone.clone();
                            rt_clone.lock().await.loop_async(move |runtime| {
                                    let realm = runtime.get_realm(&rt_id).unwrap();
                                    let channel_id = realm.id.parse::<usize>().unwrap();
                                    let callbacks = &mut QUICKJS_CALLBACKS.blocking_lock()[channel_id];
                                    let mut parsed_callbacks: Vec<QuickJsValueAdapter> = callbacks.drain(..).map(|cb| realm.from_js_value_facade(cb).unwrap()).collect();
                                    let callback_value = message.to_quick_value(realm).unwrap();
                                    for i in 0..parsed_callbacks.len() { realm.invoke_function(None, &parsed_callbacks[i], &[&callback_value]).unwrap(); }
                                    let _ = parsed_callbacks.drain(..).for_each(|cb| { callbacks.push(realm.to_js_value_facade(&cb).unwrap()); });
                            }).await;
                        }
                        QuickChannelMsg::ResolveGlobalPromise { id, result, error } => {
                            let rt_id = realm_id_clone.clone();
                            rt_clone.lock().await.loop_async(move |runtime| {
                                let realm = runtime.get_realm(&rt_id).unwrap();
                                let mut resolvers = PENDING_GLOBAL_PROMISES.blocking_lock();
                                if let Some((res, rej)) = resolvers.remove(&id) {
                                    let res_handle = realm.from_js_value_facade(res)?;
                                    let rej_handle = realm.from_js_value_facade(rej)?;
                                    if let Some(err_msg) = error {
                                        let err_val = realm.create_string(&err_msg)?;
                                        realm.invoke_function(None, &rej_handle, &[&err_val])?;
                                    } else {
                                        let val = result.to_quick_value(realm)?;
                                        realm.invoke_function(None, &res_handle, &[&val])?;
                                    }
                                }
                                Ok::<_, quickjs_runtime::jsutils::JsError>(())
                            }).await.unwrap()
                        }
                        QuickChannelMsg::NewCallback { root, on } => {
                            let cbs = msg_cb_clone.clone();
                            let mut unlocked = cbs.lock().await;
                            unlocked.push((root, on));
                        }
                    }
                });
            }
        });
    });
}
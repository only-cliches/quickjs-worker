
export type EvalStats = {
    interrupts: number,
    evalTimeMs: number,
    cpuTimeMs: number
}

export type Primitive = string | number | boolean;

/**
 * Create a new QuickJS runtime in it's own thread
 */
export const QuickJSWorker: (args?: {
    /** Capture console commands from the runtime */
    console?: {
        log: (...values: any) => any,
        warn: (...values: any) => any,
        info: (...values: any) => any,
        error: (...values: any) => any,
    },
    /** 
     * How big is the message queue used by the runtime? 
     * A message queue is used to pass commands and values between Node and the QuickJS runtime.  When the message queue is full, subsequent commands will fail and cause an error.
     * bigger channel = more memory
     * Values under 3 are not recommended
     * 
     * Default is 10.
    */
    channelSize?: number,
    /** Set the maxmimum milliseconds each call to eval is allowed to take. Default is unlimited. */
    maxEvalMs?: number,
    /** Set the maxmimum total bytes the runtime is allowed to use for memory. Default is unlimited. */
    maxMemoryBytes?: number,
    /** Set the maxmimum stack size bytes the runtime is allowed to use for memory. Default is unlimited. */
    maxStackSizeBytes?: number,
    /**
     * When eval is called, an interrupt handler is ran internally from time to time to see if execution should continue.
     * Once the number of interrupt calls reaches the threshold provided here, the eval call will be terminated.
     * Prevents while(true) loops, more interrupts allow more code exection.  Default is unlimited.
     */
    maxInterrupt?: number,
    /** How many allocations can the runtime do before the GC runs? */
    gcThresholdAlloc?: number,
    /** Automatically run the GC at regular intervals */
    gcIntervalMs?: number,
    /** Provide static callbacks and primitives to the runtime */
    staticGlobals?: {
        [key: string]: Primitive | Date | Array<Primitive> | {[key: string]: Primitive} | ((arg: any) => any)
    }
    // imports: (moduleName) => {
    //     console.log('IMPORTS', moduleName);
    //     return "export default ({})";
    // },
    // normalize: (refPath, path) => ""
    // setTimeout: true,
    // setImmediate: true
}) => {
    /** 
     * Evaluate Javascript code in the virtual machine, will recursively resolve promises that are returned from .eval and .evalSync.
     * Primitives, JSON and Arrays can returned across the eval barrier without an issue.
     * There is a serialization/deserialization cost when returning Arrays and JSON.
     * 
     * */
    eval: <T>(jsSourceCode: string, scriptName?: string) => Promise<[T, EvalStats]>,
    evalSync: <T>(jsSourceCode: string, scriptName?: string) => [T, EvalStats]
    /** Handle postMessage() events from the virtual machine */
    on: (type: "message" | "close", callback: (event: any) => void) => void
    /** Convert a block of source code into QuickJS ByteCode */
    getByteCode: (jsSourceCode: string) => Promise<Uint8Array>
    /** Load QuickJS ByteCode into the runtime */
    loadByteCode: (byteCode: Uint8Array) => Promise<void>,
    /** Force the garbage collector to run */
    gc: () => Promise<number>,
    /** Get the current memory stats for the runtime */
    memory: () => Promise<{
        realm_ct: number,
        malloc_size: number,
        malloc_limit: number,
        memory_used_size: number,
        malloc_count: number,
        memory_used_count: number,
        atom_count: number,
        atom_size: number,
        str_count: number,
        str_size: number,
        obj_count: number,
        obj_size: number,
        prop_count: number,
        prop_size: number,
        shape_count: number,
        shape_size: number,
        js_func_count: number,
        js_func_size: number,
        js_func_code_size: number,
        js_func_pc2line_count: number,
        js_func_pc2line_size: number,
        c_func_count: number,
        array_count: number,
        fast_array_count: number,
        fast_array_elements: number,
        binary_object_count: number,
        binary_object_size: number
      }>
    /** 
     * Quit the runtime and cancel all pending tasks, cannot be reversed!
     * 
     * Must be called at the end of Node scripts or the Node process will hang.
     */
    close: () => Promise<void>,
    /** Check if the runtime has previously been .close(). */
    isClosed: () => boolean,
    /** 
     * Send a message into the runtime.  Can be captured with `on('message', (msg) => {..})` in the runtime. 
     * Primitives, JSON and Arrays can sent to the runtime using this function.
     * There is a serialization/deserialization cost when sending Arrays and JSON.
     * */
    postMessage: (message: any) => void
}
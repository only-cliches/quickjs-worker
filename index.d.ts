import { EventEmitter } from 'events';

export type EvalStats = {
    interrupts: number;
    evalTimeMs: number;
    cpuTimeMs: number;
}

export type QuickJSMemoryStats = {
    realm_ct: number,
    malloc_size: number,
    malloc_limit: number,
    /** Number of bytes being used by VM Memory */
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
};

export type QuickJSOptions = {
    console?: Console;
    channelSize?: number;
    maxEvalMs?: number;
    maxMemoryBytes?: number;
    maxStackSizeBytes?: number;
    maxInterrupt?: number;
    gcThresholdAlloc?: number;
    gcIntervalMs?: number;
    globals?: Record<string, any>;
    imports?: (moduleName: string) => string;
}

export type EvalOptions = {
    filename?: string;
    type?: "module" | "classic";
    args?: any[];
}

export class QuickJS extends EventEmitter {
    constructor(options?: QuickJSOptions);

    /** Stats from the last eval call */
    lastExecutionStats: EvalStats | null;

    /**
     * Evaluate Javascript code asynchronously.
     * @param code The Javascript source code.
     * @param options Execution options (filename, module type, args).
     */
    eval<T = any>(code: string, options?: EvalOptions | string): Promise<T>;

    /**
     * Evaluate Javascript code synchronously.
     * @param code The Javascript source code.
     * @param options Execution options.
     */
    evalSync<T = any>(code: string, options?: EvalOptions | string): T;

    /**
     * Evaluate code as an ES Module. 
     * Use global `moduleReturn(val)` in the script to return a value.
     */
    evalModule<T = any>(code: string, options?: EvalOptions | string): Promise<T>;

    /**
     * Dynamically add a global variable or function.
     */
    setGlobal(key: string, value: any): Promise<void>;

    /** Send a message to the runtime */
    postMessage(msg: any): void;

    /** Compile source code to bytecode */
    getByteCode(code: string): Promise<Uint8Array>;

    /** Load compiled bytecode */
    loadByteCode(bytes: Uint8Array): Promise<void>;

    /** Force garbage collection */
    gc(): Promise<number>;

    /** Get memory statistics */
    memory(): Promise<QuickJSMemoryStats>;

    /** Close the runtime */
    close(): Promise<void>;

    /** Check if runtime is closed */
    isClosed(): boolean;

    /** Async Dispose support */
    [Symbol.asyncDispose](): Promise<void>;
}

export const QJSWorker: (options?: QuickJSOptions) => QuickJS;
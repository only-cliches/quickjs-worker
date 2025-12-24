# quickjs-vm

**A high-performance, native Node.js binding for the [QuickJS](https://bellard.org/quickjs/) Javascript Engine.**

`quickjs-vm` allows you to safely execute untrusted JavaScript code inside a sandboxed environment within your Node.js application. Unlike other libraries that compile QuickJS to WebAssembly (WASM), `quickjs-vm` uses native N-API bindings (written in Rust).

This results in **significantly higher performance**, lower overhead, and true synchronous execution capabilities compared to WASM-based alternatives like `quickjs-emscripten`.

## 🚀 Features

* **⚡️ High Performance**: Runs directly on the host CPU without WASM interpretation overhead.
* **🔄 Async & Sync Evaluation**: Strict `evalSync` for blocking operations and `eval` (Promise-based) for async workflows.
* **📦 ES Modules Support**: Full support for `import` / `export` syntax with custom module resolution handlers.
* **🛡 Sandboxing & Limits**:
* **Timeouts**: `maxEvalMs` to kill infinite loops.
* **Memory Limits**: `maxMemoryBytes` to prevent OOM crashes.
* **Stack Limits**: Control recursion depth.


* **💾 Data Serialization**: Seamless round-trip data passing for:
* Primitives (`string`, `number`, `boolean`, `null`, `undefined`)
* Complex Types (`Date`, `Buffer` / `Uint8Array`, JSON Objects)
* Functions (pass Node.js functions into QuickJS)


* **📞 Inter-Process Communication**:
* `postMessage` API for bidirectional messaging between Node and the QuickJS worker.
* Event-based architecture (`on('message')`, `on('close')`).


* **⚙️ Bytecode**: Pre-compile JavaScript to bytecode (`getByteCode`) and execute it later (`loadByteCode`) for faster startup times.
* **🧹 Lifecycle Management**:
* Explicit `close()` to release native resources.
* `gc()` to force garbage collection.
* `memory()` inspection.
* `Symbol.asyncDispose` support for the `using` keyword.



---

## 📊 Performance Benchmarks

Because `quickjs-vm` binds directly to the native QuickJS library via Rust/N-API, it outperforms WASM-based implementations significantly.

We ran the standard **V8 Benchmark Suite (v7)** comparing `quickjs-vm` against `quickjs-emscripten`.

### 🏆 Overall Score (Higher is Better)

| Library | Score | Improvement |
| --- | --- | --- |
| **quickjs-vm (Native)** | **1052** | **~42% Faster** 🚀 |
| quickjs-emscripten (WASM) | 742 |  |

### 📉 Detailed Breakdown

| Benchmark | quickjs-vm | quickjs-emscripten | Difference |
| --- | --- | --- | --- |
| **Richards** | **1014** | 579 | +75.1% |
| **DeltaBlue** | **1043** | 653 | +59.7% |
| **Crypto** | **878** | 556 | +57.9% |
| **RayTrace** | **1008** | 773 | +30.4% |
| **EarleyBoyer** | **1948** | 1409 | +38.2% |
| **RegExp** | **270** | 227 | +18.9% |
| **Splay** | **2208** | 1862 | +18.5% |
| **NavierStokes** | **1385** | 948 | +46.1% |

*Benchmarks run on macOS M1/M2 architecture. Results may vary by machine, but the relative performance gap is consistent.*

---

## 📦 Installation

```bash
npm install quickjs-vm

```

---

## 💻 Usage

### Basic Evaluation & Memory Inspection

You can execute code and immediately inspect the runtime's memory usage to ensure scripts aren't leaking or consuming too many resources.

```javascript
const { QuickJS } = require('quickjs-vm');

const vm = new QuickJS();

// Async Evaluation
const result = await vm.eval('1 + 2');
console.log('Result:', result); // 3

// Synchronous Evaluation
const syncResult = vm.evalSync('"Hello " + "World"');
console.log('Sync Result:', syncResult); // "Hello World"

// Check memory usage after execution
const stats = await vm.memory();
console.log('Memory Used:', stats); 
// Output example: { memory_used_size: 2048, memory_limit: -1, ... }

await vm.close();

```

### Function Calls with Arguments

You can pass arguments directly into your QuickJS scripts using the `{ args: [...] }` option. This is safer and cleaner than string concatenation.

```javascript
const vm = new QuickJS();

// Define a function in QuickJS that accepts arguments
const script = `
    (greeting, name, count) => {
        const lines = [];
        for (let i = 0; i < count; i++) {
            lines.push(greeting + ", " + name + "!");
        }
        return lines;
    }
`;

// Pass arguments securely from Node.js
const result = await vm.eval(script, { 
    args: ["Hello", "Developer", 3] 
});

console.log(result);
// Output: [ "Hello, Developer!", "Hello, Developer!", "Hello, Developer!" ]

```

### Managing Global State

You can inject global variables into the QuickJS environment in two ways: **at initialization** or **at runtime**.

#### 1. Initialization (Static Globals)

Use the `globals` option in the constructor to define variables that should be available immediately when the VM starts. This is ideal for configuration, polyfills, or standard library helper functions.

```javascript
const vm = new QuickJS({
    // Define globals that exist before any script runs
    globals: {
        version: '1.0.0',
        environment: process.env.NODE_ENV || 'development',
        utils: {
            log: (msg) => console.log(`[LOG]: ${msg}`),
            add: (a, b) => a + b
        }
    }
});

// These globals are immediately available
const result = await vm.eval(`
    utils.log("System version: " + version);
    utils.add(10, 20);
`);
console.log(result); // 30

```

#### 2. Runtime (Dynamic Globals)

Use `setGlobal` to inject variables after the VM has started. This is useful for passing data that is fetched asynchronously or updated during execution.

```javascript
const vm = new QuickJS();

// ... some time later ...

// Fetch data from a database or API
const userData = await db.getUser(123);

// Inject it into the running VM
await vm.setGlobal('user', userData);

// Now the script can access the new data
const userName = await vm.eval('user.name');
console.log(userName);

```

### Advanced Configuration (Limits & Globals)

Secure your sandbox by setting memory and time limits, and inject custom globals.

```javascript
const vm = new QuickJS({
    maxMemoryBytes: 1024 * 1024 * 5, // 5MB Limit
    maxEvalMs: 500,                  // 500ms Timeout
    console: console,                // Forward console.log to Node
});

// Inject a function (can be async!)
await vm.setGlobal('fetchData', async (id) => {
    // This runs in Node.js, called from QuickJS
    return db.users.find(id);
});

try {
    const user = await vm.eval('fetchData(123)');
    // this works too:
    // const user = await vm.eval('fetchData', {args: [ 123 ]});
} catch (err) {
    console.error('Script failed:', err);
}

```

### ES Modules & Custom Imports

You can define a custom import handler to load modules from the file system, a database, or memory.

```javascript
const vm = new QuickJS({
    // define function to return modules
    imports: (path) => {
        if (path === './math.js') {
             // return source code
            return 'export const add = (a, b) => a + b;';
        }
        throw new Error('Module not found');
    }
});

const code = `
    import { add } from './math.js';
    export const result = add(10, 20);
    moduleReturn(result); // Helper to return value from module
`;

const result = await vm.evalModule(code);
console.log(result); // 30

```

### Communication Channel (postMessage)

Use the bidirectional messaging channel for event-driven architectures.

```javascript
// Node.js Side
vm.on('message', (msg) => {
    console.log('Received from QJS:', msg);
});

// Send data to QuickJS
vm.postMessage({ type: 'INIT', payload: '...' });

// ------------------------------------------------

// QuickJS Side (inside eval)
on('message', (msg) => {
    if (msg.type === 'INIT') {
        postMessage({ status: 'READY' });
    }
});

```

---

## 🛠 API Reference

### `new QuickJS(options)`

* `options.maxMemoryBytes`: (Number) Max memory for the runtime.
* `options.maxEvalMs`: (Number) Max execution time before throwing an interrupt error (for each eval).
* `options.imports`: (Function) Callback `(path) => string` for loading module source code.
* `options.console`: (Object) Pass the Node `console` object to pass through vm logging to node, otherwise provide your own callbacks.
* `options.globals`: (Object) Initial map of globals to inject at startup.

### Methods

* `eval(code, options)`: Execute code asynchronously. Returns a Promise.
* `options.args`: (Array) List of arguments to pass to the script (if the script returns a function).


* `evalSync(code, options)`: Execute code synchronously. Returns the result.
* `evalModule(code)`: Execute code as an ES Module.
* `setGlobal(key, value)`: Inject a global variable or function at runtime.
* `getByteCode(code)`: Compile JS source to a `Uint8Array` of bytecode.
* `loadByteCode(bytes)`: Execute pre-compiled bytecode.
* `postMessage(msg)`: Send a message to the VM.
* `memory()`: Returns memory usage statistics (malloc limit, memory used, address count, etc).
* `gc()`: Force garbage collection.
* `close()`: Shut down the VM and release native resources.

---

## 🧪 Testing

This project maintains a high standard of quality with a comprehensive **Jest** test suite covering:

* Round-trip serialization of complex data types (Date, Buffer, etc).
* Sync vs Async execution correctness.
* Promise resolution for host functions.
* Module system resolution.
* Resource cleanup and memory leaks (using `weak` references checks).
* Benchmark scripts (V8 suite).

Run tests locally:

```bash
npm test

```

---

## License

MIT
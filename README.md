# quickjs-vm

**High-performance native Node.js bindings for the [QuickJS](https://bellard.org/quickjs/) JavaScript engine.**

`quickjs-vm` allows you to safely execute **untrusted JavaScript** inside a sandboxed QuickJS runtime embedded directly in Node.js.

Unlike libraries that compile QuickJS to WebAssembly, `quickjs-vm` uses **native N-API bindings written in Rust**, providing:

- ⚡ Significantly higher performance
- 🧠 Lower memory overhead
- ⏱ True synchronous execution (no async WASM trampolines)

If you need **fast, deterministic, sandboxed JavaScript execution** inside Node — this library is built for that purpose.

---

## 🚀 Features

### ⚡ Performance
- Native QuickJS runtime (no WASM)
- Direct execution on the host CPU
- Lower startup and execution overhead

### 🔄 Execution Modes
- `evalSync()` — blocking, deterministic execution
- `eval()` — Promise-based async execution
- `evalModule()` — ES module support

### 🛡 Sandboxing & Limits
Protect your host application from runaway or malicious scripts:

- **Execution timeouts** (`maxEvalMs`)
- **Memory limits** (`maxMemoryBytes`)
- **Prevent Infinite Loops** (`maxInterrupt`)
- **Stack depth limits** (prevents recursion abuse)

### 💾 Data Serialization
Safe, seamless data exchange between Node and QuickJS:

- Primitives (`string`, `number`, `boolean`, `null`, `undefined`)
- Structured data (Objects, Arrays, JSON)
- `Date`
- `Error`
- `Buffer` / `Uint8Array`
- Functions (call Node functions from QuickJS)

### 📞 Messaging & IPC
- Bidirectional `postMessage` API
- Event-based lifecycle:
  - `on('message')`
  - `on('close')`

### ⚙️ Bytecode Support
- Compile source → bytecode (`getByteCode`)
- Execute bytecode later (`loadByteCode`)
- Faster startup for repeated workloads

---

## 📊 Performance Benchmarks

Because `quickjs-vm` binds directly to the native QuickJS C library, it **significantly outperforms** WASM-based implementations.

Benchmarks were run using the **V8 Benchmark Suite (v7)** comparing:

- `quickjs-vm` (native)
- `quickjs-emscripten` (WASM)

### 🏆 Overall Score (Higher is Better)

| Library | Score | Improvement |
|------|------|------|
| **quickjs-vm (Native)** | **1052** | **~42% Faster** 🚀 |
| quickjs-emscripten (WASM) | 742 | |

### 📉 Detailed Results

| Benchmark | quickjs-vm | emscripten | Δ |
|--------|-----------|------------|----|
| Richards | **1014** | 579 | +75% |
| DeltaBlue | **1043** | 653 | +60% |
| Crypto | **878** | 556 | +58% |
| RayTrace | **1008** | 773 | +30% |
| EarleyBoyer | **1948** | 1409 | +38% |
| RegExp | **270** | 227 | +19% |
| Splay | **2208** | 1862 | +19% |
| NavierStokes | **1385** | 948 | +46% |

> Benchmarks run on macOS (Apple Silicon). Absolute numbers vary by machine, but the relative performance gap is consistent.

---

## 📦 Installation

```sh
npm install quickjs-vm
````

---

## 💻 Usage

### Basic Evaluation

```ts
import { QuickJS } from 'quickjs-vm';

const vm = new QuickJS();

const result = await vm.eval('1 + 2');
console.log(result); // 3

// optionally resolves promises as well
const result = await vm.eval('new Promise(res => res(1 + 2))');
console.log(result); // 3

// blocking call
const syncResult = vm.evalSync('"Hello " + "World"');
console.log(syncResult); // Hello World

// get number of bytes currently in use by the vm
const memory = await vm.memory();
console.log(memory.memory_used_size);

await vm.close();
```

---

### Passing Arguments Safely

Avoid string interpolation — pass arguments explicitly:

```js
const vm = new QuickJS();

const fn = `
  (greeting, name, count) => {
    return Array.from({ length: count }, () =>
      \`\${greeting}, \${name}!\`
    );
  }
`;

const result = await vm.eval(fn, {
  args: ['Hello', 'Developer', 3]
});

console.log(result);
// ["Hello, Developer!", "Hello, Developer!", "Hello, Developer!"]
```

---

### Global State

#### Static Globals (at startup)

```js
const vm = new QuickJS({
  globals: {
    version: '1.0.0',
    utils: {
      add: (a, b) => a + b,
      log: msg => console.log('[VM]', msg)
      getData: async (opts) => {
        // runs in Node context, callable from QuickJS
        return new Promise((res, rej) => {
          setTimeout(() => {
            res("Hello, Data")
          }, 100);
        })
      }
    }
  }
});

await vm.eval('utils.log(version); utils.add(10, 20)');
```

#### Dynamic Globals (runtime)

```js
await vm.setGlobal('user', {name: "John"}};
const name = await vm.eval('user.name');
```

---

### Advanced Configuration

```js
const vm = new QuickJS({
  maxMemoryBytes: 5 * 1024 * 1024,
  maxEvalMs: 500,
  // pass through vm console.XXX to NodeJS
  console: console
  // optional: provide your own handlers:
  // console: { log: ..., error: ... }
});

await vm.setGlobal('fetchUser', async id => {
  return db.users.find(id);
});

const user = await vm.eval('fetchUser(123)');
```

---

### ES Modules & Custom Imports

```js
const vm = new QuickJS({
  imports: path => {
    if (path === './math.js') {
      return 'export const add = (a, b) => a + b;';
    }
    throw new Error('Module not found');
  }
});

const result = await vm.evalModule(`
  import { add } from './math.js';
  export default add(10, 20);
`);
```

---

### Messaging (`postMessage`)

```js
vm.on('message', msg => {
  console.log('From VM:', msg);
});

vm.postMessage({ type: 'INIT' });
```

```js
// Inside QuickJS
on('message', msg => {
  if (msg.type === 'INIT') {
    postMessage({ status: 'READY' });
  }
});
```

---

## 🧠 When Should I Use This?

### `quickjs-vm` vs Node `vm`

| Node `vm`         | quickjs-vm                  |
| ----------------- | --------------------------- |
| Shares V8 runtime | Separate JS engine          |
| Limited isolation | Stronger isolation boundary |
| No memory limits  | Enforced memory caps        |
| Same event loop   | Independent execution       |

**Use `quickjs-vm`** when running untrusted or user-supplied code.

---

### `quickjs-vm` vs `isolated-vm`

| isolated-vm       | quickjs-vm             |
| ----------------- | ---------------------- |
| V8 isolates       | QuickJS runtime        |
| Higher overhead   | Lower memory footprint |
| Async-only APIs   | True sync execution    |
| Larger dependency | Smaller native core    |

**Use `quickjs-vm`** when you want deterministic sync execution, lower memory use, or simpler deployment.

---

### `quickjs-vm` vs WASM (emscripten)

| quickjs-emscripten    | quickjs-vm        |
| --------------------- | ----------------- |
| Interpreted execution | Native execution  |
| Async trampolines     | True sync         |
| Higher memory         | Lower overhead    |
| Slower startup        | Faster cold start |

**Use `quickjs-vm`** when performance, latency, and predictability matter.

---

## 🏗 Architecture & Lifecycle

### High-Level Architecture

```
┌─────────────┐
│   Node.js   │
│ Application │
└─────┬───────┘
      │ N-API (Rust)
┌─────▼───────────────────────────┐
│         quickjs-vm              │
│  ┌───────────────────────────┐  │
│  │  Control Thread (Node)    │◄─┼── eval / postMessage
│  └───────────┬───────────────┘  │
│              │ Channels         │
│  ┌───────────▼───────────────┐  │
│  │  QuickJS Runtime Thread   │  │
│  │  • QuickJS VM             │  │
│  │  • Memory limits          │  │
│  │  • Execution timeouts     │  │
│  └───────────┬───────────────┘  │
│              │ Messages         │
│  ┌───────────▼───────────────┐  │
│  │  Dispatcher / Callbacks   │──┼── JS callbacks
│  └───────────────────────────┘  │
└─────────────────────────────────┘
```

### Execution Lifecycle

1. **VM creation**

   * Native QuickJS runtime initialized
   * Memory and execution limits applied
   * Static globals injected

2. **Script execution**

   * Code is sent to the runtime thread
   * Execution is interrupted if limits are exceeded
   * Results are serialized back to Node

3. **Messaging**

   * `postMessage` enables async communication
   * Messages are queued and dispatched safely

4. **Shutdown**

   * `close()` signals all worker threads
   * Native resources are freed deterministically
   * No lingering handles

---

## 🧪 Testing

The project includes a comprehensive **Jest** test suite covering:

* Serialization correctness
* Sync vs async behavior
* Promise handling
* Module resolution
* Resource cleanup & leak detection
* Performance benchmarks

```bash
npm test
```

---

## 📄 License

MIT

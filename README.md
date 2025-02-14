# QuickJS-VM

[![npm version](https://badge.fury.io/js/quickjs-vm.svg)](https://badge.fury.io/js/quickjs-vm)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](#license)

A **QuickJS** runtime wrapper for **Node.js** that allows you to run untrusted ES6 code in a secure, sandboxed environment. Doesn't use WASM, compiles to a native module with Rust.

Features include:

- **Safely evaluate** untrusted Javascript (supports most of ES2023).
- **Worker-like API** and `eval` interface.  
- **Asynchronous** and **synchronous** code execution.  
- **Runtime resource limits** (memory, CPU time, stack size) to reduce attack surface.  
- **Argument passing** into the sandbox environment.  
- **Message passing** to/from the sandbox via event handlers.  
- **Bytecode** exporting and loading for more advanced usage.  
- **Global injection** of primitives, JSON objects, arrays, and sync functions.  

## Table of Contents

- [QuickJS-VM](#quickjs-vm)
  - [Table of Contents](#table-of-contents)
  - [Installation](#installation)
  - [Quick Start](#quick-start)
  - [Features](#features)
  - [Usage Examples](#usage-examples)
  - [Basic Eval](#basic-eval)
  - [Arguments and Return Values](#arguments-and-return-values)
  - [Asynchronous Operations](#asynchronous-operations)
  - [Message Passing](#message-passing)
  - [Promises \& Nested Promises](#promises--nested-promises)
  - [Memory Management and Garbage Collection](#memory-management-and-garbage-collection)
  - [Bytecode Export and Import](#bytecode-export-and-import)
- [API Reference](#api-reference)
  - [`QJSWorker(options)`](#qjsworkeroptions)
  - [Runtime Methods](#runtime-methods)
- [Security Considerations](#security-considerations)
- [Contributing](#contributing)
- [License](#license)


## Installation

```bash
npm install quickjs-vm
```

## Quick Start
```js
import { QJSWorker } from "quickjs-vm";

const runtime = QJSWorker({
  console: console,                // Forward QuickJS logs to Node console
  maxEvalMs: 1500,                 // Limit execution time for each eval
  maxMemoryBytes: 256 * 1_000_000, // Limit runtime memory
  maxStackSizeBytes: 10 * 1_000_000, // Limit stack size
  globals: {
    a_boolean: true,
    addTwo: (a, b) => a + b,
  },
});

(async () => {
  const [result] = await runtime.eval("addTwo(2, 3)");
  console.log(result); // 5

  await runtime.close();
})();
```

## Features

- Secure Sandbox: Each instance runs in its own thread and memory space, with optional time and memory limits.
- Simple API: A worker-like API for asynchronous operations and an evalSync for synchronous ones.
- Global Injection: Inject primitives, JSON objects, arrays, and sync functions into the QuickJS environment.
- Message Passing: Use on("message", ...) and postMessage(...) to communicate between Node.js and QuickJS.
- Promises & Async: Automatically resolves native JavaScript promises returned from QuickJS code.
- Bytecode Support: Export and load QuickJS bytecode for advanced or optimized usage scenarios.
- Resource Control: Configure maximum evaluation time, memory usage, and stack size to protect against runaway scripts.

## Usage Examples

Below is an example usage showing various features of quickjs-vm:
```js
import { QJSWorker } from "quickjs-vm";

const runtime = QJSWorker({
  console: {
    log: (...args) => {
      // capture console.log commands from the runtime
    },
  },
  // Alternatively, just provide Node console to pass them through
  // console: console,
  maxEvalMs: 1500, // limit execution time of each eval call
  maxMemoryBytes: 256 * 1000 * 1000, // limit runtime memory
  maxStackSizeBytes: 10 * 1000 * 1000, // limit stack size
  maxInterrupt: 10000, // prevent while(true) or similar lockups
  // inject globals into the runtime
  // supports primitives, JSON, arrays, and sync functions
  globals: {
    a_boolean: true,
    addTwo: (a, b) => a + b,
    json_value: {
      state: "Texas",
    },
    array_value: [
      { nested: "object" },
      { foo: "bar" },
    ],
    // Handle require() calls
    require: (moduleName) => {
      /* ... */
    },
  },
});

(async () => {
  const [evalResult, evalStats] = await runtime.eval("2 + 2");
  console.assert(evalResult == 4);

  const [evalResultSync] = runtime.evalSync("2 + 3");
  console.assert(evalResultSync == 5);

  const [addTwoResult] = await runtime.eval("addTwo(2, 3)");
  console.assert(addTwoResult == 5);

  // args can be passed directly
  const [addTwoResultWithArgs] = await runtime.eval(
    "addTwo",    // function name to call in the sandbox
    "script.js", // optional script name
    6,           // arg1
    6            // arg2
  );
  console.assert(addTwoResultWithArgs == 12);

  // return nested properties
  const [jsonValue] = await runtime.eval("json_value.state");
  console.assert(jsonValue == "Texas");

  // send a message to node from quickjs
  let receivedMessage;
  runtime.on("message", (msg) => {
    receivedMessage = msg;
  });
  await runtime.eval(`postMessage({hello: "from QuickJS"})`);
  console.assert(receivedMessage.hello == "from QuickJS");

  // send a message to quickjs from node
  await runtime.eval(`
    let messageFromNode;
    on('message', (msg) => { messageFromNode = msg; })
  `);
  await runtime.postMessage({ hello: "from Node" });
  const [message] = await runtime.eval("messageFromNode");
  console.assert(message.hello == "from Node");

  // Promises are resolved before returning their internal value.
  const [promise] = await runtime.eval("Promise.resolve({hello: 'world'})");
  console.assert(promise.hello == "world");

  // even nested promises are resolved
  const [nestedPromise] = await runtime.eval(
    "Promise.resolve(Promise.resolve({hello: 'world'}))"
  );
  console.assert(nestedPromise.hello == "world");

  // get memory stats
  const memStatus = await runtime.memory();
  // shows number of bytes currently being used
  console.log(memStats.memory_used_size); 

  // force garbage collector to run
  await runtime.gc();

  // bytecode, provides a Uint8Array
  const byteCode = await runtime.getByteCode(
    `const test = (a, b) => Promise.resolve(a+b)`
  );

  const runtime2 = QJSWorker();
  await runtime2.loadByteCode(byteCode);
  const [byteCodeFnResult] = await runtime2.eval("test(1, 2)");
  console.assert(byteCodeFnResult == 3);

  // make sure to close your runtimes or the node process will hang
  await runtime.close();
  await runtime2.close();
})();
```

## Basic Eval
```js
const [evalResult, evalStats] = await runtime.eval("2 + 2");
console.assert(evalResult === 4);
console.log(evalStats);

```

## Arguments and Return Values
```js
const [result] = await runtime.eval(
  "addTwo",    // function name to call in the sandbox
  "script.js", // optional script name
  6,           // arg1
  6            // arg2
);
console.assert(result === 12);
```

## Asynchronous Operations
```js
const [promiseResult] = await runtime.eval("Promise.resolve({ hello: 'world' })");
console.assert(promiseResult.hello === "world");
```

## Message Passing
From QuickJS to Node:
```js
runtime.on("message", (msg) => {
  console.log("Message from QuickJS:", msg);
});

await runtime.eval(`postMessage({ hello: "from QuickJS" })`);
```

And from Node to QuickJS:
```js
await runtime.eval(`
  let messageFromNode;
  on('message', (msg) => { messageFromNode = msg; });
`);

await runtime.postMessage({ hello: "from Node" });
const [msg] = await runtime.eval("messageFromNode");
console.assert(msg.hello === "from Node");
```

## Promises & Nested Promises
```js
const [nestedPromise] = await runtime.eval(
  "Promise.resolve(Promise.resolve({ hello: 'world' }))"
);
console.assert(nestedPromise.hello === "world");
```

## Memory Management and Garbage Collection
```js
// Retrieve memory usage stats
const memStats = await runtime.memory();
// shows number of bytes currently being used
console.log(memStats.memory_used_size); 

// Force garbage collection
await runtime.gc();
```

## Bytecode Export and Import
```js
const byteCode = await runtime.getByteCode(`const test = (a, b) => a + b;`);
const runtime2 = QJSWorker();
await runtime2.loadByteCode(byteCode);

const [byteCodeFnResult] = await runtime2.eval("test(1, 2)");
console.assert(byteCodeFnResult === 3);

await runtime2.close();
```

# API Reference

## `QJSWorker(options)`
Creates a new QuickJS runtime instance in its own thread/memory space.

**Options** (all optional):

- console `(object)`\
An object with methods like log, warn, error. Defaults to a no-op if not provided.

- maxEvalMs `(number)`\
Maximum evaluation time in milliseconds per call to eval.

- maxMemoryBytes `(number)`\
Maximum memory usage for the runtime (approximate).

- maxStackSizeBytes `(number)`\
Maximum stack size for the runtime (approximate).

- maxInterrupt `(number)`\
Interrupt count limit to break out of infinite loops.

- globals `(object)`\
Key-value pairs to inject into the global scope of QuickJS. Supports primitives, arrays, JSON objects, and sync functions.
```js
globals: {
  a_boolean: true,
  addTwo: (a, b) => a + b,
  json_value: { state: "Texas" },
  require: (moduleName) => { /* custom require logic */ },
}
```

## Runtime Methods
- `eval(code: string, fileName?: string, ...args: any[]): Promise<[any, EvalStats?]>`\
Evaluate JavaScript code asynchronously. Returns a Promise that resolves with `[result, evalStats]`.

- `evalSync(code: string, fileName?: string, ...args: any[]): [any, EvalStats?]`\
Synchronously evaluate JavaScript code. Blocks the Node.js event loop until completion.

- `on(eventName: string, callback: (msg: any) => void): void`\
Subscribe to a named event from the QuickJS environment (e.g., "message").

- `postMessage(message: any): Promise<void>`\
Send a message to the QuickJS environment, triggering any on('message') listeners inside the sandbox.

- `memory(): Promise<MemoryStats>`\
Retrieve memory usage statistics from the QuickJS runtime.

- `gc(): Promise<void>`\
Trigger garbage collection in the QuickJS runtime.

- `getByteCode(code: string): Promise<Uint8Array>`\
Compile code into QuickJS bytecode and return it.

- `loadByteCode(byteCode: Uint8Array): Promise<void>`\
Load previously exported QuickJS bytecode into the runtime.\

- `close(): Promise<void>`\
Destroy the QuickJS runtime instance and free all resources.

# Security Considerations

- While QuickJS itself is designed to be relatively secure, **no sandbox is 100% guaranteed** to be invulnerable.
- Use resource limits (maxEvalMs, maxMemoryBytes, maxStackSizeBytes, etc.) to mitigate denial-of-service attacks.
- Network, file system, or other Node.js APIs are not automatically exposed, unless explicitly injected in globals.
- Always be cautious when executing unknown or untrusted code, even within a sandbox.

# Contributing

Contributions are welcome! Here’s how you can help:

- Fork this repository.
- Create a feature branch for your patch, e.g. feature/my-new-feature.
- Write tests for your changes.
- Submit a Pull Request detailing your updates.
- 
For major changes, please open an issue first to discuss your proposal.

# License

This project is licensed under the MIT License. See the LICENSE file for more details.

Thanks for using QuickJS-VM! If you find it helpful, consider giving it a star on GitHub.

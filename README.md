# quickjs-worker
Run QuickJS as a worker from NodeJS

Run untrusted ES6 code in a secure QuickJS sandbox.  Includes worker like API and eval API.  Every instance runs in it's own thread and memory space.  Also supports loading and exporting bytecode.

Example:

```js
import { QJSWorker } from "quickjs-worker";

const runtime = QJSWorker({
    console: {
        log: (..args) => {
            // capture console.log commands from the runtime
        }
    }
    // or just provide Node console to pass them through
    // console: console,
    maxEvalMs: 1500, // limit execution time of each eval call
    maxMemoryBytes: 256 * 1000 * 1000, // limit runtime memory 
    maxStackSizeBytes: 10 * 1000 * 1000, // limit stack size
    maxInterrupt: 10000, // prevent while(true) or similar lockups
    // inject globals into the runtime
    // supports primitves, JSON, Arrays and sync functions
    staticGlobals: {
        a_boolean: true,
        addTwo: (a, b) => a + b,
        json_value: {
            state: "Texas"
        }
    }
});


runtime.on("message", (msg) => {
    // handle messages from the runtime
})

// send a message to the runtime, supports primitves, JSON and Arrays.
runtime.postMessage({key: "value"})

(async () => {

    const [evalResult, evalStats] = await runtime.eval("2 + 2");
    console.assert(evalResult == 4);
    const [evalResult, evalStats] = await runtime.eval("addTwo(2, 3)");
    console.assert(evalResult == 5);
    const [evalResult, evalStats] = await runtime.eval("json_value.state");
    console.assert(evalResult == "Texas");

    // Promises are resolved before returning the internal value.
    const [evalResult, evalStats] = await runtime.eval("Promise.resolve({hello: 'world'})");
    console.assert(evalResult.hello == "world");
    // even nested promises are resolved
    const [evalResult, evalStats] = await runtime.eval("Promise.resolve(Promise.resolve({hello: 'world'}))");
    console.assert(evalResult.hello == "world"); 

    // message passing
    await runtime.eval(`on('message' (msg) => { /* handle message */ })`);
    // send messages to Node
    await runtime.eval(`postMessage(["never", "gonna", "give"])`);

    // get memory stats
    const memStatus = await runtime.memory();
    // force garbage collector to run
    await runtime.gc();

    // byte code, provides a Uint8Array
    const byteCode = await runtime.getByteCode(`const test = (a, b) => Promise.resolve(a+b)`);
    const runtime2 = QJSWorker();
    await runtime2.loadByteCode(byteCode);
    const [evalResult, evalStats] = await runtime2.eval(`test(1, 2)`);
    console.assert(evalResult == 3);


    // make sure to close your runtimes or the node process will hang
    await runtime.close();
    await runtime2.close();
})()


```

Current major limitations:
- Alpha software, don't have any tests in place yet.
- Would like to support async functions in staticGlobals at some point.
- Does not support es6 modules `import` or `export` syntax.  
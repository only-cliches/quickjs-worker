# quickjs-worker
Run QuickJS as a worker from NodeJS

Run untrusted ES6 code in a secure QuickJS sandbox.  Includes worker like API and eval API.  Every instance runs in it's own thread and memory space.  Also supports loading and exporting bytecode.

Example:

```js
import { QJSWorker } from "quickjs-worker";

const runtime = QJSWorker({
    console: {
        log: (...args) => {
            // capture console.log commands from the runtime
        }
    },
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
        },
        array_value: [
            {nested: "object"},
            {foo: "bar"}
        ],
        // handle require() calls
        require: (moduleName: string) => {
            /* ... */
        }
    }
});




(async () => {

    const [evalResult, evalStats] = await runtime.eval("2 + 2");
    console.assert(evalResult == 4);
    const [evalResultSync] = runtime.evalSync("2 + 3");
    console.assert(evalResultSync == 5);

    const [addTwoResult] = await runtime.eval("addTwo(2, 3)");
    console.assert(addTwoResult == 5);

    // args can be passed directly
    const [addTwoResultWithArgs] = await runtime.eval("addTwo", "script_name.js", 6, 6);
    console.assert(addTwoResultWithArgs == 12);

    // return nested properties
    const [jsonValue] = await runtime.eval("json_value.state");
    console.assert(jsonValue == "Texas");

    // send a message to node from quickjs. Supports primitves, JSON and Arrays.
    let recievedMessage;
    runtime.on("message", (msg) => {
        recievedMessage = msg;
    })
    await runtime.eval(`postMessage({hello: "from QuickJS"})`)
    console.assert(recievedMessage.hello = "from QuickJS");

    // send a message to quickjs from node. Supports primitves, JSON and Arrays.
    await runtime.eval(`
        let messageFromNode;
        on('message', (msg) => { messageFromNode = msg; })
    `)
    await runtime.postMessage({hello: "from Node"})
    const [message] = await runtime.eval('messageFromNode');
    console.assert(message.hello == "from Node");

    // Promises are resolved before returning the internal value.
    const [promise] = await runtime.eval("Promise.resolve({hello: 'world'})");
    console.assert(promise.hello == "world");
    // even nested promises are resolved
    const [nestedPromise] = await runtime.eval("Promise.resolve(Promise.resolve({hello: 'world'}))");
    console.assert(nestedPromise.hello == "world"); 

    // get memory stats
    const memStatus = await runtime.memory();
    // force garbage collector to run
    await runtime.gc();

    // byte code, provides a Uint8Array
    const byteCode = await runtime.getByteCode(`const test = (a, b) => Promise.resolve(a+b)`);
    const runtime2 = QJSWorker();
    await runtime2.loadByteCode(byteCode);
    const [byteCodeFnResult] = await runtime2.eval(`test(1, 2)`);
    console.assert(byteCodeFnResult == 3);

    // make sure to close your runtimes or the node process will hang
    await runtime.close();
    await runtime2.close();
})()
```

Current major limitations:
- Would like to support async functions in globals at some point.
- Does not support es6 modules `import` or `export` syntax in the runtime.
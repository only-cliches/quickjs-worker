const { QJSWorker } = require(".");

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
    globals: {
        a_boolean: true,
        addTwo: (a, b) => a + b,
        json_value: {
            state: "Texas"
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

    // invoke function by name, passing in args from node
    const [addResult] = await runtime.eval("addTwo", "add.js", 1, 5);
    console.assert(addResult == 6);

    // invoke anonymous async function
    const [multResult] = await runtime.eval("async (a, b) => a*b;", "mult.js", 5, 5);
    console.assert(multResult == 25);

    // make sure to close your runtimes or the node process will hang
    await runtime.close();
    await runtime2.close();
})()
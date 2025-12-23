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
        addThree: async (a, b, c) => a + b + c,
        json_value: {
            state: "Texas"
        }
    }
});




(async () => {

    const [evalResult, evalStats] = await runtime.eval("addThree(1, 2, 3)");

    console.log(evalResult);
    await runtime.close();
})()
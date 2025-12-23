const { QJSWorker } = require(".");

(async () => {

    const runtime = QJSWorker({
        globals: {
            addThree: async (a, b, c) => a + b + c,
        }
    });
    const [evalResult, evalStats] = await runtime.eval("addThree(1, 2, 3)");

    console.log(evalResult);
    await runtime.close();
})()
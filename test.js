const q = require(".");

let counter = 0;
const quick = q.QuickJSWorker({
    // console: console,
    maxEvalMs: 1500,
    maxMemoryBytes: 256 * 1000 * 1000,
    maxStackSizeBytes: 10 * 1000 * 1000,
    maxInterrupt: 10000,
    // imports: (moduleName) => {
    //     console.log('IMPORTS', moduleName);
    //     return "export default ({})";
    // },
    // normalize: (refPath, path) => ""
    // setTimeout: true,
    // setImmediate: true,
    // maxMemoryBytes: 0,
    // maxStackSizeBytes: 0,
    // gcThresholdAlloc: 0.
    // gcIntervalMs: 0
});

const quick2 = q.QuickJSWorker({maxInterrupt: 1000});

quick.on("message", (msg) => {
    console.log("QUICKJS MSG", typeof msg, msg);
});


(async () => {

    let byteCode = await quick.getByteCode(`const test = (a, b) => Promise.resolve(a+b)`);
    await quick2.loadByteCode(byteCode);

    console.log(await quick2.eval(`new Promise((res, rej) => {
        setTimeout(() => res(200), 500);
    })`));

    // console.log(await quick.eval(`
    //     new Promise((res, rej) => {
    //         let k = 0;
    //         for (let i = 0; i < 500; i++) {
    //             k += 1;
    //         }
    //         setTimeout(() => res(k), 500);
    //     })
    // `));

    try {
        const d = new Date();
        await quick.eval("on('message', (msg) => { console.log([typeof msg, msg]) })");
        // await quick.eval(`postMessage(Object.properties(new Date()))`);
        // await quick.eval("console.log('hello')");
        // quick.postMessage("STRING BUDDY");
        
        console.log(await quick.eval(`
            new Promise((res) => {
                let count = 0;
                const loop = () => {
                    console.log(count++);
                    setTimeout(loop, 500);
                }
                loop();
            })
        `));
        
    } catch (e) {
        console.log("ERROR", e);
    }

    // await quick.eval(`
    //     import * as t from "test";
    //     console.log(t);
    // `)

    await quick.close();
    await quick2.close();
})()
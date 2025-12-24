import { getQuickJS } from "quickjs-emscripten"
import fs from "fs";
import { QuickJS } from "../index.js";

const benchScript = fs.readFileSync("./bench/v8-bench.js").toString();

async function main() {

    console.log("=== quickjs-vm ===\n");
    const runtime = new QuickJS({
        console: console,
        globals: {
            // pass through fs
            fs: fs
        }
    });
    const evalResult = await runtime.eval(benchScript);
    await runtime.close();

    console.log("\n\n=== quickjs-emscripten ===\n");
    const qjs = await getQuickJS();
    const vm = qjs.newContext()
    const logHandle = vm.newFunction("log", (...args) => {
        const nativeArgs = args.map(vm.dump)
        console.log(...nativeArgs)
    })
    // Partially implement `console` object
    const consoleHandle = vm.newObject()
    vm.setProp(consoleHandle, "log", logHandle)
    vm.setProp(vm.global, "console", consoleHandle)
    consoleHandle.dispose()
    logHandle.dispose()

    vm.evalCode(benchScript)
}

main()
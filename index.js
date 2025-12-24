const { QJSWorker } = require("./index.node");
const EventEmitter = require('events');

class QuickJS extends EventEmitter {
    constructor(options = {}) {
        super();
        this._native = QJSWorker(options);
        
        // Setup internal listeners to bridge events
        this._native.on('message', (msg) => this.emit('message', msg));
        this._native.on('close', () => this.emit('close'));
        
        this.lastExecutionStats = null;
    }

    /**
     * Helper to keep the Node.js event loop alive while waiting for the 
     * unref'd native worker to complete a task.
     */
    async _wrapNative(task) {
        // Create a no-op interval to act as a "ref" for the event loop
        const keepAlive = setInterval(() => {}, 1000 * 60 * 60);
        try {
            return await task();
        } finally {
            clearInterval(keepAlive);
        }
    }

    async eval(code, options = {}) {
        return this._wrapNative(async () => {
            const opts = this._normalizeOptions(options);
            const [result, stats] = await this._native.eval(code, opts);
            this.lastExecutionStats = stats;
            return result;
        });
    }

    evalSync(code, options = {}) {
        const opts = this._normalizeOptions(options);
        const [result, stats] = this._native.evalSync(code, opts);
        this.lastExecutionStats = stats;
        return result;
    }

    async evalModule(code, options = {}) {
        return this._wrapNative(async () => {
            const opts = this._normalizeOptions(options);
            opts.type = 'module';
            const [result, stats] = await this._native.eval(code, opts);
            this.lastExecutionStats = stats;
            return result;
        });
    }

    async setGlobal(key, value) {
        return this._wrapNative(() => this._native.setGlobal(key, value));
    }
    
    postMessage(msg) {
        this._native.postMessage(msg);
    }

    async getByteCode(code) {
        return this._wrapNative(() => this._native.getByteCode(code));
    }

    async loadByteCode(bytes) {
        return this._wrapNative(() => this._native.loadByteCode(bytes));
    }

    async gc() {
        return this._wrapNative(() => this._native.gc());
    }

    async memory() {
        return this._wrapNative(() => this._native.memory());
    }

    async close() {
        if (this.isClosed()) return;
        
        // 1. Call the native close (which sends Quit, drops Channel, resolves Promise)
        await this._wrapNative(() => this._native.close());
        
        // 2. Yield to the event loop to allow handle cleanup
        await new Promise(resolve => setTimeout(resolve, 10));
    }

    isClosed() {
        return this._native.isClosed();
    }

    // Support 'using' keyword
    async [Symbol.asyncDispose]() {
        if (!this.isClosed()) {
            await this.close();
        }
    }

    _normalizeOptions(options) {
        if (typeof options === 'string') {
            return { filename: options };
        }
        return options;
    }
}

module.exports = {
    QuickJS,
    QJSWorker: (opts) => new QuickJS(opts)
};
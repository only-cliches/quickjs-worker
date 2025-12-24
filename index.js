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

    async eval(code, options = {}) {
        const opts = this._normalizeOptions(options);
        const [result, stats] = await this._native.eval(code, opts);
        this.lastExecutionStats = stats;
        return result;
    }

    evalSync(code, options = {}) {
        const opts = this._normalizeOptions(options);
        const [result, stats] = this._native.evalSync(code, opts);
        this.lastExecutionStats = stats;
        return result;
    }

    async evalModule(code, options = {}) {
        const opts = this._normalizeOptions(options);
        opts.type = 'module';
        const [result, stats] = await this._native.eval(code, opts);
        this.lastExecutionStats = stats;
        return result;
    }

    async setGlobal(key, value) {
        return this._native.setGlobal(key, value);
    }
    
    postMessage(msg) {
        this._native.postMessage(msg);
    }

    async getByteCode(code) {
        return this._native.getByteCode(code);
    }

    async loadByteCode(bytes) {
        return this._native.loadByteCode(bytes);
    }

    async gc() {
        return this._native.gc();
    }

    async memory() {
        return this._native.memory();
    }

    async close() {
        return this._native.close();
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
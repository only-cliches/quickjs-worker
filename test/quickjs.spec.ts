import { QuickJS } from '../index'; 
import { EventEmitter } from 'events';

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

describe('QuickJS Integration Tests', () => {
    let qjs: QuickJS;

    afterEach(async () => {
        if (qjs && !qjs.isClosed()) {
            await qjs.close();
        }
    });
    
    // ... [Previous Instantiation / Basic Evaluation / Synchronous / Module / Globals tests remain same] ...
    describe('Instantiation', () => {
        
        it('should instantiate without errors', () => {
            qjs = new QuickJS();
            expect(qjs).toBeInstanceOf(QuickJS);
            expect(qjs).toBeInstanceOf(EventEmitter);
        });

        it('should accept options during instantiation', () => {
            qjs = new QuickJS({
                maxMemoryBytes: 1024 * 1024 * 10,
                maxEvalMs: 500
            });
            expect(qjs).toBeDefined();
        });
    });

    describe('Basic Evaluation (Async)', () => {
        beforeEach(() => {
            qjs = new QuickJS();
        });

        it('should evaluate simple javascript expressions', async () => {
            const result = await qjs.eval('1 + 1');
            expect(result).toBe(2);
        });

        it('should return strings', async () => {
            const result = await qjs.eval('"Hello " + "World"');
            expect(result).toBe('Hello World');
        });

        it('should return complex objects', async () => {
            const result = await qjs.eval('({ a: 1, b: "test" })');
            expect(result).toEqual({ a: 1, b: "test" });
        });

        it('should handle runtime errors gracefully', async () => {
            await expect(qjs.eval('throw new Error("test error")')).rejects.toBeDefined();
        });

        it('should respect timeout limits', async () => {
            await qjs.close();
            qjs = new QuickJS({ maxEvalMs: 100 });
            
            await expect(qjs.eval('while(true) {}')).rejects.toBeDefined();
        });

        it('should capture execution stats', async () => {
            await qjs.eval('1+1');
            const stats = qjs.lastExecutionStats;
            expect(stats).toBeDefined();
            expect(stats).toHaveProperty('cpuTimeMs');
            expect(stats).toHaveProperty('evalTimeMs');
        });
    });

    describe('Synchronous Evaluation', () => {
        beforeEach(() => {
            qjs = new QuickJS();
        });

        it('should evaluate synchronously', () => {
            const result = qjs.evalSync('2 * 5');
            expect(result).toBe(10);
        });

        it('should throw errors synchronously', () => {
            expect(() => {
                qjs.evalSync('throw new Error("sync error")');
            }).toThrow('sync error');
        });
    });

    describe('Module Support', () => {
        beforeEach(() => {
            qjs = new QuickJS();
        });

        it('should evaluate ES modules', async () => {
            const code = `
                export const x = 10;
                moduleReturn(x * 2); 
            `;
            const result = await qjs.evalModule(code);
            expect(result).toBe(20);
        });
        
        it('should handle imports if a loader is configured', async () => {
            qjs = new QuickJS({
                imports: (path: string) => {
                    if (path === './math.js') {
                        return 'export function add(a, b) { return a + b; }';
                    }
                    return '';
                }
            });

            const code = `
                import { add } from './math.js';
                moduleReturn(add(5, 7));
            `;
            const result = await qjs.evalModule(code);
            expect(result).toBe(12);
        });
    });

    describe('Globals and Data Injection', () => {
        beforeEach(() => {
            qjs = new QuickJS();
        });

        it('should set global variables', async () => {
            await qjs.setGlobal('myVar', 123);
            const result = await qjs.eval('myVar');
            expect(result).toBe(123);
        });

        it('should inject complex objects', async () => {
            await qjs.setGlobal('config', {
                active: true,
                limits: { max: 100 }
            });
            const result = await qjs.eval('config.limits.max');
            expect(result).toBe(100);
        });

        it('should inject functions that can be called from JS', async () => {
            const mockFn = jest.fn((x) => x * 2);
            await qjs.setGlobal('double', mockFn);

            const result = await qjs.eval('double(5)');
            expect(result).toBe(10);
            expect(mockFn).toHaveBeenCalledWith(5);
        });

        it('should handle async function injection (Promises)', async () => {
            const asyncFn = jest.fn(async (x) => {
                await sleep(50);
                return x + 1;
            });
            await qjs.setGlobal('addAsync', asyncFn);

            const result = await qjs.eval('(async () => await addAsync(10))()');
            expect(result).toBe(11);
        });
    });

    // -------------------------------------------------------------
    // UPDATED SECTION:
    // -------------------------------------------------------------
    describe('Communication (postMessage)', () => {
        beforeEach(() => {
            qjs = new QuickJS();
        });

        it('should receive messages from QuickJS', (done) => {
            qjs.on('message', (msg) => {
                try {
                    expect(msg).toBe('Hello from QJS');
                    done();
                } catch (e) {
                    done(e);
                }
            });

            qjs.eval('postMessage("Hello from QJS")');
        });

        it('should send messages to QuickJS', async () => {
            // Setup listener
            await qjs.eval(`
                let received = null;
                on('message', (msg) => { received = msg; });
            `);

            // Send message
            qjs.postMessage({ foo: 'bar' });

            await new Promise(r => setTimeout(r, 100));
            const result = await qjs.eval(`received`);
            expect(result).toEqual({ foo: 'bar' });
        });
    });
    // -------------------------------------------------------------

    describe('Bytecode', () => {
        beforeEach(() => {
            qjs = new QuickJS();
        });

        it('should compile to bytecode and reload', async () => {
            const source = '1 + 2';
            const bytecode = await qjs.getByteCode(source);
            expect(bytecode).toBeInstanceOf(Uint8Array);
            expect(bytecode.length).toBeGreaterThan(0);
            await qjs.loadByteCode(bytecode);
            
            const funcSource = 'globalThis.myFunc = () => 42;';
            const funcBytecode = await qjs.getByteCode(funcSource);
            await qjs.loadByteCode(funcBytecode);
            
            const result = await qjs.eval('myFunc()');
            expect(result).toBe(42);
        });
    });

    describe('Memory and GC', () => {
        beforeEach(() => {
            qjs = new QuickJS();
        });

        it('should return memory usage', async () => {
            const mem = await qjs.memory();
            const parsed = typeof mem === 'string' ? JSON.parse(mem) : mem;
            expect(parsed).toBeDefined();
            expect(Object.keys(parsed).length).toBeGreaterThan(0);
        });

        it('should run garbage collection', async () => {
            await expect(qjs.gc()).resolves.not.toThrow();
        });
    });

    describe('Resource Management', () => {
        it('should close resources explicitly', async () => {
            qjs = new QuickJS();
            expect(qjs.isClosed()).toBe(false);
            await qjs.close();
            expect(qjs.isClosed()).toBe(true);
            await expect(qjs.eval('1+1')).rejects.toThrow('Runtime is closed');
        });

        it('should support AsyncDispose (Symbol.asyncDispose)', async () => {
            {
                const worker = new QuickJS();
                try {
                    await worker.eval('1+1');
                } finally {
                    await worker[Symbol.asyncDispose]();
                }
                expect(worker.isClosed()).toBe(true);
            }
        });
    });
});
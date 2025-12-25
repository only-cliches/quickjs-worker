import { QuickJS } from '../index';
import fc from 'fast-check';

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));



describe('Promise Resolution Across Boundaries', () => {
    let qjs: QuickJS;

    beforeEach(() => {
        qjs = new QuickJS();
    });

    afterEach(async () => {
        if (qjs && !qjs.isClosed()) {
            await qjs.close();
        }
    });

    test('Invariant: Error Fidelity', async () => {
        await fc.assert(
            fc.asyncProperty(fc.string(), fc.string(), async (name, msg) => {
                // Ensure we don't generate empty strings if that's invalid for your use case
                if (!name || !msg) return;

                const script = `
                    (async () => {
                        const e = new Error(${JSON.stringify(msg)});
                        e.name = ${JSON.stringify(name)};
                        throw e;
                    })()
                `;

                try {
                    await qjs.eval(script);
                    throw new Error('Should have thrown');
                } catch (e: any) {
                    expect(e.constructor.name).toBe('Error');
                    expect(e.message).toBe(msg);
                    expect(e.name).toBe(name);
                }
            })
        );
    });

    describe('Node -> QuickJS (Injecting Async Functions)', () => {
        it('should resolve a Node.js Promise awaited inside QuickJS', async () => {
            const asyncIdentity = async (val: string) => {
                await sleep(50);
                return "Node processed: " + val;
            };

            await qjs.setGlobal('asyncNodeFn', asyncIdentity);

            const result = await qjs.eval(`
                (async () => {
                    const res = await asyncNodeFn("hello");
                    return res;
                })()
            `);

            expect(result).toBe("Node processed: hello");
        });

        it('should handle rejected Node.js Promises inside QuickJS', async () => {
            const asyncError = async () => {
                await sleep(20);
                throw new Error("Node Boom!");
            };

            await qjs.setGlobal('asyncError', asyncError);

            const result = await qjs.eval(`
                (async () => {
                    try {
                        await asyncError();
                        return "did not fail";
                    } catch (e) {
                        return String(e);
                    }
                })()
            `);

            expect(result).toContain("Node Boom!");
        });
    });

    describe('QuickJS -> Node (Returning Promises from Script)', () => {
        it('should resolve a Promise returned by QuickJS to Node', async () => {
            const script = `
                Promise.resolve("step 1")
                    .then(val => val + " -> step 2")
                    .then(val => val + " -> step 3");
            `;

            const result = await qjs.eval(script);
            expect(result).toBe("step 1 -> step 2 -> step 3");
        });

        it('should reject Node promise if QuickJS returns a rejected Promise', async () => {
            // Direct Promise.reject()
            const script = `Promise.reject(new Error("Direct Rejection"))`;
            await expect(qjs.eval(script)).rejects.toThrow("Direct Rejection");
        });

        it('should reject Node promise if QuickJS async function throws', async () => {
            // Async function throwing
            const script = `
                (async () => {
                    throw new Error("Async Throw");
                })()
            `;
            await expect(qjs.eval(script)).rejects.toThrow("Async Throw");
        });

        it('should handle mixed Async/Await chains', async () => {
            await qjs.setGlobal('nodeHelper', async (n: number) => n * 2);

            const script = `
                (async () => {
                    const val = await nodeHelper(10);
                    return val + 5;
                })()
            `;

            const result = await qjs.eval(script);
            expect(result).toBe(25);
        });
    });
    describe('Error Propagation Across Boundary', () => {
        describe('QuickJS -> Node (rejections)', () => {
            it('should reject with a real Error instance (preserve message)', async () => {
                try {
                    await qjs.eval(`Promise.reject(new Error("Direct Rejection"))`);
                    throw new Error("Expected rejection");
                } catch (e: any) {
                    expect(e.message).toContain("Direct Rejection");
                }
            });

            it('should preserve Error.name when QuickJS sets a custom name', async () => {
                const script = `
        const err = new Error("Boom");
        err.name = "MyQuickJSError";
        Promise.reject(err);
      `;
                try {
                    await qjs.eval(script);
                    throw new Error("Expected rejection");
                } catch (e: any) {
                    // expect(e).toBeInstanceOf(Error);
                    expect(e.name).toBe("MyQuickJSError");
                    expect(e.message).toContain("Boom");
                }
            });

            it('should include stack when available (non-brittle)', async () => {
                try {
                    await qjs.eval(`Promise.reject(new Error("Stack Me"))`);
                    throw new Error("Expected rejection");
                } catch (e: any) {
                    // stack can vary by engine/version; keep this loose
                    // expect(e).toBeInstanceOf(Error);
                    if (typeof e.stack === "string") {
                        expect(e.stack).toContain("Stack Me");
                    }
                }
            });

            it('should reject with raw string values if QuickJS rejects a string', async () => {
                await expect(qjs.eval(`Promise.reject("string-reject")`)).rejects.toBe("string-reject");
            });

            it('should reject with raw objects if QuickJS rejects a plain object', async () => {
                const script = `Promise.reject({ kind: "E_OBJ", message: "object-reject", code: 123 })`;
                await expect(qjs.eval(script)).rejects.toMatchObject({
                    kind: "E_OBJ",
                    message: "object-reject",
                    code: 123,
                });
            });
        });

        describe('Node -> QuickJS (thrown/rejected from injected functions)', () => {
            it('should surface Node Error.message + name inside QuickJS catch', async () => {
                const nodeFn = async () => {
                    const err: any = new Error("Node Boom!");
                    err.name = "NodeCustomError";
                    err.code = "E_NODE";
                    throw err;
                };

                await qjs.setGlobal("nodeFn", nodeFn);

                const result = await qjs.eval(`
        (async () => {
          try {
            await nodeFn();
            return "did not fail";
          } catch (e) {
            return {
              name: e && e.name,
              message: e && e.message,
              code: e && e.code,
              stackType: typeof (e && e.stack),
            };
          }
        })()
      `);

                expect(result).toMatchObject({
                    name: "NodeCustomError",
                    message: "Node Boom!",
                    code: "E_NODE",
                });
                // stack is optional but should usually be a string if present
                expect(["string", "undefined"]).toContain((result as any).stackType);
            });

            it('should propagate non-Error rejections from Node into QuickJS (string)', async () => {
                await qjs.setGlobal("nodeRejectString", async () => Promise.reject("NOPE"));

                const result = await qjs.eval(`
        (async () => {
          try {
            await nodeRejectString();
            return "did not fail";
          } catch (e) {
            return { value: e, type: typeof e };
          }
        })()
      `);

                expect(result).toEqual({ value: "NOPE", type: "string" });
            });

            it('should propagate non-Error rejections from Node into QuickJS (object)', async () => {
                await qjs.setGlobal("nodeRejectObj", async () =>
                    Promise.reject({ kind: "E_NODE_OBJ", message: "bad", code: 999 })
                );

                const result = await qjs.eval(`
                (async () => {
                try {
                    await nodeRejectObj();
                    return "did not fail";
                } catch (e) {
                    return e;
                }
                })()
            `);

                expect(result).toMatchObject({
                    kind: "E_NODE_OBJ",
                    message: "bad",
                    code: 999,
                });
            });
        });
    });


});
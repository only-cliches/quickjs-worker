import { QuickJS } from '../index'; 

describe('Data Serialization & Eval Arguments', () => {
    let qjs: QuickJS;

    beforeEach(() => {
        qjs = new QuickJS();
    });

    afterEach(async () => {
        if (qjs && !qjs.isClosed()) {
            await qjs.close();
        }
    });

    describe('Round Trip Data Serialization', () => {
        const identityScript = '(x) => x';

        it('should serialize Strings correctly', async () => {
            const input = "Hello, 🌍!";
            const result = await qjs.eval(identityScript, { args: [input] });
            expect(result).toBe(input);
        });

        it('should serialize Numbers correctly', async () => {
            const inputInt = 42;
            const inputFloat = 3.14159;
            const resInt = await qjs.eval(identityScript, { args: [inputInt] });
            const resFloat = await qjs.eval(identityScript, { args: [inputFloat] });
            
            expect(resInt).toBe(inputInt);
            expect(resFloat).toBe(inputFloat);
        });

        it('should serialize Booleans correctly', async () => {
            expect(await qjs.eval(identityScript, { args: [true] })).toBe(true);
            expect(await qjs.eval(identityScript, { args: [false] })).toBe(false);
        });

        it('should serialize Null correctly', async () => {
            const result = await qjs.eval(identityScript, { args: [null] });
            expect(result).toBeNull();
        });

        it('should serialize Undefined correctly', async () => {
            const result = await qjs.eval(identityScript, { args: [undefined] });
            expect(result).toBeUndefined();
        });

        it('should serialize Arrays correctly', async () => {
            const input = [1, "two", true, null];
            const result = await qjs.eval(identityScript, { args: [input] });
            expect(result).toEqual(input);
        });

        it('should serialize JSON Objects correctly', async () => {
            const input = { 
                foo: "bar", 
                nested: { 
                    val: 123 
                } 
            };
            const result = await qjs.eval(identityScript, { args: [input] });
            expect(result).toEqual(input);
        });

        it('should serialize Dates correctly', async () => {
            const input = new Date("2023-01-01T12:00:00Z");
            const result = await qjs.eval(identityScript, { args: [input] });
            
            // NOTE: Passed as a direct argument, the native bridge preserves the Date type.
            // We use Duck Typing check because cross-context instances can confuse 'instanceof'
            expect(result).not.toBeNull();
            expect(typeof result).toBe('object');
            expect(typeof (result as Date).toISOString).toBe('function');
            expect((result as Date).toISOString()).toBe(input.toISOString());
        });

        it('should serialize Buffers (Uint8Array) correctly', async () => {
            const input = Buffer.from([0x01, 0x02, 0xff]);
            const result = await qjs.eval(identityScript, { args: [input] });
            
            expect(Buffer.isBuffer(result) || result instanceof Uint8Array).toBe(true);
            expect(Buffer.compare(input, result as Buffer)).toBe(0);
        });
        
        it('should handle complex mixed structures (via JSON serialization)', async () => {
            const input = {
                a: [1, 2],
                b: "string",
                c: new Date(1000), 
                d: Buffer.from([10])
            };
            const result = await qjs.eval(identityScript, { args: [input] });
            
            expect(result.a).toEqual([1, 2]);
            expect(result.b).toBe("string");
            
            // NOTE: Nested objects are currently serialized via JSON.stringify/parse.
            // Dates become ISO strings, and Buffers/Uint8Arrays become standard JSON objects/arrays.
            expect(typeof result.c).toBe('string');
            expect(result.c).toBe(input.c.toISOString());
            
            // Buffers inside objects serialize to JSON object structure { type: 'Buffer', data: [...] } or regular arrays depending on Node version behavior with JSON.stringify
            expect(result.d).toBeDefined();
        });
    });

    describe('Function Arguments in Eval', () => {
        it('should pass multiple arguments in correct order', async () => {
            const script = '(a, b, c) => [a, b, c]';
            const args = [1, "second", true];
            
            const result = await qjs.eval(script, { args });
            
            expect(result).toHaveLength(3);
            expect(result[0]).toBe(1);
            expect(result[1]).toBe("second");
            expect(result[2]).toBe(true);
        });

        it('should handle empty argument lists', async () => {
            const script = '() => "called"';
            const result = await qjs.eval(script, { args: [] });
            expect(result).toBe("called");
        });

        it('should pass arguments to evalSync as well', () => {
            const script = '(a, b) => a + b';
            const result = qjs.evalSync(script, { args: [10, 20] });
            expect(result).toBe(30);
        });

        it('should allow arguments to be modified inside QuickJS without affecting Node original', async () => {
            const inputObj = { val: 1 };
            const script = '(obj) => { obj.val = 999; return obj; }';
            
            const result = await qjs.eval(script, { args: [inputObj] });
            
            // Result from QJS should have new value
            expect(result.val).toBe(999);
            // Original Node object should remain untouched (serialization copies data)
            expect(inputObj.val).toBe(1);
        });
    });
});
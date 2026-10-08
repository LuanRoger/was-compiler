'use strict';

const { withStage, MAX_CALL_ARGS } = require('./abi.cjs');

function instantiate(wasmBytes, imports = {}) {
    const module = withStage('compilation', 'cannot compile WebAssembly bytes', () => new WebAssembly.Module(wasmBytes));
    const instance = withStage('instantiation', 'cannot instantiate WebAssembly module', () => new WebAssembly.Instance(module, imports));

    function call(name, args = []) {
        return withStage('invocation', `cannot call ${typeof name === 'string' ? JSON.stringify(name) : 'entry'}`, () => {
            if (typeof name !== 'string') throw new TypeError('entry name must be a string');
            const entry = instance.exports[name];
            if (typeof entry !== 'function') throw new TypeError(`${JSON.stringify(name)} is not a function export`);
            if (!Array.isArray(args)) throw new TypeError(`arguments for ${name} must be an array`);
            if (args.length > MAX_CALL_ARGS) throw new RangeError(`calls are limited to ${MAX_CALL_ARGS} arguments`);
            if (args.length !== entry.length) throw new TypeError(`${name} expects ${entry.length} arguments, got ${args.length}`);
            for (let i = 0; i < args.length; i++) {
                if (typeof args[i] !== 'number' && typeof args[i] !== 'bigint') {
                    throw new TypeError(`argument ${i + 1} must be a raw numeric scalar (number or bigint); encode strings and arrays as descriptor handles explicitly`);
                }
            }
            return Reflect.apply(entry, undefined, args);
        });
    }

    return { instance, call };
}

module.exports = { instantiate };

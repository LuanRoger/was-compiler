'use strict';

const { Buffer } = require('node:buffer');
const { TextDecoder, TextEncoder } = require('node:util');

const MAX_COPY_BYTES = 64 * 1024 * 1024;
const MAX_CALL_ARGS = 1024;
const ADDRESS_SPACE = 0x100000000;
const DESCRIPTOR_SIZE = 8;
const DESCRIPTOR_DATA_OFFSET = 0;
const DESCRIPTOR_LENGTH_OFFSET = 4;
const DESCRIPTOR_ALIGNMENT = 8;
const NUMBER_ELEMENT_SIZE = 8;
const STRING_ELEMENT_SIZE = 1;
const BOOL_ELEMENT_SIZE = 1;
const STRING_HANDLE_SIZE = 4;
const MIN_ADDRESS = 8;
const decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
const encoder = new TextEncoder();

class RuntimeError extends Error {
    constructor(stage, message, cause) {
        super(message, cause === undefined ? undefined : { cause });
        this.name = 'RuntimeError';
        this.stage = stage;
    }
}

function withStage(stage, context, operation) {
    try {
        return operation();
    } catch (error) {
        if (error instanceof RuntimeError) throw error;
        const detail = error instanceof Error ? error.message : String(error);
        throw new RuntimeError(stage, `${context}: ${detail}`, error);
    }
}

function copySize(bytes) {
    if (!Number.isSafeInteger(bytes) || bytes < 0 || bytes > MAX_COPY_BYTES) {
        throw new RangeError(`value exceeds the ${MAX_COPY_BYTES}-byte runtime copy limit`);
    }
    return bytes;
}

function unsignedHandle(value) {
    if (!Number.isInteger(value) || value < -0x80000000 || value >= ADDRESS_SPACE) {
        throw new TypeError('descriptor handle must be a 32-bit integer');
    }
    return value >>> 0;
}

function range(buffer, offset, bytes, alignment, label) {
    if (!Number.isSafeInteger(offset) || offset < MIN_ADDRESS || offset >= ADDRESS_SPACE) {
        throw new RangeError(`${label} must be an unsigned address at least 8`);
    }
    if (offset % alignment !== 0) {
        throw new RangeError(`${label} must be ${alignment}-byte aligned`);
    }
    if (!Number.isSafeInteger(bytes) || bytes < 0 ||
        offset + bytes > ADDRESS_SPACE || offset + bytes > buffer.byteLength) {
        throw new RangeError(`${label} range is out of bounds of module memory`);
    }
}

function readDescriptor(memory, handle, width) {
    if (!(memory instanceof WebAssembly.Memory)) {
        throw new TypeError('descriptor-v1 requires an exported WebAssembly.Memory named memory');
    }
    if (width !== STRING_ELEMENT_SIZE && width !== STRING_HANDLE_SIZE && width !== NUMBER_ELEMENT_SIZE) {
        throw new TypeError('descriptor element width must be 1, 4 or 8');
    }
    const buffer = memory.buffer;
    const address = unsignedHandle(handle);
    range(buffer, address, DESCRIPTOR_SIZE, DESCRIPTOR_ALIGNMENT, 'descriptor');
    const view = new DataView(buffer);
    const data = view.getUint32(address + DESCRIPTOR_DATA_OFFSET, true);
    const length = view.getUint32(address + DESCRIPTOR_LENGTH_OFFSET, true);
    const bytes = length * width;
    range(buffer, data, bytes, width, 'descriptor payload');
    copySize(bytes);
    return { buffer, view, data, length, bytes };
}

function snapshotArray(type, value, label) {
    if (!Array.isArray(value)) throw new TypeError(`${label} must be ${type}`);
    const length = value.length;
    const isString = type === 'string[]';
    let bytes = copySize(length * (isString ? STRING_HANDLE_SIZE : BOOL_ELEMENT_SIZE));
    const snapshot = new Array(length);
    for (let i = 0; i < length; i++) {
        const element = value[i];
        if (typeof element !== (isString ? 'string' : 'boolean')) {
            throw new TypeError(`${label}[${i}] must be ${isString ? 'string' : 'bool'}`);
        }
        if (isString) bytes = copySize(bytes + Buffer.byteLength(element, 'utf8'));
        snapshot[i] = element;
    }
    return snapshot;
}

function validateNative(type, value, label = 'value') {
    switch (type) {
        case 'number':
            if (typeof value === 'number') return;
            break;
        case 'bool':
            if (typeof value === 'boolean') return;
            break;
        case 'string':
            if (typeof value === 'string') {
                copySize(Buffer.byteLength(value, 'utf8'));
                return;
            }
            break;
        case 'number[]':
            if (Array.isArray(value)) {
                copySize(value.length * NUMBER_ELEMENT_SIZE);
                for (let i = 0; i < value.length; i++) {
                    if (typeof value[i] !== 'number') {
                        throw new TypeError(`${label}[${i}] must be number`);
                    }
                }
                return;
            }
            break;
        case 'string[]':
        case 'bool[]':
            snapshotArray(type, value, label);
            return;
        case 'void':
            if (value === undefined) return;
            break;
        default:
            throw new TypeError(`unsupported ABI type ${JSON.stringify(type)}`);
    }
    throw new TypeError(`${label} must be ${type}${type === 'void' ? ' (undefined)' : ''}`);
}

function createAbi(getInstance) {
    function memoryExports() {
        const instance = getInstance();
        if (!instance) {
            throw new Error('module instance is unavailable; a Start function cannot use the memory ABI');
        }
        const exports = instance.exports;
        if (!(exports.memory instanceof WebAssembly.Memory) || typeof exports.__alloc !== 'function') {
            throw new TypeError('descriptor-v1 requires exports memory and __alloc');
        }
        return exports;
    }

    function encode(type, value, label = 'value') {
        if (type === 'bool[]' || type === 'string[]') {
            // Accessors may change values or length; encode only the checked scalar snapshot.
            value = snapshotArray(type, value, label);
        } else {
            validateNative(type, value, label);
        }
        if (type === 'number' || type === 'void') return value;
        if (type === 'bool') return value ? 1 : 0;
        const exports = memoryExports();
        const payload = type === 'string' ? encoder.encode(value) : value;
        const width = type === 'string' ? STRING_ELEMENT_SIZE :
            type === 'bool[]' ? BOOL_ELEMENT_SIZE :
                type === 'string[]' ? STRING_HANDLE_SIZE : NUMBER_ELEMENT_SIZE;
        const bytes = copySize(payload.length * width);
        const handle = unsignedHandle(exports.__alloc(DESCRIPTOR_SIZE + bytes));
        const buffer = exports.memory.buffer;
        range(buffer, handle, DESCRIPTOR_SIZE + bytes, DESCRIPTOR_ALIGNMENT, 'allocator result');
        const view = new DataView(buffer);
        const data = handle + DESCRIPTOR_SIZE;
        view.setUint32(handle + DESCRIPTOR_DATA_OFFSET, data, true);
        view.setUint32(handle + DESCRIPTOR_LENGTH_OFFSET, payload.length, true);
        if (type === 'string[]') {
            for (let i = 0; i < payload.length; i++) {
                const stringHandle = encode('string', payload[i], `${label}[${i}]`);
                // Nested allocations may detach every earlier view, not the saved offsets.
                new DataView(exports.memory.buffer).setUint32(data + i * STRING_HANDLE_SIZE, stringHandle, true);
            }
        } else if (type === 'bool[]') {
            const target = new Uint8Array(buffer, data, bytes);
            for (let i = 0; i < payload.length; i++) target[i] = payload[i] ? 1 : 0;
        } else if (type === 'string') {
            new Uint8Array(buffer, data, bytes).set(payload);
        } else {
            for (let i = 0; i < payload.length; i++) {
                view.setFloat64(data + i * NUMBER_ELEMENT_SIZE, payload[i], true);
            }
        }
        return handle;
    }

    function decode(type, value, label = 'value') {
        if (type === 'number' || type === 'void') {
            validateNative(type, value, label);
            return value;
        }
        if (type === 'bool') {
            if (value !== 0 && value !== 1) {
                throw new TypeError(`${label} must be ABI bool (0 or 1)`);
            }
            return value === 1;
        }
        if (type !== 'string' && type !== 'number[]' && type !== 'bool[]' && type !== 'string[]') {
            throw new TypeError(`unsupported ABI type ${JSON.stringify(type)}`);
        }
        const memory = memoryExports().memory;
        const descriptor = readDescriptor(
            memory, value,
            type === 'string' ? STRING_ELEMENT_SIZE :
                type === 'bool[]' ? BOOL_ELEMENT_SIZE :
                    type === 'string[]' ? STRING_HANDLE_SIZE : NUMBER_ELEMENT_SIZE);
        if (type === 'string') {
            return decoder.decode(new Uint8Array(descriptor.buffer, descriptor.data, descriptor.bytes));
        }
        if (type === 'string[]') {
            // Check the entire logical copy before allocating native results or decoding text.
            let bytes = descriptor.bytes;
            for (let i = 0; i < descriptor.length; i++) {
                const handle = descriptor.view.getUint32(descriptor.data + i * STRING_HANDLE_SIZE, true);
                const string = readDescriptor(memory, handle, STRING_ELEMENT_SIZE);
                bytes = copySize(bytes + string.bytes);
            }
        }
        const result = new Array(descriptor.length);
        for (let i = 0; i < result.length; i++) {
            if (type === 'string[]') {
                const handle = descriptor.view.getUint32(descriptor.data + i * STRING_HANDLE_SIZE, true);
                result[i] = decode('string', handle, `${label}[${i}]`);
            } else if (type === 'bool[]') {
                const byte = descriptor.view.getUint8(descriptor.data + i);
                if (byte !== 0 && byte !== 1) {
                    throw new TypeError(`${label}[${i}] must be ABI bool (0 or 1)`);
                }
                result[i] = byte === 1;
            } else {
                result[i] = descriptor.view.getFloat64(descriptor.data + i * NUMBER_ELEMENT_SIZE, true);
            }
        }
        return result;
    }

    return { encode, decode };
}

module.exports = { createAbi, readDescriptor, validateNative, RuntimeError, withStage, MAX_CALL_ARGS, MAX_COPY_BYTES };

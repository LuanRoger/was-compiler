'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { instantiate } = require('./index.cjs');
const { RuntimeError, withStage } = require('./abi.cjs');

const HELP = `Usage: node runtime/run.cjs PROGRAM.wasm [--entry NAME] [--args JSON_ARRAY] [--imports PROVIDER.cjs] [--help]

Defaults: --entry main --args []
The CommonJS provider exports raw imports or a factory getInstance => rawImports.
Program and provider paths resolve from the current working directory.
No implicit env or WASI imports are supplied. Providers execute trusted Node.js code.
--args is a JSON array of numeric scalars passed directly to WebAssembly.
Use 0/1 for bool and explicit descriptor handles for strings and number[].
Native strings, arrays, booleans, and objects are rejected; no types are inferred.
Raw results print directly, including descriptor handles. Void results print nothing.
Provider factories receive a getter for the instance, unavailable during Start callbacks.
`;

function parseArguments(argv) {
    return withStage('invocation', 'invalid runner arguments', () => {
        if (argv.includes('--help')) return { help: true };
        if (argv.length === 0 || argv[0].startsWith('--')) {
            throw new Error('PROGRAM.wasm is required; use --help for usage');
        }
        const options = { program: argv[0], entry: 'main', args: [], provider: undefined };
        const seen = new Set();
        for (let i = 1; i < argv.length; i += 2) {
            const flag = argv[i];
            if (!['--entry', '--args', '--imports'].includes(flag)) throw new Error(`unknown option ${flag}; use --help for usage`);
            if (seen.has(flag)) throw new Error(`duplicate option ${flag}`);
            seen.add(flag);
            const value = argv[i + 1];
            if (value === undefined || value.startsWith('--')) throw new Error(`${flag} requires a value`);
            if (flag === '--entry') {
                if (value.length === 0) throw new Error('--entry requires a nonempty name');
                options.entry = value;
            } else if (flag === '--args') {
                options.args = JSON.parse(value);
                if (!Array.isArray(options.args)) throw new Error('--args must be a JSON array');
                if (options.args.some(value => typeof value !== 'number')) {
                    throw new TypeError('--args elements must be raw numeric scalars; native strings, arrays, booleans, and objects are not supported');
                }
            } else {
                if (value.length === 0) throw new Error('--imports requires a filesystem path');
                options.provider = value;
            }
        }
        return options;
    });
}

function main(argv = process.argv.slice(2)) {
    try {
        const options = parseArguments(argv);
        if (options.help) {
            process.stdout.write(HELP);
            return 0;
        }
        const bytes = withStage('file', `cannot read program ${JSON.stringify(options.program)}`, () => fs.readFileSync(path.resolve(options.program)));
        const provider = options.provider === undefined ? {} : withStage('file', `cannot load import provider ${JSON.stringify(options.provider)}`, () => require(path.resolve(options.provider)));
        let runtime;
        const imports = withStage('instantiation', 'cannot prepare raw imports', () =>
            typeof provider === 'function' ? provider(() => runtime && runtime.instance) : provider);
        runtime = instantiate(bytes, imports);
        const result = runtime.call(options.entry, options.args);
        if (result !== undefined) console.log(result);
        return 0;
    } catch (error) {
        const stage = error instanceof RuntimeError ? error.stage : 'invocation';
        const detail = error instanceof Error ? error.message : String(error);
        console.error(`${stage}: ${detail}`);
        return 1;
    }
}

if (require.main === module) process.exitCode = main();

module.exports = { main };

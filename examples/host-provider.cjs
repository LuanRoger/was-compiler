'use strict';

const { createAbi } = require('../runtime/abi.cjs');

module.exports = getInstance => {
    const abi = createAbi(getInstance);
    return {
        env: {
            echo(handle) {
                return abi.encode('string', abi.decode('string', handle) + ' from host');
            },
            print(handle) {
                console.log(abi.decode('string', handle));
            },
            adjust(handle) {
                const values = abi.decode('number[]', handle);
                return abi.encode('number[]', values.map(value => value + 1));
            },
        },
    };
};

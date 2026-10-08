import init, { Compiler } from "../../pkg/was_compiler.js";

self.addEventListener("message", async ({ data }) => {
    const logs = [];
    let logsTruncated = false;
    let phase = "Compiler initialization";

    try {
        await init();
        phase = "Compilation";
        const compiler = new Compiler();
        let bytes;
        let wat = null;

        try {
            compiler.setExports(["main"]);
            compiler.setOptimizationLevel(data.optimization);
            if (data.hostPrint) {
                compiler.registerHostFunction("print", "env", "log_number", ["number"], "void");
            }
            bytes = compiler.compile(data.source).slice();
            if (data.showWat) {
                wat = compiler.compileWat(data.source);
            }
        } finally {
            compiler.free();
        }

        const downloadable = bytes.slice();
        self.postMessage({ type: "compiled", bytes: downloadable, wat }, [downloadable.buffer]);

        phase = "Instantiation";
        const imports = data.hostPrint ? {
            env: {
                log_number(value) {
                    if (logs.length < 1000) {
                        logs.push(String(value));
                    } else {
                        logsTruncated = true;
                    }
                },
            },
        } : {};
        const { instance } = await WebAssembly.instantiate(bytes, imports);

        phase = "Execution";
        const main = instance.exports.main;
        if (typeof main !== "function" || main.length !== 0) {
            throw new Error("The demo requires an exported zero-argument main() function.");
        }
        const result = main();
        if (typeof result !== "number") {
            throw new Error("The demo requires main() to return a number.");
        }
        self.postMessage({ type: "result", result: String(result), logs, logsTruncated });
    } catch (error) {
        self.postMessage({
            type: "error",
            phase,
            message: error instanceof Error ? error.message : String(error),
            logs,
            logsTruncated,
        });
    }
});

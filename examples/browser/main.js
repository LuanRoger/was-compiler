const form = document.getElementById("playground");
const source = document.getElementById("source");
const hostPrint = document.getElementById("host-print");
const showWat = document.getElementById("show-wat");
const optimization = document.getElementById("optimization");
const run = document.getElementById("run");
const stop = document.getElementById("stop");
const download = document.getElementById("download");
const status = document.getElementById("status");
const output = document.getElementById("output");
const wat = document.getElementById("wat");

let worker = null;
let timer = null;
let downloadUrl = null;

function finish() {
    clearTimeout(timer);
    timer = null;
    worker?.terminate();
    worker = null;
    run.disabled = false;
    stop.disabled = true;
}

function setDeadline(milliseconds, message) {
    clearTimeout(timer);
    timer = setTimeout(() => {
        finish();
        status.textContent = "Timed out; worker terminated.";
        output.textContent = message;
    }, milliseconds);
}

function clearArtifact() {
    if (downloadUrl !== null) {
        URL.revokeObjectURL(downloadUrl);
        downloadUrl = null;
    }
    download.disabled = true;
}

form.addEventListener("submit", (event) => {
    event.preventDefault();
    finish();
    clearArtifact();
    output.textContent = "Loading compiler and compiling source…";
    wat.textContent = showWat.checked ? "Waiting for compilation…" : "WAT generation disabled.";
    status.textContent = "Loading / compiling…";
    run.disabled = true;
    stop.disabled = false;

    try {
        const current = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
        worker = current;

        current.addEventListener("message", ({ data }) => {
            if (worker !== current) {
                return;
            }
            if (data.type === "compiled") {
                downloadUrl = URL.createObjectURL(new Blob([data.bytes], { type: "application/wasm" }));
                download.disabled = false;
                wat.textContent = data.wat ?? "WAT generation disabled.";
                status.textContent = `Compiled ${data.bytes.byteLength} bytes. Running main()…`;
                output.textContent = "Compiled successfully. Instantiating and running main()…";
                setDeadline(3000, "Execution exceeded 3 seconds; worker terminated. Compiled Wasm and WAT remain available.");
            } else if (data.type === "result") {
                const logs = data.logs.length ? data.logs.join("\n") : "(no host calls)";
                output.textContent = `main() → ${data.result}\n\nHost log:\n${logs}${data.logsTruncated ? "\n(log limit reached)" : ""}`;
                status.textContent = "Finished.";
                finish();
            } else if (data.type === "error") {
                const logs = data.logs.length ? `\n\nHost log before failure:\n${data.logs.join("\n")}${data.logsTruncated ? "\n(log limit reached)" : ""}` : "";
                output.textContent = `${data.phase}: ${data.message}${logs}`;
                status.textContent = "Failed. See diagnostics below.";
                finish();
            }
        });

        current.addEventListener("error", (error) => {
            if (worker !== current) {
                return;
            }
            error.preventDefault();
            output.textContent = `${error.message || "Worker failed to load."}\n\nBuild pkg/ and serve the project root over HTTP. Check the browser console and docs/browser.md for module, MIME, and CSP setup.`;
            status.textContent = "Worker failed.";
            finish();
        });

        current.addEventListener("messageerror", () => {
            if (worker !== current) {
                return;
            }
            output.textContent = "Could not receive the worker result.";
            status.textContent = "Worker communication failed.";
            finish();
        });

        setDeadline(15000, "Compiler loading or compilation exceeded 15 seconds; worker terminated. Check the build, HTTP server, and browser console.");
        current.postMessage({
            source: source.value,
            hostPrint: hostPrint.checked,
            showWat: showWat.checked,
            optimization: Number(optimization.value),
        });
    } catch (error) {
        output.textContent = error instanceof Error ? error.message : String(error);
        status.textContent = "Could not start worker.";
        finish();
    }
});

stop.addEventListener("click", () => {
    finish();
    status.textContent = "Stopped; worker terminated.";
    output.textContent = "Worker terminated by request. Any completed compilation remains available for download.";
});

download.addEventListener("click", () => {
    if (downloadUrl === null) {
        return;
    }
    const link = document.createElement("a");
    link.href = downloadUrl;
    link.download = "program.wasm";
    document.body.append(link);
    link.click();
    link.remove();
});

window.addEventListener("pagehide", () => {
    finish();
    clearArtifact();
});

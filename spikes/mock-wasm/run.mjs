// Runs the wasm build in Node: node run.mjs [n]
import { first_events, send_round_trip } from "./pkg/mock_wasm_spike.js";

const n = Number(process.argv[2] ?? 300);
const t0 = performance.now();
const lines = await first_events(n);
const t1 = performance.now();
process.stdout.write(lines + "\n");
process.stderr.write(await send_round_trip() + "\n");
process.stderr.write(`wasm: ${n} events in ${(t1 - t0).toFixed(0)} ms of real time\n`);

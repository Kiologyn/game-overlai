import { spawn, spawnSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const webDir = path.join(root, "web");
const tauriDir = path.join(root, "src-tauri");
const viteUrl = "http://127.0.0.1:5173";

const vite = spawn("npm.cmd", ["run", "dev", "--", "--host", "127.0.0.1", "--strictPort"], {
  cwd: webDir,
  env: process.env,
  stdio: "inherit",
  windowsHide: true,
  shell: true,
});
let app;

function killTree(child) {
  if (!child?.pid) return;
  spawnSync("taskkill", ["/pid", String(child.pid), "/t", "/f"], { stdio: "ignore", windowsHide: true });
}

async function waitForVite() {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (vite.exitCode !== null || vite.signalCode !== null) {
      throw new Error("Vite exited before it became ready.");
    }
    const ready = await new Promise((resolve) => {
      const socket = net.createConnection({ host: "127.0.0.1", port: 5173 });
      socket.once("connect", () => { socket.destroy(); resolve(true); });
      socket.once("error", () => resolve(false));
    });
    if (ready) return;
    await delay(200);
  }
  throw new Error("Vite did not start on 127.0.0.1:5173.");
}

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    killTree(app);
    killTree(vite);
    process.exit(signal === "SIGINT" ? 130 : 143);
  });
}

try {
  await waitForVite();
  console.log(`Vite ready at ${viteUrl}`);
  app = spawn("cargo", ["run"], {
    cwd: tauriDir,
    env: { ...process.env, OVERLAI_UI_DEV: viteUrl },
    stdio: "inherit",
    windowsHide: true,
  });
  const code = await new Promise((resolve, reject) => {
    app.once("error", reject);
    app.once("close", (exitCode) => resolve(exitCode ?? 1));
  });
  process.exitCode = code;
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
} finally {
  killTree(vite);
}

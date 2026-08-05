#!/usr/bin/env node
// Thin wrapper around the real `tauri` CLI binary, invoked by `npm run
// tauri` (see package.json) so every existing `npm run tauri <...>`
// command keeps working unchanged.
//
// The one thing it actually does: for `dev` specifically, Vite's dev
// server is hardcoded to a fixed port (`vite.config.ts`) with
// `strictPort: true`, because `tauri.conf.json`'s `build.devUrl` has to
// name a single fixed URL for the webview to load. That's normally fine,
// but it means a *second* `npm run tauri dev` on the same machine (the
// documented way to test messaging between two accounts locally -- see
// README's "Running two instances locally" section, and
// `apps/desktop/src-tauri/src/accounts.rs`'s `P2P_CHAT_PROFILE`) used to
// just fail outright: the first instance already owns port 1420, so
// Vite's `beforeDevCommand` errors out and Tauri never even opens a
// window for the second one.
//
// This finds a free port starting at 1420, tells Vite to use it via
// `TAURI_DEV_PORT` (read by `vite.config.ts`), and tells the Tauri CLI to
// point the webview at that same port via a `--config` override on
// `build.devUrl` (`tauri dev --help` documents `-c/--config` as merging
// JSON on top of `tauri.conf.json`). Both env var and CLI flag reach the
// same `tauri dev` invocation, which then spawns Vite as its own
// `beforeDevCommand` child, inheriting the env var down the chain.
//
// Port selection goes through a lockfile per candidate port rather than
// just probing-then-releasing a TCP bind: two `npm run tauri dev`s
// launched close together (exactly the two-terminal workflow the README
// describes) can otherwise both probe port 1420 as free before either one
// has actually bound it yet, and both pick the same port. An exclusive
// lockfile create (`wx`) is atomic across processes, so whichever wrapper
// gets there first wins the port outright; a lock left behind by a
// previous instance that crashed without cleaning up is detected (its PID
// is no longer alive) and reclaimed automatically.
import { createServer } from "node:net";
import { spawn } from "node:child_process";
import { closeSync, existsSync, mkdirSync, openSync, readFileSync, unlinkSync, writeSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BASE_PORT = 1420;
const MAX_TRIES = 20;
const LOCK_DIR = join(tmpdir(), "seal-dev-ports");

function isPidAlive(pid) {
  if (!Number.isInteger(pid)) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/** Atomically claims `port`'s lockfile for this process, reclaiming it
 * first if it was left behind by a process that's no longer running.
 * Returns the lock path on success, or `null` if someone else genuinely
 * holds it right now. */
function tryClaimPortLock(port) {
  mkdirSync(LOCK_DIR, { recursive: true });
  const lockPath = join(LOCK_DIR, `${port}.lock`);
  try {
    const fd = openSync(lockPath, "wx");
    writeSync(fd, String(process.pid));
    closeSync(fd);
    return lockPath;
  } catch (err) {
    if (err.code !== "EEXIST") throw err;
    if (existsSync(lockPath)) {
      const heldBy = Number(readFileSync(lockPath, "utf8").trim());
      if (!isPidAlive(heldBy)) {
        try {
          unlinkSync(lockPath);
        } catch {
          // Someone else already reclaimed it between our check and now --
          // fine, just report this port as taken for this attempt.
          return null;
        }
        return tryClaimPortLock(port);
      }
    }
    return null;
  }
}

function isPortFree(port) {
  return new Promise((resolve) => {
    const srv = createServer();
    srv.once("error", () => resolve(false));
    srv.once("listening", () => srv.close(() => resolve(true)));
    srv.listen(port, "127.0.0.1");
  });
}

async function claimFreePort() {
  for (let i = 0; i < MAX_TRIES; i++) {
    const port = BASE_PORT + i;
    const lockPath = tryClaimPortLock(port);
    if (!lockPath) continue; // another wrapper instance already owns this one
    if (await isPortFree(port)) return { port, lockPath };
    // Locked by us, but something outside this wrapper's coordination is
    // already listening here -- release and keep looking.
    try {
      unlinkSync(lockPath);
    } catch {
      // already gone, nothing to do
    }
  }
  throw new Error(
    `no free port found in ${BASE_PORT}-${BASE_PORT + MAX_TRIES - 1} -- ` +
      "that's a lot of Seal dev instances already running",
  );
}

const args = process.argv.slice(2);
const isDev = args[0] === "dev";

let spawnArgs = args;
const extraEnv = {};
let releaseLock = () => {};

if (isDev) {
  const { port, lockPath } = await claimFreePort();
  if (port !== BASE_PORT) {
    console.log(
      `[tauri] port ${BASE_PORT} is already in use (another Seal dev instance?) -- using ${port} for this one`,
    );
  }
  releaseLock = () => {
    try {
      unlinkSync(lockPath);
    } catch {
      // already gone, nothing to do
    }
  };
  extraEnv.TAURI_DEV_PORT = String(port);
  spawnArgs = [
    args[0],
    "--config",
    JSON.stringify({ build: { devUrl: `http://localhost:${port}` } }),
    ...args.slice(1),
  ];
}

const child = spawn("tauri", spawnArgs, {
  stdio: "inherit",
  env: { ...process.env, ...extraEnv },
  // npm installs a `tauri.cmd` shim on Windows, not a bare `tauri.exe`;
  // spawning through a shell is what lets Node's PATH lookup find it.
  shell: process.platform === "win32",
});

child.on("exit", (code, signal) => {
  releaseLock();
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 0);
  }
});

child.on("error", (err) => {
  releaseLock();
  console.error(err);
  process.exit(1);
});

for (const sig of ["SIGINT", "SIGTERM"]) {
  process.on(sig, () => {
    releaseLock();
    if (!child.killed) child.kill(sig);
  });
}
